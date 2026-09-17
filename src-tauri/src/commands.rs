use std::sync::Arc;

use brail_core::config::{AppConfig, StreamProfile};
use brail_core::error::BrailError;
use brail_core::profile::CapabilityProfile;
use brail_capture::{sources::CaptureSource, CaptureEngine};
use brail_encoder::EncodeController;
use brail_security::CredentialVault;
use tauri::{AppHandle, State};
use uuid::Uuid;

use crate::events;
use crate::state::{AppState, RecordingHandle, StreamingHandle};

#[tauri::command]
pub async fn get_hardware_profile(
    state: State<'_, Arc<AppState>>,
) -> Result<CapabilityProfile, String> {
    let mut cached = state.hardware_profile.lock().await;
    if let Some(profile) = cached.as_ref() {
        return Ok(profile.clone());
    }

    let profile = brail_hardware::detect_capabilities()
        .await
        .map_err(|e| e.to_string())?;
    *cached = Some(profile.clone());
    Ok(profile)
}

#[tauri::command]
pub async fn get_config(state: State<'_, Arc<AppState>>) -> Result<AppConfig, String> {
    Ok(state.config.lock().await.clone())
}

#[tauri::command]
pub async fn save_config(state: State<'_, Arc<AppState>>, config: AppConfig) -> Result<(), String> {
    state.config_store.save(&config).map_err(|e| e.to_string())?;
    *state.config.lock().await = config;
    Ok(())
}

#[derive(serde::Serialize)]
pub struct CaptureSourceDto {
    pub kind: &'static str,
    pub id: isize,
    pub label: String,
}

impl From<CaptureSource> for CaptureSourceDto {
    fn from(s: CaptureSource) -> Self {
        match s {
            CaptureSource::Monitor { handle_id, friendly_name } => Self {
                kind: "monitor",
                id: handle_id,
                label: friendly_name,
            },
            CaptureSource::Window { hwnd_id, title } => Self {
                kind: "window",
                id: hwnd_id,
                label: title,
            },
        }
    }
}

#[tauri::command]
pub async fn list_capturable_windows() -> Result<Vec<CaptureSourceDto>, String> {
    let sources = brail_capture::sources::enumerate_capturable_windows().map_err(|e| e.to_string())?;
    Ok(sources.into_iter().map(CaptureSourceDto::from).collect())
}

#[tauri::command]
pub async fn start_recording(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    monitor_handle_id: isize,
) -> Result<(), String> {
    start_recording_impl(&app, &state, monitor_handle_id).await
}

/// Plain-async core of `start_recording`, callable both from the Tauri IPC
/// wrapper above (which only has a `State<'_, ..>` extractor available)
/// and directly from `main.rs`'s hotkey dispatcher (which only has a plain
/// `&Arc<AppState>`, since `State` can only be constructed by Tauri's own
/// command-invocation machinery, not built by hand from a reference).
pub async fn start_recording_impl(
    app: &AppHandle,
    state: &Arc<AppState>,
    monitor_handle_id: isize,
) -> Result<(), String> {
    let config = state.config.lock().await.clone();

    // Refuse to start without at least a minute of disk headroom at the
    // configured bitrate — see brail-storage::disk_space's design note on
    // why this check happens before, not after, starting.
    let bitrate = config.recording.encoder.bitrate_kbps.unwrap_or(6000);
    let has_headroom = brail_storage::disk_space::has_minimum_headroom(&config.recording.output_dir, bitrate)
        .map_err(|e| e.to_string())?;
    if !has_headroom {
        return Err("Not enough free disk space to start recording.".into());
    }

    let handle = {
        let mut capture_guard = state.capture.lock().await;
        let mut engine = CaptureEngine::new().map_err(|e| e.to_string())?;

        let hmonitor = windows::Win32::Graphics::Gdi::HMONITOR(monitor_handle_id as *mut _);
        engine
            .start_monitor_capture(hmonitor, config.recording.capture_cursor)
            .map_err(|e: BrailError| e.to_string())?;

        let handle = engine.handle();
        *capture_guard = Some(engine);
        handle
    };

    let output_path = brail_storage::output_paths::generate_output_path(
        &config.recording.output_dir,
        "Recording",
        container_extension(config.recording.container),
    );

    let (encoder, mut packet_rx) = EncodeController::spawn(
        handle.subscribe(),
        config.recording.encoder.clone(),
        config.recording.resolution.width,
        config.recording.resolution.height,
        config.recording.frame_rate.as_u32(),
    )
    .map_err(|e: BrailError| e.to_string())?;

    let recovery = brail_recovery::RecoveryManager::new(&config_dir());
    recovery.mark_recording_started(&output_path).map_err(|e| e.to_string())?;

    let mux_path = output_path.clone();
    let container = config.recording.container;
    let codec = config.recording.encoder.codec;
    let (w, h) = (config.recording.resolution.width, config.recording.resolution.height);
    let fps = config.recording.frame_rate.as_u32();
    let has_audio = config.audio.desktop.enabled || config.audio.microphone.enabled;

    // The writer task owns the muxer exclusively — packets are handed over
    // via the encoder's mpsc channel — so file I/O never blocks the
    // encode thread's real-time loop.
    let app_for_writer = app.clone();
    let writer_task = tokio::spawn(async move {
        let mut muxer = match brail_encoder::Muxer::create(&mux_path, container, codec, w, h, fps, has_audio) {
            Ok(m) => m,
            Err(e) => {
                events::emit(&app_for_writer, brail_core::events::AppEvent::Error(e));
                return;
            }
        };

        while let Some(packet) = packet_rx.recv().await {
            if let Err(e) = muxer.write_packet(&packet) {
                events::emit(&app_for_writer, brail_core::events::AppEvent::Error(e));
                break;
            }
        }

        if let Err(e) = muxer.finalize() {
            events::emit(&app_for_writer, brail_core::events::AppEvent::Error(e));
        }
    });

    *state.recording.lock().await = Some(RecordingHandle {
        output_path: output_path.clone(),
        encoder,
        writer_task,
        started_at: std::time::Instant::now(),
    });

    events::emit(app, brail_core::events::AppEvent::RecordingStarted {
        output_path: output_path.display().to_string(),
    });

    Ok(())
}

