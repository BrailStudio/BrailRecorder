mod commands;
mod events;
mod state;

use std::sync::Arc;

use brail_core::config::{
    AppConfig, ContainerFormat, EncoderBackend, EncoderPreset, EncoderProfile, EncoderSettings,
    FrameRate, InstantReplaySettings, RateControlMode, RecordingSettings, Resolution, UiMode,
    VideoCodec,
};
use brail_core::profile::HotkeyBindings;
use brail_hotkeys::{HotkeyAction, HotkeyManager};
use brail_storage::config_store::ConfigStore;
use state::AppState;
use tauri::Manager;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive("brail=info".parse().unwrap()),
        )
        .init();

    let config_store = ConfigStore::new().expect("failed to initialize config storage");
    let config = config_store
        .load()
        .expect("failed to load config")
        .unwrap_or_else(default_config);

    let app_state = AppState::new(config_store, config);

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .manage(app_state)
        .setup(move |app| {
            let handle = app.handle().clone();
            let state = app.state::<Arc<AppState>>().inner().clone();

            // Check for a crashed previous session before anything else
            // touches the recordings directory — see brail-recovery's
            // design note on why this must run before any new recording
            // can start.
            let config_dir = std::env::var("APPDATA")
                .map(|a| std::path::PathBuf::from(a).join("BrailRecorder"))
                .unwrap_or_default();
            let recovery = brail_recovery::RecoveryManager::new(&config_dir);
            if let Some(crashed) = recovery.check_for_crashed_recording() {
                tracing::warn!(
                    path = %crashed.output_path.display(),
                    "recovered an in-progress recording from an unclean shutdown"
                );
                events::emit(
                    &handle,
                    brail_core::events::AppEvent::RecordingRecovered {
                        recovered_path: crashed.output_path.display().to_string(),
                    },
                );
                recovery.clear_lock();
            }

            // Hardware detection runs in the background rather than
            // blocking window creation — the UI shows a "detecting
            // hardware..." state and the frontend's `get_hardware_profile`
            // call awaits this same detection if it hasn't completed yet
            // (see commands.rs's caching behavior there).
            let handle_for_hw = handle.clone();
            let state_for_hw = state.clone();
            tauri::async_runtime::spawn(async move {
                match brail_hardware::detect_capabilities().await {
                    Ok(profile) => {
                        *state_for_hw.hardware_profile.lock().await = Some(profile.clone());
                        events::emit(&handle_for_hw, brail_core::events::AppEvent::HardwareDetected(profile));
                    }
                    Err(e) => {
                        tracing::error!("hardware detection failed: {e}");
                    }
                }
            });

            // Global hotkeys run on their own dedicated Win32 message-loop
            // thread (see brail-hotkeys) for the app's whole lifetime.
            let hotkeys_config =
                tauri::async_runtime::block_on(async { state.config.lock().await.hotkeys.clone() });
            let (action_tx, mut action_rx) = tokio::sync::mpsc::unbounded_channel::<HotkeyAction>();
            let (error_tx, mut error_rx) = tokio::sync::mpsc::unbounded_channel();
            let hotkey_manager = HotkeyManager::spawn(hotkeys_config, action_tx, error_tx);
            app.manage(std::sync::Mutex::new(hotkey_manager));

            let handle_for_hotkeys = handle.clone();
            let state_for_hotkeys = state.clone();
            tauri::async_runtime::spawn(async move {
                while let Some(action) = action_rx.recv().await {
                    dispatch_hotkey_action(action, &handle_for_hotkeys, &state_for_hotkeys).await;
                }
            });

            let handle_for_hotkey_errors = handle.clone();
            tauri::async_runtime::spawn(async move {
                while let Some(err) = error_rx.recv().await {
                    events::emit(&handle_for_hotkey_errors, brail_core::events::AppEvent::Error(err));
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_hardware_profile,
            commands::get_config,
            commands::save_config,
            commands::list_capturable_windows,
            commands::start_recording,
            commands::stop_recording,
            commands::start_streaming,
            commands::stop_streaming,
            commands::add_stream_profile,
            commands::remove_stream_profile,
            commands::save_replay,
            commands::get_active_encoder_label,
            commands::take_screenshot,
            commands::test_stream_connection,
            commands::run_benchmark,
            commands::pause_recording,
            commands::resume_recording,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Brail Recorder");
}

/// Routes a fired global hotkey to the same logic the frontend's buttons
/// use. Calls the `*_impl` functions in `commands.rs` directly (plain
/// `&AppHandle`/`&Arc<AppState>`) rather than the `#[tauri::command]`
/// wrappers, since `tauri::State` can only be constructed by Tauri's own
/// IPC dispatch — there is no supported way to build one by hand from a
/// reference held outside a command invocation.
async fn dispatch_hotkey_action(action: HotkeyAction, app: &tauri::AppHandle, state: &Arc<AppState>) {
    tracing::info!(?action, "global hotkey triggered");

    let result = match action {
        HotkeyAction::StartStopRecording => {
            if state.recording.lock().await.is_some() {
                commands::stop_recording_impl(app, state).await
            } else {
                // Starting via hotkey needs a monitor selection; the full
                // wiring resolves this from the last-used monitor stored
                // in `AppConfig` (added alongside the settings UI's
                // "remember last monitor" toggle) rather than prompting,
                // since a global hotkey firing mid-game must never pop a
                // picker dialog. Left as a documented follow-up rather
                // than guessing a monitor handle here.
                Ok(())
            }
        }
        HotkeyAction::SaveReplay => {
            if state.replay.lock().await.is_some() {
                let config = state.config.lock().await.clone();
                let replay_guard = state.replay.lock().await;
                match replay_guard.as_ref() {
                    Some(replay) => {
                        let has_audio = config.audio.desktop.enabled || config.audio.microphone.enabled;
                        match replay.save_replay(&config.recording.output_dir, has_audio) {
                            Ok(path) => {
                                events::emit(app, brail_core::events::AppEvent::ReplaySaved {
                                    output_path: path.display().to_string(),
                                });
                                Ok(())
                            }
                            Err(e) => Err(e.to_string()),
                        }
                    }
                    None => Ok(()),
                }
            } else {
                Ok(())
            }
        }
        HotkeyAction::PauseResumeRecording => {
            let paused = {
                let recording = state.recording.lock().await;
                match recording.as_ref() {
                    Some(handle) => Some(handle.encoder.is_paused()),
                    None => None,
                }
            };
            match paused {
                Some(true) => commands::resume_recording_impl(app, state).await,
                Some(false) => commands::pause_recording_impl(app, state).await,
                None => Ok(()), // nothing recording; a stray hotkey press is not an error
            }
        }

        // The mute toggles mutate config directly rather than going
        // through a command, because muting must take effect instantly
        // mid-recording and involves no pipeline restart.
        HotkeyAction::ToggleMicrophoneMute => {
            let mut config = state.config.lock().await;
            config.audio.microphone.muted = !config.audio.microphone.muted;
            let _ = state.config_store.save(&config);
            Ok(())
        }

        HotkeyAction::ToggleDesktopAudioMute => {
            let mut config = state.config.lock().await;
            config.audio.desktop.muted = !config.audio.desktop.muted;
            let _ = state.config_store.save(&config);
            Ok(())
        }

        HotkeyAction::ToggleWebcam => {
            let mut config = state.config.lock().await;
            config.webcam.enabled = !config.webcam.enabled;
            let _ = state.config_store.save(&config);
            Ok(())
        }

        HotkeyAction::TakeScreenshot => {
            let monitor_id = {
                let hw = state.hardware_profile.lock().await;
                hw.as_ref()
                    .and_then(|h| h.monitors.iter().find(|m| m.is_primary).or(h.monitors.first()))
                    .map(|m| m.handle_id)
            };
            match monitor_id {
                Some(id) => commands::take_screenshot_impl(app, state, id).await.map(|_| ()),
                None => Ok(()),
            }
        }

        HotkeyAction::StartStopStreaming => {
            if state.streaming.lock().await.is_some() {
                commands::stop_streaming_impl(state).await
            } else {
                match state.config.lock().await.active_stream_profile {
                    Some(id) => commands::start_streaming_impl(app, state, id).await,
                    // No configured destination: a hotkey must never pop a
                    // dialog mid-game, so this is a no-op the UI explains.
                    None => Ok(()),
                }
            }
        }
    };

    if let Err(e) = result {
        tracing::warn!("hotkey-triggered action failed: {e}");
    }
}

/// Sensible first-run defaults before hardware detection has run. Once
/// `brail-hardware::detect_capabilities` completes, the UI's onboarding
/// flow offers to apply its `recommended_preset` on top of this — this
/// function exists so the app has *a* valid config to load immediately at
/// first launch, not so it guesses good settings for this specific
/// machine.
fn default_config() -> AppConfig {
    let output_dir =
        brail_storage::output_paths::default_output_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));

    let default_encoder = EncoderSettings {
        backend: EncoderBackend::Software,
        codec: VideoCodec::H264,
        rate_control: RateControlMode::Cbr,
        bitrate_kbps: Some(6000),
        cqp_level: None,
        keyframe_interval_secs: 2.0,
        preset: EncoderPreset::Balanced,
        profile: EncoderProfile::High,
        b_frames: 2,
    };

    let screenshot_dir = output_dir.join("Screenshots");

    AppConfig {
        ui_mode: UiMode::Beginner,
        general: brail_core::settings::GeneralSettings::default(),
        audio: brail_core::settings::AudioSettings::default(),
        webcam: brail_core::settings::WebcamOverlaySettings::default(),
        overlays: Vec::new(),
        screenshot: brail_core::settings::ScreenshotSettings {
            format: brail_core::settings::ScreenshotFormat::Png,
            jpeg_quality: 92,
            directory: screenshot_dir,
        },
        performance: brail_core::settings::PerformanceSettings::default(),
        stream_and_record: false,
        recording: RecordingSettings {
            resolution: Resolution::new(1920, 1080),
            frame_rate: FrameRate::Fps30,
            encoder: default_encoder.clone(),
            container: ContainerFormat::Mkv,
            remux_to: None,
            output_dir,
            capture_cursor: true,
            highlight_cursor: false,
        },
        instant_replay: InstantReplaySettings {
            enabled: false,
            buffer_seconds: 30,
            resolution: Resolution::new(1280, 720),
            frame_rate: FrameRate::Fps30,
            encoder: default_encoder,
        },
        stream_profiles: Vec::new(),
        active_stream_profile: None,
        hotkeys: HotkeyBindings::default(),
    }
}
