use serde::{Deserialize, Serialize};

use crate::config::{
    EncoderPreset, EncoderProfile, EncoderSettings, FrameRate, RateControlMode, Resolution,
    StreamingService, VideoCodec,
};
use crate::profile::{CapabilityProfile, EncoderCapability};

/// The six stream quality presets from the spec (§12). Each maps to a
/// concrete resolution/fps/bitrate, but is only ever *offered* to the user
/// if `is_supported_by` confirms the hardware can actually sustain it —
/// the spec is explicit that we must not pretend a machine can handle a
/// configuration it can't.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum QualityPreset {
    UltraLow,
    Low,
    Balanced,
    FullHd,
    High,
    Ultra,
}

impl QualityPreset {
    pub const ALL: &'static [QualityPreset] = &[
        QualityPreset::UltraLow,
        QualityPreset::Low,
        QualityPreset::Balanced,
        QualityPreset::FullHd,
        QualityPreset::High,
        QualityPreset::Ultra,
    ];

    pub fn display_name(&self) -> &'static str {
        match self {
            QualityPreset::UltraLow => "Ultra Low",
            QualityPreset::Low => "Low",
            QualityPreset::Balanced => "Balanced",
            QualityPreset::FullHd => "Full HD",
            QualityPreset::High => "High",
            QualityPreset::Ultra => "Ultra",
        }
    }

    pub fn resolution(&self) -> Resolution {
        match self {
            QualityPreset::UltraLow => Resolution::new(640, 360),
            QualityPreset::Low => Resolution::new(854, 480),
            QualityPreset::Balanced => Resolution::new(1280, 720),
            QualityPreset::FullHd => Resolution::new(1920, 1080),
            QualityPreset::High => Resolution::new(2560, 1440),
            QualityPreset::Ultra => Resolution::new(3840, 2160),
        }
    }

    pub fn frame_rate(&self) -> FrameRate {
        match self {
            QualityPreset::UltraLow | QualityPreset::Low => FrameRate::Fps30,
            _ => FrameRate::Fps60,
        }
    }

    /// Whether this machine can plausibly sustain the preset. "Plausibly"
    /// means: a verified encoder exists whose reported ceiling covers this
    /// resolution. It deliberately does not claim the machine will hit the
    /// target framerate under game load — that's measured live by the
    /// Adaptive Engine, not predicted here.
    pub fn is_supported_by(&self, caps: &CapabilityProfile) -> bool {
        let needed = self.resolution().pixel_count();
        caps.supported_encoders
            .iter()
            .any(|e| e.verified && e.max_resolution.pixel_count() >= needed)
    }
}

/// Recommended video bitrate in kbps for a given resolution/fps against a
/// given service. Values follow each service's published live-streaming
/// guidance for H.264; HEVC/AV1 get a reduction since they achieve
/// comparable quality at lower bitrates.
///
/// The spec explicitly forbids hardcoding one bitrate for every machine —
/// this is a *starting point* derived from the selected resolution, fps,
/// codec and service, and every value remains editable in Advanced mode.
pub fn recommended_bitrate_kbps(
    resolution: Resolution,
    fps: u32,
    codec: VideoCodec,
    service: StreamingService,
) -> u32 {
    let pixels = resolution.pixel_count();

    // Base H.264 rates at 30fps, interpolated by pixel count against the
    // common rungs each service publishes.
    let base_30fps = match pixels {
        p if p <= 640 * 360 => 800,
        p if p <= 854 * 480 => 1_200,
        p if p <= 1280 * 720 => 3_000,
        p if p <= 1920 * 1080 => 6_000,
        p if p <= 2560 * 1440 => 12_000,
        _ => 25_000,
    };

    // Higher framerates need roughly 1.5x, not 2x — motion between frames
    // is smaller so inter-frame prediction is more efficient.
    let fps_scaled = if fps > 30 {
        (base_30fps as f64 * 1.5) as u32
    } else {
        base_30fps
    };

    let codec_scaled = match codec {
        VideoCodec::H264 => fps_scaled,
        VideoCodec::Hevc => (fps_scaled as f64 * 0.75) as u32,
        VideoCodec::Av1 => (fps_scaled as f64 * 0.65) as u32,
    };

    // Twitch caps non-partner ingest around 6000 kbps; exceeding it gets
    // the stream transcoded badly or rejected, so it's clamped rather than
    // offered.
    match service {
        StreamingService::Twitch => codec_scaled.min(6_000),
        _ => codec_scaled,
    }
}