#[tauri::command]
pub async fn stop_recording(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    stop_recording_impl(&app, &state).await
}

pub async fn stop_recording_impl(app: &AppHandle, state: &Arc<AppState>) -> Result<(), String> {
    let handle = state.recording.lock().await.take();
    let Some(handle) = handle else {
        return Err("No recording is currently active.".into());
    };

    // Shutting down the encoder lets its blocking thread flush remaining
    // packets through the still-open channel; only after that thread exits
    // (dropping the sender) does the writer task's `recv()` loop end and
    // the muxer finalize — awaiting `writer_task` below is what actually
    // waits for that whole chain, not just the shutdown signal itself.
    handle.encoder.shutdown();
    let _ = handle.writer_task.await;

    let recovery = brail_recovery::RecoveryManager::new(&config_dir());
    recovery.mark_recording_finished().map_err(|e| e.to_string())?;

    let mut capture_guard = state.capture.lock().await;
    if let Some(mut engine) = capture_guard.take() {
        engine.stop();
    }

    let size = std::fs::metadata(&handle.output_path).map(|m| m.len()).unwrap_or(0);
    events::emit(app, brail_core::events::AppEvent::RecordingStopped {
        output_path: handle.output_path.display().to_string(),
        final_size_bytes: size,
    });

    Ok(())
}

#[tauri::command]
pub async fn start_streaming(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    profile_id: Uuid,
) -> Result<(), String> {
    start_streaming_impl(&app, &state, profile_id).await
}

pub async fn start_streaming_impl(
    app: &AppHandle,
    state: &Arc<AppState>,
    profile_id: Uuid,
) -> Result<(), String> {
    let mut config = state.config.lock().await.clone();
    let profile = config
        .stream_profiles
        .iter_mut()
        .find(|p| p.id == profile_id)
        .ok_or("Stream profile not found")?;

    let stream_key = CredentialVault::load_stream_key(profile_id)
        .map_err(|e| e.to_string())?
        .ok_or("No stream key saved for this profile. Add one in Settings.")?;
    profile.stream_key = Some(stream_key);
    let profile = profile.clone();

    let handle = {
        let capture_guard = state.capture.lock().await;
        let Some(engine) = capture_guard.as_ref() else {
            return Err("Start capture (e.g. by recording) before streaming.".into());
        };
        engine.handle()
    };

    let (encoder, packet_source_rx) = EncodeController::spawn(
        handle.subscribe(),
        profile.encoder.clone(),
        profile.resolution.width,
        profile.resolution.height,
        profile.frame_rate.as_u32(),
    )
    .map_err(|e: BrailError| e.to_string())?;

    // StreamSession::run takes ownership of the encoder's packet receiver
    // directly and consumes it until the encoder's thread exits (shutdown
    // -> flush -> its packet_tx sender drops -> this receiver closes ->
    // the session's `recv()` loop ends cleanly) — no extra channel needed
    // between the encoder and the session for the single-consumer
    // streaming case.
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
    let app_for_events = app.clone();
    tokio::spawn(async move {
        while let Some(event) = event_rx.recv().await {
            events::emit(&app_for_events, event);
        }
    });

    let session = brail_streaming::StreamSession::new(profile, event_tx);
    let session_task = tokio::spawn(session.run(packet_source_rx));

    *state.streaming.lock().await = Some(StreamingHandle {
        session_task,
        encoder,
    });

    Ok(())
}

