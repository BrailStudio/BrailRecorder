use serde::{Deserialize, Serialize};

use crate::config::{EncoderBackend, Resolution, VideoCodec};

/// Snapshot of what this machine can actually do, produced once at startup
/// by brail-hardware and re-used everywhere else (UI preset filtering,
/// smart defaults, streaming profile validation) so no other crate needs to
/// re-probe DXGI/NVENC/AMF/QSV itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityProfile {
    pub cpu_name: String,
    pub cpu_physical_cores: u32,
    pub cpu_logical_cores: u32,
    pub total_ram_mb: u64,
    pub gpus: Vec<GpuInfo>,
    pub windows_build: u32,
    pub windows_version_name: String,
    pub monitors: Vec<MonitorInfo>,
    pub audio_input_devices: Vec<String>,
    pub audio_output_devices: Vec<String>,
    pub cameras: Vec<String>,
    pub supported_encoders: Vec<EncoderCapability>,
    /// Best-effort recommendation computed from the above (see
    /// brail-hardware::recommend), shown to the user but always overridable.
    pub recommended_preset: RecommendedPreset,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuInfo {
    pub name: String,
    pub vendor: GpuVendor,
    pub dedicated_vram_mb: u64,
    pub driver_version: String,
    /// True if this adapter is what Windows Graphics Capture will actually
    /// composite against (relevant on hybrid-graphics laptops).
    pub is_capture_adapter: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GpuVendor {
    Nvidia,
    Amd,
    Intel,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorInfo {
    pub id: String,
    /// Raw HMONITOR value. The UI passes this straight back to
    /// `start_recording`/`take_screenshot`, so monitor selection survives a
    /// display being added or removed between enumeration and use — an
    /// index into the list would silently point at the wrong display.
    pub handle_id: isize,
    pub friendly_name: String,
    pub resolution: Resolution,
    pub refresh_rate_hz: u32,
    pub is_primary: bool,
    pub dpi_scale_percent: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncoderCapability {
    pub backend: EncoderBackend,
    pub codec: VideoCodec,
    pub max_resolution: Resolution,
    pub max_fps_at_max_resolution: u32,
    /// Verified by actually opening and closing the encoder once during
    /// hardware probing (see brail-hardware::probe::verify_encoder), not
    /// just inferred from GPU model — driver/codec-SDK mismatches are
    /// common enough that inference alone would violate "don't advertise
    /// unsupported combinations."
    pub verified: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecommendedPreset {
    LowEnd720p30,
    Gaming1080p60,
    HighEnd1440p60,
    Custom,
}

/// User-configurable global hotkeys. Values are Win32 virtual-key + modifier
/// combinations; validated for conflicts at registration time by
/// brail-hotkeys, which is the only crate allowed to call RegisterHotKey.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyBindings {
    pub start_stop_recording: Option<HotkeyCombo>,
    pub start_stop_streaming: Option<HotkeyCombo>,
    pub save_replay: Option<HotkeyCombo>,
    pub toggle_microphone_mute: Option<HotkeyCombo>,
    pub toggle_desktop_audio_mute: Option<HotkeyCombo>,
    pub toggle_webcam: Option<HotkeyCombo>,
    pub pause_resume_recording: Option<HotkeyCombo>,
    pub take_screenshot: Option<HotkeyCombo>,
}

impl Default for HotkeyBindings {
    fn default() -> Self {
        Self {
            start_stop_recording: Some(HotkeyCombo { ctrl: true, shift: true, alt: false, win: false, key: "F9".into() }),
            start_stop_streaming: Some(HotkeyCombo { ctrl: true, shift: true, alt: false, win: false, key: "F10".into() }),
            save_replay: Some(HotkeyCombo { ctrl: false, shift: false, alt: false, win: false, key: "F11".into() }),
            toggle_microphone_mute: Some(HotkeyCombo { ctrl: true, shift: true, alt: false, win: false, key: "M".into() }),
            toggle_desktop_audio_mute: Some(HotkeyCombo { ctrl: true, shift: true, alt: false, win: false, key: "D".into() }),
            toggle_webcam: None,
            pause_resume_recording: Some(HotkeyCombo { ctrl: true, shift: true, alt: false, win: false, key: "F8".into() }),
            take_screenshot: Some(HotkeyCombo { ctrl: false, shift: false, alt: false, win: false, key: "F12".into() }),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyCombo {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub win: bool,
    /// Virtual key name, e.g. "F9", "M". Mapped to a VK_* code in
    /// brail-hotkeys::vk.
    pub key: String,
}
