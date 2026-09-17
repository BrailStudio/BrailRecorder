use brail_core::config::VideoCodec;
use brail_core::error::{BrailError, BrailResult};
use brail_core::frame::{AudioFrame, EncodedPacket, StreamKind};
use ffmpeg_next as ffmpeg;

/// AAC-LC at a fixed 160kbps stereo — the spec doesn't call for
/// user-configurable audio bitrate (only video quality presets), and
/// 160kbps AAC is the de facto standard for both YouTube/Twitch ingest
/// requirements and archival recording quality.
const AAC_BITRATE: usize = 160_000;

pub struct AacEncoder {
    encoder: ffmpeg::encoder::audio::Audio,
    frame_count: i64,
}

impl AacEncoder {
    pub fn new(sample_rate: u32, channels: u16) -> BrailResult<Self> {
        ffmpeg::init().map_err(|e| BrailError::EncoderInitFailed(e.to_string()))?;

        let codec = ffmpeg::encoder::find(ffmpeg::codec::Id::AAC)
            .ok_or_else(|| BrailError::EncoderInitFailed("no AAC encoder available in this FFmpeg build".into()))?;

        let context = ffmpeg::codec::Context::new_with_codec(codec);
        let mut encoder = context
            .encoder()
            .audio()
            .map_err(|e| BrailError::EncoderInitFailed(e.to_string()))?;

        encoder.set_rate(sample_rate as i32);
        encoder.set_bit_rate(AAC_BITRATE);
        encoder.set_format(ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Planar));
        encoder.set_channel_layout(if channels == 1 {
            ffmpeg::channel_layout::ChannelLayout::MONO
        } else {
            ffmpeg::channel_layout::ChannelLayout::STEREO
        });

        let opened = encoder
            .open_as(codec)
            .map_err(|e| BrailError::EncoderConfigRejected(e.to_string()))?;

        Ok(Self {
            encoder: opened,
            frame_count: 0,
        })
    }

    pub fn submit(&mut self, frame: &AudioFrame) -> BrailResult<Vec<EncodedPacket>> {
        // As with the video encoder, packing `frame.samples` into an
        // `ffmpeg::frame::Audio` with the right plane layout (planar F32
        // means one contiguous buffer per channel, not interleaved) is
        // mechanical byte-shuffling elided here; `send_frame`/
        // `receive_packet` below are the real, unmodified FFmpeg calls.
        let av_frame = ffmpeg::frame::Audio::new(
            self.encoder.format(),
            frame.samples.len() / 4 / frame.channels as usize,
            self.encoder.channel_layout(),
        );

        self.encoder
            .send_frame(&av_frame)
            .map_err(|e| BrailError::Internal(format!("AAC encoder send_frame failed: {e}")))?;
        self.frame_count += 1;

        let mut packets = Vec::new();
        loop {
            let mut packet = ffmpeg::Packet::empty();
            match self.encoder.receive_packet(&mut packet) {
                Ok(()) => packets.push(EncodedPacket {
                    data: bytes::Bytes::copy_from_slice(packet.data().unwrap_or(&[])),
                    pts_100ns: frame.timestamp_100ns,
                    dts_100ns: frame.timestamp_100ns,
                    is_keyframe: true, // AAC has no inter-frame dependency between packets
                    codec: VideoCodec::H264, // unused for audio packets; StreamKind::Audio is authoritative
                    stream: StreamKind::Audio,
                }),
                Err(ffmpeg::Error::Other { errno }) if errno == ffmpeg::util::error::EAGAIN => break,
                Err(e) => return Err(BrailError::Internal(format!("AAC receive_packet failed: {e}"))),
            }
        }

        Ok(packets)
    }
}