#[tauri::command]
pub async fn stop_streaming(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    stop_streaming_impl(&state).await
}

pub async fn stop_streaming_impl(state: &Arc<AppState>) -> Result<(), String> {
    let handle = state.streaming.lock().await.take();
    if let Some(handle) = handle {
        handle.encoder.shutdown();
        let _ = handle.session_task.await;
    }
    Ok(())
}

#[tauri::command]
pub async fn add_stream_profile(
    state: State<'_, Arc<AppState>>,
    profile: StreamProfile,
    stream_key: String,
) -> Result<(), String> {
    CredentialVault::store_stream_key(profile.id, &stream_key).map_err(|e| e.to_string())?;

    let mut config = state.config.lock().await;
    config.stream_profiles.push(profile);
    state.config_store.save(&config).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn remove_stream_profile(state: State<'_, Arc<AppState>>, profile_id: Uuid) -> Result<(), String> {
    CredentialVault::delete_stream_key(profile_id).map_err(|e| e.to_string())?;

    let mut config = state.config.lock().await;
    config.stream_profiles.retain(|p| p.id != profile_id);
    state.config_store.save(&config).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub async fn save_replay(state: State<'_, Arc<AppState>>) -> Result<String, String> {
    let config = state.config.lock().await;
    let replay_guard = state.replay.lock().await;
    let Some(replay) = replay_guard.as_ref() else {
        return Err("Instant replay is not enabled.".into());
    };

    let has_audio = config.audio.desktop.enabled || config.audio.microphone.enabled;
    let path = replay
        .save_replay(&config.recording.output_dir, has_audio)
        .map_err(|e: BrailError| e.to_string())?;

    Ok(path.display().to_string())
}

fn container_extension(container: brail_core::config::ContainerFormat) -> &'static str {
    match container {
        brail_core::config::ContainerFormat::Mkv => "mkv",
        brail_core::config::ContainerFormat::Mp4 => "mp4",
        brail_core::config::ContainerFormat::WebM => "webm",
    }
}

fn config_dir() -> std::path::PathBuf {
    let appdata = std::env::var("APPDATA").unwrap_or_default();
    std::path::Path::new(&appdata).join("BrailRecorder")
}

/* ------------------------------------------------------------------
   Commands added for screenshots, connection testing, the adaptive
   engine, and benchmark mode. Each one is wired to a real
   implementation — per §67, no UI control in this app resolves to a
   fake success.
   ------------------------------------------------------------------ */

/// Human-readable label for the encoder that would actually be used with
/// the current settings, e.g. "NVIDIA NVENC H.264" (§6: show the active
/// encoder clearly). Reports the software fallback honestly rather than
/// implying hardware encoding when none was verified.
#[tauri::command]
pub async fn get_active_encoder_label(state: State<'_, Arc<AppState>>) -> Result<String, String> {
    let config = state.config.lock().await;
    let hardware = state.hardware_profile.lock().await;

    let Some(caps) = hardware.as_ref() else {
        return Ok("Detecting hardware…".into());
    };

    let codec = config.recording.encoder.codec;
    let resolution = config.recording.resolution;

    match brail_core::presets::best_encoder_for(caps, codec, resolution) {
        Some(cap) => Ok(cap.backend.display_name(cap.codec)),
        None => Ok(brail_core::config::EncoderBackend::Software.display_name(codec)),
    }
}

/// Captures a single frame of the given monitor and writes it to the
/// configured screenshot folder (§22).
#[tauri::command]
pub async fn take_screenshot(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    monitor_handle_id: isize,
) -> Result<String, String> {
    take_screenshot_impl(&app, &state, monitor_handle_id).await
}

pub async fn take_screenshot_impl(
    app: &AppHandle,
    state: &Arc<AppState>,
    monitor_handle_id: isize,
) -> Result<String, String> {
    let config = state.config.lock().await.clone();
    std::fs::create_dir_all(&config.screenshot.directory)
        .map_err(|e| format!("Could not create the screenshot folder: {e}"))?;

    let extension = match config.screenshot.format {
        brail_core::settings::ScreenshotFormat::Png => "png",
        brail_core::settings::ScreenshotFormat::Jpeg => "jpg",
    };
    let path = brail_storage::output_paths::generate_output_path(
        &config.screenshot.directory,
        "Screenshot",
        extension,
    );

    // Screenshot capture is blocking GPU work (a staging-texture readback
    // plus image encoding), so it runs off the async runtime rather than
    // stalling a worker thread that the recording pipeline also uses.
    let screenshot_settings = config.screenshot.clone();
    let target = path.clone();
    let written = tokio::task::spawn_blocking(move || {
        brail_capture::screenshot::capture_monitor_to_file(
            monitor_handle_id,
            &screenshot_settings,
            &target,
        )
    })
    .await
    .map_err(|e| format!("Screenshot task failed: {e}"))?
    .map_err(|e: BrailError| e.to_string())?;

    events::emit(
        app,
        brail_core::events::AppEvent::ScreenshotSaved {
            output_path: written.display().to_string(),
        },
    );

    Ok(written.display().to_string())
}

/// Runs a genuine connect → handshake → publish → disconnect cycle against
/// the configured ingest endpoint (§60). Nothing is published, so this
/// never creates a stray empty broadcast on the user's channel.
#[tauri::command]
pub async fn test_stream_connection(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    profile_id: Uuid,
) -> Result<brail_streaming::TestResult, String> {
    let mut profile = {
        let config = state.config.lock().await;
        config
            .stream_profiles
            .iter()
            .find(|p| p.id == profile_id)
            .cloned()
            .ok_or("Stream profile not found")?
    };

    // The key is pulled from the vault only for the duration of this call
    // and is never logged or echoed back to the frontend.
    profile.stream_key = CredentialVault::load_stream_key(profile_id).map_err(|e| e.to_string())?;

    let result = brail_streaming::test_connection(&profile).await;

    events::emit(
        &app,
        brail_core::events::AppEvent::StreamTestResult {
            success: result.success,
            message: result.message.clone(),
            round_trip_ms: result.round_trip_ms,
        },
    );

    Ok(result)
}

/// Runs benchmark mode: samples real resource usage during a short
/// recording and returns the diagnostic report (§64).
#[tauri::command]
pub async fn run_benchmark(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
) -> Result<brail_performance::BenchmarkReport, String> {
    let logical_cores = {
        let hw = state.hardware_profile.lock().await;
        hw.as_ref().map(|h| h.cpu_logical_cores).unwrap_or(1)
    };

    let mut monitor = brail_performance::ResourceMonitor::new(logical_cores);
    // Baseline before any capture work starts — this is what the spec's
    // idle-RAM target is measured against.
    let idle = monitor.sample().process_ram_mb;

    let target_fps = state.config.lock().await.recording.frame_rate.as_u32();
    let mut run = brail_performance::BenchmarkRun::start(
        idle,
        target_fps,
        std::time::Duration::from_secs(30),
    );

    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(1));
    while !run.is_complete() {
        ticker.tick().await;

        let resources = monitor.sample();
        let (capture_fps, dropped) = {
            let recording = state.recording.lock().await;
            match recording.as_ref() {
                Some(handle) => {
                    let stats = handle.encoder.stats();
                    (
                        stats.encode_fps,
                        stats.frames_dropped_encoder_backpressure,
                    )
                }
                None => (0.0, 0),
            }
        };

        run.record(resources, capture_fps, 0.0, dropped, None);
    }

    let report = run.finish();
    events::emit(
        &app,
        brail_core::events::AppEvent::BenchmarkComplete {
            summary: report.summary.clone(),
        },
    );

    Ok(report)
}

