use serde::{Deserialize, Serialize};

/// Per-source audio controls (§17). Volume and mute are applied in the
/// mixer before encoding; gain is applied to the microphone input only.
/// Levels for the UI meters are published as events rather than polled,
/// so an idle meter costs nothing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioTrackSettings {
    pub enabled: bool,
    pub muted: bool,
    /// 0.0 to 1.0 linear gain applied at mix time.
    pub volume: f32,
    pub device_id: Option<String>,
}

impl Default for AudioTrackSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            muted: false,
            volume: 1.0,
            device_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioSettings {
    pub desktop: AudioTrackSettings,
    pub microphone: AudioTrackSettings,
    /// Extra microphone gain in dB, separate from `volume` because gain is
    /// applied pre-mix (it can clip) while volume is a post-mix trim.
    pub microphone_gain_db: f32,
    /// Off by default per the spec's "do not add expensive audio
    /// processing by default" requirement.
    pub noise_suppression: bool,
    pub auto_gain_control: bool,
    /// 44100 or 48000. 48kHz is the default and what every streaming
    /// service expects.
    pub sample_rate: u32,
    /// Write desktop and microphone as separate tracks in the container
    /// (MKV supports this; MP4 remux preserves it) so they can be balanced
    /// in post rather than being permanently mixed down.
    pub separate_tracks: bool,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            desktop: AudioTrackSettings::default(),
            microphone: AudioTrackSettings::default(),
            microphone_gain_db: 0.0,
            noise_suppression: false,
            auto_gain_control: false,
            sample_rate: 48_000,
            separate_tracks: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OverlayAnchor {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Webcam picture-in-picture overlay (§18). Position is expressed as an
/// anchor plus a margin rather than absolute pixels so the overlay lands
/// correctly regardless of output resolution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebcamOverlaySettings {
    pub enabled: bool,
    pub device_name: Option<String>,
    pub width_percent: f32,
    pub anchor: OverlayAnchor,
    pub margin_percent: f32,
    pub mirror: bool,
    pub corner_radius_px: u32,
    pub border_px: u32,
    pub border_color: String,
    /// Crop as fractions of the source frame (left, top, right, bottom),
    /// applied before scaling — lets a 16:9 webcam be cropped to a square
    /// or tighter headshot without distorting it.
    pub crop: [f32; 4],
}

impl Default for WebcamOverlaySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            device_name: None,
            width_percent: 20.0,
            anchor: OverlayAnchor::BottomRight,
            margin_percent: 2.0,
            mirror: true,
            corner_radius_px: 8,
            border_px: 0,
            border_color: "#3fc7c0".into(),
            crop: [0.0, 0.0, 0.0, 0.0],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum OverlayItem {
    Image {
        id: uuid::Uuid,
        path: std::path::PathBuf,
        anchor: OverlayAnchor,
        width_percent: f32,
        margin_percent: f32,
        opacity: f32,
    },
    Text {
        id: uuid::Uuid,
        content: String,
        anchor: OverlayAnchor,
        font_size_px: u32,
        color: String,
        margin_percent: f32,
    },
    Rect {
        id: uuid::Uuid,
        anchor: OverlayAnchor,
        width_percent: f32,
        height_percent: f32,
        margin_percent: f32,
        color: String,
        opacity: f32,
    },
}

/// The six Brail Adaptive Engine modes (§23). The engine measures real
/// resource usage and either recommends or applies settings changes
/// depending on `auto_apply` — the spec requires never silently destroying
/// quality, so auto-apply is opt-in and every change is announced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdaptiveMode {
    UltraLite,
    LowEnd,
    Balanced,
    Quality,
    Streaming,
    Custom,
}

impl AdaptiveMode {
    pub fn display_name(&self) -> &'static str {
        match self {
            AdaptiveMode::UltraLite => "Ultra Lite",
            AdaptiveMode::LowEnd => "Low-End",
            AdaptiveMode::Balanced => "Balanced",
            AdaptiveMode::Quality => "Quality",
            AdaptiveMode::Streaming => "Streaming",
            AdaptiveMode::Custom => "Custom",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            AdaptiveMode::UltraLite => "Minimum resource usage. Preview off, lowest encoder cost.",
            AdaptiveMode::LowEnd => "Prioritizes game performance over recording quality.",
            AdaptiveMode::Balanced => "Balances quality against system impact.",
            AdaptiveMode::Quality => "Prioritizes video quality. Uses more CPU and GPU.",
            AdaptiveMode::Streaming => "Prioritizes stable delivery over peak quality.",
            AdaptiveMode::Custom => "Your own settings. The engine only warns, never changes.",
        }
    }

    /// Whether the engine may change settings on its own in this mode.
    /// Custom never auto-applies — it's the user's explicit opt-out.
    pub fn allows_auto_apply(&self) -> bool {
        !matches!(self, AdaptiveMode::Custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceSettings {
    pub adaptive_mode: AdaptiveMode,
    /// Master switch for the engine changing anything automatically. Even
    /// when true, `AdaptiveMode::Custom` still only warns.
    pub auto_optimize: bool,
    /// Preview costs real GPU time; the spec calls for disabling it on
    /// low-end machines and in Gaming Mode. 0 disables the preview.
    pub preview_fps: u32,
    pub resource_monitoring: bool,
    /// Gaming Mode (§53): minimizes preview, UI activity, and frame copies
    /// while a game is being captured.
    pub gaming_mode: bool,
    pub hardware_acceleration: bool,
}

impl Default for PerformanceSettings {
    fn default() -> Self {
        Self {
            adaptive_mode: AdaptiveMode::Balanced,
            auto_optimize: true,
            preview_fps: 10,
            resource_monitoring: true,
            gaming_mode: false,
            hardware_acceleration: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScreenshotFormat {
    Png,
    Jpeg,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotSettings {
    pub format: ScreenshotFormat,
    pub jpeg_quality: u8,
    pub directory: std::path::PathBuf,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralSettings {
    pub start_with_windows: bool,
    pub minimize_to_tray: bool,
    pub show_notifications: bool,
    /// Tokens: {type} {date} {time} {resolution} {fps}. Validated by
    /// `brail-storage::output_paths::render_filename` — an unknown token is
    /// left literal rather than erroring, so a typo can't block a recording
    /// from starting.
    pub filename_format: String,
    pub theme: String,
    pub language: String,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            start_with_windows: false, // spec: don't enable startup behavior by default
            minimize_to_tray: true,
            show_notifications: true,
            filename_format: "Brail_{date}_{time}".into(),
            theme: "dark".into(),
            language: "en".into(),
        }
    }
}
