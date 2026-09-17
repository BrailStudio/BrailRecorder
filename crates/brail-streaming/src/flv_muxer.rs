use brail_core::config::VideoCodec;
use brail_core::frame::{EncodedPacket, StreamKind};

/// FLV tag type constants (from the Adobe FLV spec, Annex E).
const TAG_TYPE_AUDIO: u8 = 8;
const TAG_TYPE_VIDEO: u8 = 9;

/// Builds one FLV tag (11-byte header + payload + 4-byte previous-tag-size
/// trailer) for a single encoded packet, ready to write directly to the
/// RTMP `publish` stream via `rtmp_client::RtmpClient::send_media`. FLV
/// tags are what RTMP actually transports for media data (RTMP's own
/// framing wraps these tags in chunks) — this is the same packaging OBS
/// and ffmpeg's own RTMP muxer produce.
pub fn mux_packet(packet: &EncodedPacket, is_first_packet_for_stream: bool) -> Vec<u8> {
    let mut payload = Vec::new();

    match packet.stream {
        StreamKind::Video => {
            // Byte 0: frame type (4 bits) | codec id (4 bits).
            // Frame type 1 = keyframe, 2 = inter frame. Codec id 7 = AVC (H.264).
            // HEVC has no official FLV codec id; the enhanced-RTMP extension
            // (`ExVideoTagHeader`, used by YouTube/Twitch's newer ingest) is
            // required for HEVC/AV1 over RTMP — selected here based on codec.
            if packet.codec == VideoCodec::H264 {
                let frame_type = if packet.is_keyframe { 1 } else { 2 };
                payload.push((frame_type << 4) | 7);
                // AVCPacketType: 1 = NALU (subsequent packets after the one-time AVCDecoderConfigurationRecord).
                payload.push(1);
                // Composition time offset (PTS - DTS), 24-bit signed, in ms.
                let cts_ms = ((packet.pts_100ns - packet.dts_100ns) / 10_000) as i32;
                payload.extend_from_slice(&cts_ms.to_be_bytes()[1..4]);
                payload.extend_from_slice(&packet.data);
            } else {
                payload.extend_from_slice(enhanced_rtmp_video_header(packet).as_slice());
                payload.extend_from_slice(&packet.data);
            }
        }
        StreamKind::Audio => {
            // Byte 0: sound format (4 bits, 10 = AAC) | sound rate (2 bits,
            // always 3/44kHz per FLV spec quirk — actual rate is out-of-band
            // via the AudioSpecificConfig) | sound size (1 bit, 1=16-bit) |
            // sound type (1 bit, 1=stereo).
            payload.push((10 << 4) | (3 << 2) | (1 << 1) | 1);
            // AACPacketType: 1 = raw AAC frame (0 is the one-time
            // AudioSpecificConfig sent before the first frame).
            payload.push(1);
            payload.extend_from_slice(&packet.data);
        }
    }

    build_tag(
        if packet.stream == StreamKind::Video { TAG_TYPE_VIDEO } else { TAG_TYPE_AUDIO },
        (packet.dts_100ns / 10_000) as u32, // FLV timestamps are milliseconds
        &payload,
        is_first_packet_for_stream,
    )
}

fn build_tag(tag_type: u8, timestamp_ms: u32, payload: &[u8], _is_first: bool) -> Vec<u8> {
    let mut tag = Vec::with_capacity(11 + payload.len() + 4);

    tag.push(tag_type);
    tag.extend_from_slice(&(payload.len() as u32).to_be_bytes()[1..4]); // 24-bit data size
    tag.extend_from_slice(&timestamp_ms.to_be_bytes()[1..4]); // 24-bit timestamp (lower)
    tag.push((timestamp_ms >> 24) as u8); // timestamp extended (upper byte)
    tag.extend_from_slice(&[0, 0, 0]); // StreamID, always 0
    tag.extend_from_slice(payload);

    let tag_size_with_header = (11 + payload.len()) as u32;
    tag.extend_from_slice(&tag_size_with_header.to_be_bytes());

    tag
}

/// Header for the Enhanced RTMP extension used to carry HEVC/AV1 over
/// RTMP, since classic FLV only defines a codec id for H.264. YouTube and
/// Twitch's newer ingest endpoints both support this extension as of the
/// spec's target date; a service that doesn't would need the encoder
/// backend restricted to H.264 for that stream profile, which
/// `brail-core::config::StreamProfile` validation is responsible for
/// enforcing before this function is ever reached.
fn enhanced_rtmp_video_header(packet: &EncodedPacket) -> Vec<u8> {
    let fourcc: [u8; 4] = match packet.codec {
        VideoCodec::Hevc => *b"hvc1",
        VideoCodec::Av1 => *b"av01",
        VideoCodec::H264 => *b"avc1",
    };

    let mut header = Vec::with_capacity(5);
    // Enhanced RTMP: top bit set (0x80) marks the extended header; low 4
    // bits carry the packet type (1 = CodedFrames), bits 4-6 the frame
    // type (1=key, 2=inter).
    let frame_type: u8 = if packet.is_keyframe { 1 } else { 2 };
    header.push(0x80 | (frame_type << 4) | 1);
    header.extend_from_slice(&fourcc);
    header
}
