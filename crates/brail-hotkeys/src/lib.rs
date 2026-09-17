//! brail-hotkeys: global hotkey registration via Win32 `RegisterHotKey`.
//!
//! `RegisterHotKey`/`WM_HOTKEY` only work on a thread that runs a real
//! Win32 message loop (`GetMessage`/`DispatchMessage`), which the Tauri
//! webview's own message loop cannot be repurposed for without risking UI
//! jank. This crate spawns one dedicated thread whose entire job is: own a
//! message-only window, register every configured hotkey against it, and
//! forward `WM_HOTKEY` notifications out through a channel — never
//! anything UI-related.

use brail_core::error::{BrailError, BrailResult};
use brail_core::profile::{HotkeyBindings, HotkeyCombo};
use tokio::sync::mpsc;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT,
    MOD_SHIFT, MOD_WIN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, PostThreadMessageW, TranslateMessage, MSG, WM_HOTKEY, WM_QUIT,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotkeyAction {
    StartStopRecording,
    StartStopStreaming,
    SaveReplay,
    ToggleMicrophoneMute,
    ToggleDesktopAudioMute,
    ToggleWebcam,
    PauseResumeRecording,
    TakeScreenshot,
}

pub struct HotkeyManager {
    thread_id: u32,
    join_handle: Option<std::thread::JoinHandle<()>>,
}

impl HotkeyManager {
    /// Spawns the message-loop thread and registers every binding that's
    /// `Some(..)` in `bindings`. Returns immediately; hotkey presses arrive
    /// asynchronously on `action_tx`. A binding that fails to register
    /// (most commonly because another running application already claimed
    /// that combination) reports `BrailError::HotkeyRegistrationFailed` via
    /// `action_tx`'s companion error channel rather than aborting startup —
    /// one broken hotkey should never block the rest of the app.
    pub fn spawn(
        bindings: HotkeyBindings,
        action_tx: mpsc::UnboundedSender<HotkeyAction>,
        error_tx: mpsc::UnboundedSender<BrailError>,
    ) -> Self {
        let (thread_id_tx, thread_id_rx) = std::sync::mpsc::channel();

        let join_handle = std::thread::Builder::new()
            .name("brail-hotkeys".into())
            .spawn(move || {
                let thread_id = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
                let _ = thread_id_tx.send(thread_id);

                let registrations: Vec<(i32, HotkeyAction)> = [
                    (bindings.start_stop_recording, HotkeyAction::StartStopRecording),
                    (bindings.start_stop_streaming, HotkeyAction::StartStopStreaming),
                    (bindings.save_replay, HotkeyAction::SaveReplay),
                    (bindings.toggle_microphone_mute, HotkeyAction::ToggleMicrophoneMute),
                    (bindings.toggle_desktop_audio_mute, HotkeyAction::ToggleDesktopAudioMute),
                    (bindings.toggle_webcam, HotkeyAction::ToggleWebcam),
                    (bindings.pause_resume_recording, HotkeyAction::PauseResumeRecording),
                    (bindings.take_screenshot, HotkeyAction::TakeScreenshot),
                ]
                .into_iter()
                .enumerate()
                .filter_map(|(i, (combo, action))| {
                    let combo = combo?;
                    let id = 0xB000 + i as i32; // arbitrary app-private hotkey id range
                    match register(id, &combo) {
                        Ok(()) => Some((id, action)),
                        Err(e) => {
                            let _ = error_tx.send(BrailError::HotkeyRegistrationFailed(format!(
                                "{action:?}: {e}"
                            )));
                            None
                        }
                    }
                })
                .collect();

                run_message_loop(&registrations, &action_tx);

                for (id, _) in &registrations {
                    unsafe {
                        let _ = UnregisterHotKey(HWND::default(), *id);
                    }
                }
            })
            .expect("failed to spawn brail-hotkeys thread");

        let thread_id = thread_id_rx.recv().unwrap_or(0);

        Self {
            thread_id,
            join_handle: Some(join_handle),
        }
    }

    pub fn shutdown(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn register(id: i32, combo: &HotkeyCombo) -> anyhow::Result<()> {
    let mut modifiers = MOD_NOREPEAT;
    if combo.ctrl {
        modifiers |= MOD_CONTROL;
    }
    if combo.shift {
        modifiers |= MOD_SHIFT;
    }
    if combo.alt {
        modifiers |= MOD_ALT;
    }
    if combo.win {
        modifiers |= MOD_WIN;
    }

    let vk = virtual_key_from_name(&combo.key)
        .ok_or_else(|| anyhow::anyhow!("unknown key name '{}'", combo.key))?;

    unsafe {
        RegisterHotKey(HWND::default(), id, modifiers, vk as u32)?;
    }
    Ok(())
}

fn run_message_loop(registrations: &[(i32, HotkeyAction)], action_tx: &mpsc::UnboundedSender<HotkeyAction>) {
    let mut msg = MSG::default();
    unsafe {
        loop {
            let result = GetMessageW(&mut msg, HWND::default(), 0, 0);
            if result.0 <= 0 {
                break; // WM_QUIT or an error; either way, exit the loop
            }

            if msg.message == WM_HOTKEY {
                let id = msg.wParam.0 as i32;
                if let Some((_, action)) = registrations.iter().find(|(rid, _)| *rid == id) {
                    let _ = action_tx.send(*action);
                }
            }

            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// Maps the small set of key names the UI's hotkey picker can produce
/// (function keys, letters, digits) to Win32 virtual-key codes. Extending
/// the UI picker to more keys means extending this match arm — kept as an
/// explicit table rather than a generic parser since the UI only ever
/// sends names this table defines, by construction.
fn virtual_key_from_name(name: &str) -> Option<u16> {
    use windows::Win32::UI::Input::KeyboardAndMouse::*;

    match name {
        "F1" => Some(VK_F1.0),
        "F2" => Some(VK_F2.0),
        "F3" => Some(VK_F3.0),
        "F4" => Some(VK_F4.0),
        "F5" => Some(VK_F5.0),
        "F6" => Some(VK_F6.0),
        "F7" => Some(VK_F7.0),
        "F8" => Some(VK_F8.0),
        "F9" => Some(VK_F9.0),
        "F10" => Some(VK_F10.0),
        "F11" => Some(VK_F11.0),
        "F12" => Some(VK_F12.0),
        single_char if single_char.len() == 1 => {
            let c = single_char.chars().next()?.to_ascii_uppercase();
            if c.is_ascii_alphanumeric() {
                Some(c as u16)
            } else {
                None
            }
        }
        _ => None,
    }
}
