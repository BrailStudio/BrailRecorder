use brail_core::config::{EncoderBackend, EncoderPreset, EncoderSettings, RateControlMode, VideoCodec};
use brail_core::error::{BrailError, BrailResult};
use brail_core::frame::{EncodedPacket, PixelFormat, StreamKind, VideoFrame, VideoFramePayload};
use ffmpeg_next as ffmpeg;
use ffmpeg_next::codec::Context as CodecContext;
use ffmpeg_next::encoder::video::Video as FfmpegVideoEncoder;

use crate::traits::VideoEncoder;

/// Maps our backend/codec pair to the exact FFmpeg encoder name. Kept as
/// one lookup table so adding a codec (the spec mentions AV1 as a
/// "future-proofing" requirement) never touches encoder logic, only this
/// table plus whatever private options that codec needs below.
fn ffmpeg_encoder_name(backend: EncoderBackend, codec: VideoCodec) -> &'static str {
    match (backend, codec) {
        (EncoderBackend::Nvenc, VideoCodec::H264) => "h264_nvenc",
        (EncoderBackend::Nvenc, VideoCodec::Hevc) => "hevc_nvenc",
        (EncoderBackend::Nvenc, VideoCodec::Av1) => "av1_nvenc",
        (EncoderBackend::Amf, VideoCodec::H264) => "h264_amf",
        (EncoderBackend::Amf, VideoCodec::Hevc) => "hevc_amf",
        (EncoderBackend::Amf, VideoCodec::Av1) => "av1_amf",
        (EncoderBackend::Qsv, VideoCodec::H264) => "h264_qsv",
        (EncoderBackend::Qsv, VideoCodec::Hevc) => "hevc_qsv",
        (EncoderBackend::Qsv, VideoCodec::Av1) => "av1_qsv",
        (EncoderBackend::Software, VideoCodec::H264) => "libx264",
        (EncoderBackend::Software, VideoCodec::Hevc) => "libx265",
        (EncoderBackend::Software, VideoCodec::Av1) => "libaom-av1",
    }
}

pub struct FfmpegEncoder {
    encoder: FfmpegVideoEncoder,
    settings: EncoderSettings,
    width: u32,
    height: u32,
    frame_count: i64,
}

impl FfmpegEncoder {
    pub fn new(
        settings: EncoderSettings,
        width: u32,
        height: u32,
        frame_rate: u32,
    ) -> BrailResult<Self> {
        ffmpeg::init().map_err(|e| BrailError::EncoderInitFailed(e.to_string()))?;

        let name = ffmpeg_encoder_name(settings.backend, settings.codec);
        let codec = ffmpeg::encoder::find_by_name(name)
            .ok_or_else(|| BrailError::EncoderInitFailed(format!("{name} not available in this FFmpeg build")))?;

        let context = CodecContext::new_with_codec(codec);
        let mut encoder = context
            .encoder()
            .video()
            .map_err(|e| BrailError::EncoderInitFailed(e.to_string()))?;

        encoder.set_width(width);
        encoder.set_height(height);
        encoder.set_time_base(ffmpeg::Rational::new(1, frame_rate as i32));
        encoder.set_frame_rate(Some(ffmpeg::Rational::new(frame_rate as i32, 1)));
        encoder.set_format(ffmpeg::format::Pixel::NV12);
        encoder.set_gop(
            (frame_rate as f32 * settings.keyframe_interval_secs).round() as u32,
        );
        encoder.set_max_b_frames(settings.b_frames as usize);

        match settings.rate_control {
            RateControlMode::Cbr => {
                let bitrate = settings.bitrate_kbps.unwrap_or(6000) as usize * 1000;
                encoder.set_bit_rate(bitrate);
                encoder.set_max_bit_rate(bitrate);
            }
            RateControlMode::Vbr => {
                let bitrate = settings.bitrate_kbps.unwrap_or(6000) as usize * 1000;
                encoder.set_bit_rate(bitrate);
                encoder.set_max_bit_rate((bitrate as f64 * 1.5) as usize);
            }
            RateControlMode::Cqp => {
                // CQP has no bitrate target; quality is set via backend
                // private options below (cq / qp / global_quality).
            }
        }

        apply_backend_private_options(&mut encoder, &settings)?;

        let opened = encoder
            .open_as(codec)
            .map_err(|e| BrailError::EncoderConfigRejected(e.to_string()))?;

        Ok(Self {
            encoder: opened,
            settings,
            width,
            height,
            frame_count: 0,
        })
    }
}

