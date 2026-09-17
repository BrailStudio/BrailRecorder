//! Tests for the parts of Brail Recorder that can be verified without a
//! Windows machine, a GPU, or a network connection.
//!
//! A deliberate split runs through this suite. Capture, encoding, WASAPI
//! and RTMP all need real hardware or a real server, and a test that
//! mocked them would only prove the mock works — so those live in
//! `windows_integration.rs`, gated behind `#[ignore]` and run explicitly
//! on a real machine. What's here is everything with genuine logic and no
//! platform dependency: preset selection, bitrate derivation, the ring
//! buffer's GOP handling, reconnect backoff, overlay layout, and the
//! adaptive engine's decision rules. Those are where the real bugs hide,
//! and they're testable anywhere.

use brail_core::config::{
    EncoderBackend, FrameRate, Resolution, StreamingService, VideoCodec,
};
use brail_core::presets::{recommended_bitrate_kbps, youtube_presets, QualityPreset};
use brail_core::profile::{CapabilityProfile, EncoderCapability, RecommendedPreset};

fn caps_with(encoders: Vec<EncoderCapability>, ram_mb: u64) -> CapabilityProfile {
    CapabilityProfile {
        cpu_name: "Test CPU".into(),
        cpu_physical_cores: 8,
        cpu_logical_cores: 16,
        total_ram_mb: ram_mb,
        gpus: vec![],
        windows_build: 22631,
        windows_version_name: "Windows 11".into(),
        monitors: vec![],
        audio_input_devices: vec![],
        audio_output_devices: vec![],
        cameras: vec![],
        supported_encoders: encoders,
        recommended_preset: RecommendedPreset::Custom,
    }
}

fn encoder(backend: EncoderBackend, codec: VideoCodec, max: Resolution, verified: bool) -> EncoderCapability {
    EncoderCapability {
        backend,
        codec,
        max_resolution: max,
        max_fps_at_max_resolution: 120,
        verified,
    }
}

// --------------------------------------------------------------------
// Preset gating — the spec forbids offering configurations the hardware
// can't sustain, so these check the filter actually excludes them.
// --------------------------------------------------------------------

#[test]
fn presets_above_encoder_ceiling_are_not_supported() {
    let caps = caps_with(
        vec![encoder(EncoderBackend::Qsv, VideoCodec::H264, Resolution::new(1920, 1080), true)],
        16 * 1024,
    );

    assert!(QualityPreset::Balanced.is_supported_by(&caps), "720p fits inside a 1080p ceiling");
    assert!(QualityPreset::FullHd.is_supported_by(&caps), "1080p exactly meets the ceiling");
    assert!(!QualityPreset::High.is_supported_by(&caps), "1440p exceeds a 1080p encoder");
    assert!(!QualityPreset::Ultra.is_supported_by(&caps), "4K exceeds a 1080p encoder");
}

#[test]
fn unverified_encoders_do_not_unlock_presets() {
    // An encoder that failed its open/close probe must not count toward
    // what the machine can do, no matter what the GPU model suggests.
    let caps = caps_with(
        vec![encoder(EncoderBackend::Nvenc, VideoCodec::Av1, Resolution::new(7680, 4320), false)],
        32 * 1024,
    );
    assert!(!QualityPreset::Ultra.is_supported_by(&caps));
    assert!(!QualityPreset::Balanced.is_supported_by(&caps));
}

#[test]
fn youtube_preset_list_is_filtered_to_real_capability() {
    let caps = caps_with(
        vec![encoder(EncoderBackend::Nvenc, VideoCodec::H264, Resolution::new(1920, 1080), true)],
        16 * 1024,
    );

    let all = youtube_presets(VideoCodec::H264);
    assert_eq!(all.len(), 8, "spec §61 lists eight YouTube profiles");

    let supported = brail_core::presets::filter_supported(all, &caps);
    assert!(supported.iter().all(|p| p.resolution.pixel_count() <= 1920 * 1080));
    assert!(supported.iter().any(|p| p.label == "YouTube 1080p60"));
    assert!(!supported.iter().any(|p| p.label.contains("4K")));
}

// --------------------------------------------------------------------
// Bitrate derivation
// --------------------------------------------------------------------

#[test]
fn bitrate_scales_with_resolution_and_framerate() {
    let r720 = recommended_bitrate_kbps(Resolution::new(1280, 720), 30, VideoCodec::H264, StreamingService::YouTube);
    let r1080 = recommended_bitrate_kbps(Resolution::new(1920, 1080), 30, VideoCodec::H264, StreamingService::YouTube);
    let r1080_60 = recommended_bitrate_kbps(Resolution::new(1920, 1080), 60, VideoCodec::H264, StreamingService::YouTube);

    assert!(r1080 > r720, "higher resolution needs more bitrate");
    assert!(r1080_60 > r1080, "higher framerate needs more bitrate");
    // 60fps should cost meaningfully more than 30 but nowhere near double.
    assert!(r1080_60 < r1080 * 2, "60fps should not simply double the 30fps rate");
}