/// The eight YouTube profiles from the spec (§61), generated rather than
/// hardcoded so each one's bitrate is derived from the same table above
/// and stays consistent with the custom-resolution path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServicePreset {
    pub label: String,
    pub resolution: Resolution,
    pub frame_rate: FrameRate,
    pub bitrate_kbps: u32,
    pub codec: VideoCodec,
}

pub fn youtube_presets(codec: VideoCodec) -> Vec<ServicePreset> {
    let rungs: &[(&str, Resolution, FrameRate)] = &[
        ("YouTube 720p30", Resolution::new(1280, 720), FrameRate::Fps30),
        ("YouTube 720p60", Resolution::new(1280, 720), FrameRate::Fps60),
        ("YouTube 1080p30", Resolution::new(1920, 1080), FrameRate::Fps30),
        ("YouTube 1080p60", Resolution::new(1920, 1080), FrameRate::Fps60),
        ("YouTube 1440p30", Resolution::new(2560, 1440), FrameRate::Fps30),
        ("YouTube 1440p60", Resolution::new(2560, 1440), FrameRate::Fps60),
        ("YouTube 4K30", Resolution::new(3840, 2160), FrameRate::Fps30),
        ("YouTube 4K60", Resolution::new(3840, 2160), FrameRate::Fps60),
    ];

    rungs
        .iter()
        .map(|(label, res, fps)| ServicePreset {
            label: (*label).to_string(),
            resolution: *res,
            frame_rate: *fps,
            bitrate_kbps: recommended_bitrate_kbps(*res, fps.as_u32(), codec, StreamingService::YouTube),
            codec,
        })
        .collect()
}

/// Filters a preset list down to only what this machine's verified
/// encoders can handle — the spec requires that unsupported profiles are
/// not displayed at all, rather than shown and then failing at start time.
pub fn filter_supported(presets: Vec<ServicePreset>, caps: &CapabilityProfile) -> Vec<ServicePreset> {
    presets
        .into_iter()
        .filter(|p| {
            caps.supported_encoders.iter().any(|e: &EncoderCapability| {
                e.verified
                    && e.codec == p.codec
                    && e.max_resolution.pixel_count() >= p.resolution.pixel_count()
                    && e.max_fps_at_max_resolution >= p.frame_rate.as_u32()
            })
        })
        .collect()
}

/// Picks the best available encoder backend for a codec, preferring
/// hardware over software as the spec requires ("do not automatically use
/// CPU encoding when a suitable hardware encoder is available"). Returns
/// `None` only if nothing at all supports the codec.
pub fn best_encoder_for(
    caps: &CapabilityProfile,
    codec: VideoCodec,
    resolution: Resolution,
) -> Option<&EncoderCapability> {
    caps.supported_encoders
        .iter()
        .filter(|e| {
            e.verified && e.codec == codec && e.max_resolution.pixel_count() >= resolution.pixel_count()
        })
        // Hardware first; among hardware, any is fine (a machine realistically
        // has at most one vendor's encoder verified for a given codec).
        .min_by_key(|e| if e.backend.is_hardware() { 0 } else { 1 })
}

/// Builds a complete `EncoderSettings` for a target, applying the spec's
/// service defaults (2-second keyframe interval for live streaming) and
/// choosing the best verified encoder available.
pub fn build_encoder_settings(
    caps: &CapabilityProfile,
    resolution: Resolution,
    fps: u32,
    codec: VideoCodec,
    service: Option<StreamingService>,
) -> EncoderSettings {
    let backend = best_encoder_for(caps, codec, resolution)
        .map(|e| e.backend)
        .unwrap_or(crate::config::EncoderBackend::Software);

    let bitrate = service
        .map(|s| recommended_bitrate_kbps(resolution, fps, codec, s))
        .unwrap_or_else(|| recommended_bitrate_kbps(resolution, fps, codec, StreamingService::Custom));

    EncoderSettings {
        backend,
        codec,
        // Live streaming needs CBR for predictable ingest behavior; local
        // recording defaults to VBR for better quality per byte.
        rate_control: if service.is_some() { RateControlMode::Cbr } else { RateControlMode::Vbr },
        bitrate_kbps: Some(bitrate),
        cqp_level: None,
        // 2 seconds is what every major ingest expects for segment
        // alignment; shorter wastes bitrate, longer delays viewer joins.
        keyframe_interval_secs: 2.0,
        preset: EncoderPreset::Balanced,
        profile: EncoderProfile::High,
        // B-frames improve compression but add encode latency; live
        // streaming keeps them minimal, recording can afford more.
        b_frames: if service.is_some() { 0 } else { 2 },
    }
}