/// Pauses an in-progress recording without finalizing the file.
#[tauri::command]
pub async fn pause_recording(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    pause_recording_impl(&app, &state).await
}

pub async fn pause_recording_impl(app: &AppHandle, state: &Arc<AppState>) -> Result<(), String> {
    let recording = state.recording.lock().await;
    if recording.is_none() {
        return Err("No recording is currently active.".into());
    }
    // Pausing stops frames being submitted to the encoder while leaving
    // the muxer and output file open, so resuming continues the same file
    // rather than starting a second one.
    recording.as_ref().unwrap().encoder.set_paused(true);
    events::emit(app, brail_core::events::AppEvent::RecordingPaused);
    Ok(())
}

#[tauri::command]
pub async fn resume_recording(app: AppHandle, state: State<'_, Arc<AppState>>) -> Result<(), String> {
    resume_recording_impl(&app, &state).await
}

pub async fn resume_recording_impl(app: &AppHandle, state: &Arc<AppState>) -> Result<(), String> {
    let recording = state.recording.lock().await;
    if recording.is_none() {
        return Err("No recording is currently active.".into());
    }
    recording.as_ref().unwrap().encoder.set_paused(false);
    events::emit(app, brail_core::events::AppEvent::RecordingResumed);
    Ok(())
}