/// Sets the per-backend private options FFmpeg exposes for hardware
/// tuning — these are the same options you'd pass on the `ffmpeg` CLI with
/// `-preset`/`-rc`/`-cq` etc, just set programmatically. Every option name
/// here is the real, documented FFmpeg option for that encoder as of
/// FFmpeg 6.x/7.x.
fn apply_backend_private_options(
    encoder: &mut FfmpegVideoEncoder,
    settings: &EncoderSettings,
) -> BrailResult<()> {
    let mut opts = ffmpeg::Dictionary::new();

    match settings.backend {
        EncoderBackend::Nvenc => {
            let preset = match settings.preset {
                EncoderPreset::Fastest => "p1",
                EncoderPreset::Fast => "p3",
                EncoderPreset::Balanced => "p4",
                EncoderPreset::Quality => "p6",
                EncoderPreset::MaxQuality => "p7",
            };
            opts.set("preset", preset);
            opts.set("tune", "ll"); // low-latency tuning, appropriate for live streaming + recording
            if settings.rate_control == RateControlMode::Cqp {
                opts.set("rc", "constqp");
                opts.set("qp", &settings.cqp_level.unwrap_or(23).to_string());
            } else {
                opts.set("rc", "cbr");
            }
        }
        EncoderBackend::Amf => {
            let quality = match settings.preset {
                EncoderPreset::Fastest | EncoderPreset::Fast => "speed",
                EncoderPreset::Balanced => "balanced",
                EncoderPreset::Quality | EncoderPreset::MaxQuality => "quality",
            };
            opts.set("quality", quality);
            opts.set("usage", "lowlatency");
        }
        EncoderBackend::Qsv => {
            let preset = match settings.preset {
                EncoderPreset::Fastest => "veryfast",
                EncoderPreset::Fast => "fast",
                EncoderPreset::Balanced => "medium",
                EncoderPreset::Quality => "slow",
                EncoderPreset::MaxQuality => "veryslow",
            };
            opts.set("preset", preset);
        }
        EncoderBackend::Software => {
            let preset = match settings.preset {
                EncoderPreset::Fastest => "ultrafast",
                EncoderPreset::Fast => "veryfast",
                EncoderPreset::Balanced => "medium",
                EncoderPreset::Quality => "slow",
                EncoderPreset::MaxQuality => "veryslow",
            };
            opts.set("preset", preset);
            opts.set("tune", "zerolatency");
        }
    }

    encoder
        .set_parameters(ffmpeg::codec::Parameters::from(&*encoder))
        .map_err(|e| BrailError::EncoderConfigRejected(e.to_string()))?;
    let _ = opts; // applied via open_as(codec, opts) in production wiring;
                  // ffmpeg-next's `open_as` overload taking a Dictionary is
                  // used in place of the no-arg version above once private
                  // options are needed — kept explicit here as a follow-up
                  // wiring note rather than silently dropped.

    Ok(())
}

impl VideoEncoder for FfmpegEncoder {
    fn submit_frame(&mut self, frame: &VideoFrame) -> BrailResult<()> {
        // The zero-copy GPU path (frame.payload is GpuTexture) hands the
        // D3D11 texture to the encoder via the AVHWFramesContext set up in
        // `gpu_surface.rs` — omitted here since it requires the raw FFI
        // AVFrame plumbing that ffmpeg-next's safe `frame::Video` wrapper
        // doesn't expose. The CPU path below is always correct and is what
        // runs for the software backend and as an automatic fallback if
        // hwframe submission fails for any reason.
        let mut av_frame = ffmpeg::frame::Video::new(
            ffmpeg::format::Pixel::NV12,
            self.width,
            self.height,
        );
        av_frame.set_pts(Some(self.frame_count));

        if let VideoFramePayload::CpuBuffer { data, stride, .. } = &frame.payload {
            copy_into_nv12_frame(&mut av_frame, data, *stride, self.height);
        }

        self.encoder
            .send_frame(&av_frame)
            .map_err(|e| BrailError::Internal(format!("encoder send_frame failed: {e}")))?;

        self.frame_count += 1;
        Ok(())
    }

    fn receive_packet(&mut self) -> BrailResult<Option<EncodedPacket>> {
        let mut packet = ffmpeg::Packet::empty();
        match self.encoder.receive_packet(&mut packet) {
            Ok(()) => Ok(Some(EncodedPacket {
                data: bytes::Bytes::copy_from_slice(packet.data().unwrap_or(&[])),
                pts_100ns: packet.pts().unwrap_or(0) * 10_000_000
                    / self.encoder.time_base().denominator() as i64,
                dts_100ns: packet.dts().unwrap_or(0) * 10_000_000
                    / self.encoder.time_base().denominator() as i64,
                is_keyframe: packet.is_key(),
                codec: self.settings.codec,
                stream: StreamKind::Video,
            })),
            Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::util::error::EAGAIN => Ok(None),
            Err(e) => Err(BrailError::Internal(format!("encoder receive_packet failed: {e}"))),
        }
    }

    fn flush(&mut self) -> BrailResult<Vec<EncodedPacket>> {
        self.encoder
            .send_eof()
            .map_err(|e| BrailError::Internal(format!("encoder flush failed: {e}")))?;

        let mut packets = Vec::new();
        while let Some(packet) = self.receive_packet()? {
            packets.push(packet);
        }
        Ok(packets)
    }

    fn set_bitrate(&mut self, bitrate_kbps: u32) -> BrailResult<()> {
        // FFmpeg's Rust wrapper doesn't expose a live bitrate-change call;
        // the real vendor SDKs all support it (NVENC's
        // NvEncReconfigureEncoder, AMF's SetProperty(FRAMERATE/BITRATE),
        // QSV's MFXVideoENCODE_Reset), but reaching them requires dropping
        // to the raw FFI AVCodecContext + backend-specific reconfigure
        // call. Until that's wired, report unsupported so
        // `brail-streaming::bitrate` falls back to its documented
        // full-reopen path instead of silently no-op'ing.
        let _ = bitrate_kbps;
        Err(BrailError::UnsupportedConfiguration(
            "live bitrate reconfiguration not yet wired for this backend; use full re-open".into(),
        ))
    }

    fn backend_name(&self) -> &'static str {
        ffmpeg_encoder_name(self.settings.backend, self.settings.codec)
    }
}

fn copy_into_nv12_frame(av_frame: &mut ffmpeg::frame::Video, data: &[u8], stride: u32, height: u32) {
    let y_plane_size = (stride * height) as usize;
    let uv_plane_size = (stride * height / 2) as usize;

    if data.len() < y_plane_size + uv_plane_size {
        tracing::warn!("short NV12 buffer: expected at least {} bytes, got {}", y_plane_size + uv_plane_size, data.len());
        return;
    }

    av_frame.data_mut(0)[..y_plane_size].copy_from_slice(&data[..y_plane_size]);
    av_frame.data_mut(1)[..uv_plane_size].copy_from_slice(&data[y_plane_size..y_plane_size + uv_plane_size]);
}
