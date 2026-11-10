use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

impl Resolution {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    /// The fixed presets from the spec. `Custom` resolutions bypass this and
    /// are validated directly against the source resolution + hardware caps.
    pub const PRESETS: &'static [(&'static str, Resolution)] = &[
        ("144p", Resolution::new(256, 144)),
        ("240p", Resolution::new(426, 240)),
        ("360p", Resolution::new(640, 360)),
        ("480p", Resolution::new(854, 480)),
        ("540p", Resolution::new(960, 540)),
        ("720p", Resolution::new(1280, 720)),
        ("900p", Resolution::new(1600, 900)),
        ("1080p", Resolution::new(1920, 1080)),
        ("1440p", Resolution::new(2560, 1440)),
        ("2160p", Resolution::new(3840, 2160)),
    ];

    pub fn pixel_count(&self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FrameRate {
    Fps30,
    Fps60,
    Fps90,
    Fps120,
}

impl FrameRate {
    pub fn as_u32(&self) -> u32 {
        match self {
            FrameRate::Fps30 => 30,
            FrameRate::Fps60 => 60,
            FrameRate::Fps90 => 90,
            FrameRate::Fps120 => 120,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoCodec {
    H264,
    Hevc,
    Av1,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EncoderBackend {
    /// NVIDIA NVENC via FFmpeg's h264_nvenc/hevc_nvenc/av1_nvenc.
    Nvenc,
    /// AMD AMF via FFmpeg's h264_amf/hevc_amf/av1_amf.
    Amf,
    /// Intel Quick Sync via FFmpeg's h264_qsv/hevc_qsv/av1_qsv.
    Qsv,
    /// libx264/libx265/libaom-av1 software fallback.
    Software,
}

impl EncoderBackend {
    pub fn display_name(&self, codec: VideoCodec) -> String {
        let codec_name = match codec {
            VideoCodec::H264 => "H.264",
            VideoCodec::Hevc => "HEVC",
            VideoCodec::Av1 => "AV1",
        };
        let backend_name = match self {
            EncoderBackend::Nvenc => "NVIDIA NVENC",
            EncoderBackend::Amf => "AMD AMF",
            EncoderBackend::Qsv => "Intel Quick Sync",
            EncoderBackend::Software => "Software (CPU)",
        };
        format!("{backend_name} {codec_name}")
    }

    pub fn is_hardware(&self) -> bool {
        !matches!(self, EncoderBackend::Software)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RateControlMode {
    Cbr,
    Vbr,
    /// Constant-quality / constant QP hardware mode. `Cqp` carries no
    /// bitrate target — quality is set directly.
    Cqp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncoderSettings {
    pub backend: EncoderBackend,
    pub codec: VideoCodec,
    pub rate_control: RateControlMode,
    pub bitrate_kbps: Option<u32>,
    pub cqp_level: Option<u8>,
    pub keyframe_interval_secs: f32,
    pub preset: EncoderPreset,
    pub profile: EncoderProfile,
    pub b_frames: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EncoderPreset {
    /// Lowest latency / lowest CPU-GPU cost, larger files. Used for the
    /// "Ultra Low" / low-end-PC defaults.
    Fastest,
    Fast,
    Balanced,
    Quality,
    /// Highest quality per bit, most encode cost. Only offered when
    /// hardware headroom is confirmed by brail-hardware's benchmark.
    MaxQuality,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EncoderProfile {
    Baseline,
    Main,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerFormat {
    Mkv,
    Mp4,
    WebM,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingSettings {
    pub resolution: Resolution,
    pub frame_rate: FrameRate,
    pub encoder: EncoderSettings,
    pub container: ContainerFormat,
    /// Record to this resilient container first (MKV is default and
    /// recommended); if set, remux to `remux_to` without re-encoding once
    /// the recording finalizes cleanly.
    pub remux_to: Option<ContainerFormat>,
    pub output_dir: std::path::PathBuf,
    pub capture_cursor: bool,
    pub highlight_cursor: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamingProtocol {
    Rtmp,
    Rtmps,
    Srt,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StreamingService {
    YouTube,
    Twitch,
    Facebook,
    Custom,
}

impl StreamingService {
    /// Official ingest endpoint. Custom profiles supply their own
    /// `server_url` instead of using this. Only YouTube/Twitch/Facebook's
    /// documented primary ingest is baked in here — never a third-party
    /// relay.
    pub fn default_server_url(&self) -> Option<&'static str> {
        match self {
            StreamingService::YouTube => Some("rtmp://a.rtmp.youtube.com/live2"),
            StreamingService::Twitch => Some("rtmp://live.twitch.tv/app"),
            StreamingService::Facebook => Some("rtmps://live-api-s.facebook.com:443/rtmp"),
            StreamingService::Custom => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamProfile {
    pub id: uuid::Uuid,
    pub name: String,
    pub service: StreamingService,
    pub protocol: StreamingProtocol,
    pub server_url: String,
    /// Never logged, never serialized to disk in plaintext — see
    /// brail-security::vault. This field only exists transiently in memory
    /// once the vault has decrypted it for an active connection attempt.
    #[serde(skip)]
    pub stream_key: Option<String>,
    pub resolution: Resolution,
    pub frame_rate: FrameRate,
    pub encoder: EncoderSettings,
    pub reconnect: ReconnectPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconnectPolicy {
    pub enabled: bool,
    pub max_attempts: u32,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            enabled: true,
            max_attempts: 10,
            initial_backoff_ms: 1000,
            max_backoff_ms: 30_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiMode {
    Beginner,
    Advanced,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstantReplaySettings {
    pub enabled: bool,
    /// 15 / 30 / 60 / 120 / 300 are the presets offered in the UI; any
    /// other value is a valid custom duration (§20).
    pub buffer_seconds: u32,
    pub resolution: Resolution,
    pub frame_rate: FrameRate,
    pub encoder: EncoderSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub ui_mode: UiMode,
    pub general: crate::settings::GeneralSettings,
    pub recording: RecordingSettings,
    pub audio: crate::settings::AudioSettings,
    pub webcam: crate::settings::WebcamOverlaySettings,
    pub overlays: Vec<crate::settings::OverlayItem>,
    pub screenshot: crate::settings::ScreenshotSettings,
    pub performance: crate::settings::PerformanceSettings,
    pub instant_replay: InstantReplaySettings,
    pub stream_profiles: Vec<StreamProfile>,
    pub active_stream_profile: Option<uuid::Uuid>,
    pub hotkeys: crate::profile::HotkeyBindings,
    /// When true, a single encode is routed to both the stream and a local
    /// recording file (§51) instead of running two encoders. Only possible
    /// when the recording and stream settings match; the UI disables the
    /// option and explains why when they differ.
    pub stream_and_record: bool,
}
