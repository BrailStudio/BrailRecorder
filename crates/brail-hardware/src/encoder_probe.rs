use brail_core::config::{EncoderBackend, Resolution, VideoCodec};
use brail_core::profile::{EncoderCapability, GpuInfo, GpuVendor};
use ffmpeg_next as ffmpeg;

/// For every (vendor, codec) combination that's plausible given the GPUs
/// present, actually attempts to open the corresponding FFmpeg hardware
/// encoder (`h264_nvenc`, `hevc_amf`, `av1_qsv`, ...) against a throwaway
/// 64x64 frame and immediately closes it. This is the "verify, don't
/// infer" step the spec requires: a GPU model alone doesn't guarantee the
/// installed driver/codec-SDK actually supports a given codec (e.g. AV1
/// NVENC needs an RTX 40-series card specifically, not just "an NVIDIA
/// GPU").
///
/// Software (libx264) is always included and always verified, since it has
/// no hardware dependency — it's the universal fallback the spec requires
/// to always be available.
pub async fn probe_encoders(gpus: &[GpuInfo]) -> anyhow::Result<Vec<EncoderCapability>> {
    ffmpeg::init()?;

    let mut candidates: Vec<(EncoderBackend, VideoCodec, &str)> = vec![
        (EncoderBackend::Software, VideoCodec::H264, "libx264"),
        (EncoderBackend::Software, VideoCodec::Hevc, "libx265"),
    ];

    let has_vendor = |v: GpuVendor| gpus.iter().any(|g| g.vendor == v);

    if has_vendor(GpuVendor::Nvidia) {
        candidates.push((EncoderBackend::Nvenc, VideoCodec::H264, "h264_nvenc"));
        candidates.push((EncoderBackend::Nvenc, VideoCodec::Hevc, "hevc_nvenc"));
        candidates.push((EncoderBackend::Nvenc, VideoCodec::Av1, "av1_nvenc"));
    }
    if has_vendor(GpuVendor::Amd) {
        candidates.push((EncoderBackend::Amf, VideoCodec::H264, "h264_amf"));
        candidates.push((EncoderBackend::Amf, VideoCodec::Hevc, "hevc_amf"));
        candidates.push((EncoderBackend::Amf, VideoCodec::Av1, "av1_amf"));
    }
    if has_vendor(GpuVendor::Intel) {
        candidates.push((EncoderBackend::Qsv, VideoCodec::H264, "h264_qsv"));
        candidates.push((EncoderBackend::Qsv, VideoCodec::Hevc, "hevc_qsv"));
        candidates.push((EncoderBackend::Qsv, VideoCodec::Av1, "av1_qsv"));
    }

    let mut results = Vec::new();

    for (backend, codec, ffmpeg_name) in candidates {
        // Actually opening a hardware encoder can briefly spike GPU clocks
        // and takes tens of milliseconds; run each probe on a blocking
        // thread so this doesn't stall the async hardware-detection task
        // the UI is awaiting on.
        let name = ffmpeg_name.to_string();
        let verified = tokio::task::spawn_blocking(move || try_open_encoder(&name))
            .await
            .unwrap_or(false);

        // Only report the encoder as supported at all if we could verify
        // it — an unverified hardware encoder is not listed, rather than
        // listed with verified=false, so the UI never offers a preset that
        // silently falls back to software without saying so.
        if verified || backend == EncoderBackend::Software {
            results.push(EncoderCapability {
                backend,
                codec,
                max_resolution: max_resolution_for(backend, codec),
                max_fps_at_max_resolution: max_fps_for(backend),
                verified,
            });
        }
    }

    Ok(results)
}

fn try_open_encoder(ffmpeg_name: &str) -> bool {
    let Ok(codec) = ffmpeg::encoder::find_by_name(ffmpeg_name).ok_or(()) else {
        return false;
    };

    // A minimal valid encoder context: smallest realistic resolution,
    // 30fps, a conservative bitrate. If the driver/SDK genuinely doesn't
    // support this codec, `open_as` fails here rather than later during a
    // real recording.
    let context = ffmpeg::codec::Context::new_with_codec(codec);
    let Ok(mut video) = context.encoder().video() else {
        return false;
    };

    video.set_width(64);
    video.set_height(64);
    video.set_format(ffmpeg::format::Pixel::NV12);
    video.set_time_base(ffmpeg::Rational::new(1, 30));
    video.set_bit_rate(1_000_000);

    video.open_as(codec).is_ok()
}

fn max_resolution_for(backend: EncoderBackend, codec: VideoCodec) -> Resolution {
    // Conservative, vendor-documented ceilings. The true ceiling is
    // queried per-device via NVENC's NvEncGetEncodeCaps /
    // AMF's GetCaps / QSV's mfxVideoParam during a deeper probe pass;
    // these are the safe defaults used until that deeper probe runs.
    match (backend, codec) {
        (EncoderBackend::Software, _) => Resolution::new(7680, 4320),
        (_, VideoCodec::Av1) => Resolution::new(7680, 4320),
        _ => Resolution::new(4096, 4096),
    }
}

fn max_fps_for(backend: EncoderBackend) -> u32 {
    match backend {
        EncoderBackend::Software => 60,
        _ => 120,
    }
}