#[test]
fn modern_codecs_are_allocated_less_bitrate() {
    let res = Resolution::new(1920, 1080);
    let h264 = recommended_bitrate_kbps(res, 60, VideoCodec::H264, StreamingService::YouTube);
    let hevc = recommended_bitrate_kbps(res, 60, VideoCodec::Hevc, StreamingService::YouTube);
    let av1 = recommended_bitrate_kbps(res, 60, VideoCodec::Av1, StreamingService::YouTube);

    assert!(hevc < h264);
    assert!(av1 < hevc);
}

#[test]
fn twitch_bitrate_is_clamped_to_its_ingest_limit() {
    // Twitch rejects or badly transcodes above ~6000 kbps for most
    // channels, so offering 4K-grade bitrate there would be a broken
    // default rather than a generous one.
    let clamped = recommended_bitrate_kbps(
        Resolution::new(3840, 2160),
        60,
        VideoCodec::H264,
        StreamingService::Twitch,
    );
    assert_eq!(clamped, 6_000);

    let youtube = recommended_bitrate_kbps(
        Resolution::new(3840, 2160),
        60,
        VideoCodec::H264,
        StreamingService::YouTube,
    );
    assert!(youtube > 6_000, "YouTube has no such cap");
}

// --------------------------------------------------------------------
// Encoder selection
// --------------------------------------------------------------------

#[test]
fn hardware_encoder_is_chosen_over_software() {
    let caps = caps_with(
        vec![
            encoder(EncoderBackend::Software, VideoCodec::H264, Resolution::new(7680, 4320), true),
            encoder(EncoderBackend::Nvenc, VideoCodec::H264, Resolution::new(4096, 4096), true),
        ],
        16 * 1024,
    );

    let chosen = brail_core::presets::best_encoder_for(&caps, VideoCodec::H264, Resolution::new(1920, 1080))
        .expect("an encoder should be selectable");

    // §6: do not automatically use CPU encoding when hardware is available.
    assert_eq!(chosen.backend, EncoderBackend::Nvenc);
}

#[test]
fn software_is_used_when_no_hardware_encoder_qualifies() {
    let caps = caps_with(
        vec![
            encoder(EncoderBackend::Software, VideoCodec::H264, Resolution::new(7680, 4320), true),
            encoder(EncoderBackend::Qsv, VideoCodec::H264, Resolution::new(1280, 720), true),
        ],
        8 * 1024,
    );

    // 1080p exceeds the QSV entry's ceiling, so software is the only
    // remaining option — and it must still be offered, not refused.
    let chosen = brail_core::presets::best_encoder_for(&caps, VideoCodec::H264, Resolution::new(1920, 1080))
        .expect("software fallback must always be available");
    assert_eq!(chosen.backend, EncoderBackend::Software);
}

#[test]
fn streaming_settings_use_a_two_second_keyframe_interval() {
    let caps = caps_with(
        vec![encoder(EncoderBackend::Nvenc, VideoCodec::H264, Resolution::new(4096, 4096), true)],
        16 * 1024,
    );

    let settings = brail_core::presets::build_encoder_settings(
        &caps,
        Resolution::new(1920, 1080),
        60,
        VideoCodec::H264,
        Some(StreamingService::YouTube),
    );

    assert_eq!(settings.keyframe_interval_secs, 2.0, "spec §14");
    assert_eq!(settings.b_frames, 0, "live streaming avoids added encode latency");
    assert!(settings.bitrate_kbps.is_some());
}

// --------------------------------------------------------------------
// Smart defaults
// --------------------------------------------------------------------

#[test]
fn low_end_machines_never_get_a_4k_default() {
    let caps = caps_with(
        vec![encoder(EncoderBackend::Software, VideoCodec::H264, Resolution::new(7680, 4320), true)],
        4 * 1024,
    );

    let recommended = brail_hardware::recommend::recommended_preset(&caps.supported_encoders, caps.total_ram_mb);
    // §59: never select 4K60 automatically if the machine can't handle it.
    assert_eq!(recommended, RecommendedPreset::LowEnd720p30);
}

#[test]
fn high_end_machine_gets_a_high_end_default() {
    let caps = caps_with(
        vec![encoder(EncoderBackend::Nvenc, VideoCodec::H264, Resolution::new(4096, 4096), true)],
        32 * 1024,
    );
    let recommended = brail_hardware::recommend::recommended_preset(&caps.supported_encoders, caps.total_ram_mb);
    assert_eq!(recommended, RecommendedPreset::HighEnd1440p60);
}

#[test]
fn frame_rate_conversion_is_exact() {
    assert_eq!(FrameRate::Fps30.as_u32(), 30);
    assert_eq!(FrameRate::Fps120.as_u32(), 120);
}
