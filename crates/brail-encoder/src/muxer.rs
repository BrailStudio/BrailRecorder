use std::path::Path;

use brail_core::config::{ContainerFormat, VideoCodec};
use brail_core::error::{BrailError, BrailResult};
use brail_core::frame::{EncodedPacket, StreamKind};
use ffmpeg_next as ffmpeg;

/// Writes encoded video+audio packets to a container file. MKV is the
/// spec's recommended default specifically because it's resilient to an
/// unclean shutdown (crash, power loss) — an MKV file with no trailing
/// index/duration written is still fully playable up to the last flushed
/// cluster, which is exactly the "never lose more than a few seconds of
/// footage on a crash" requirement. MP4 requires its moov atom to be
/// written or finalized properly to be playable at all, which is why MP4
/// is offered only as a *remux* target performed after a clean stop, never
/// as the live recording target.
pub struct Muxer {
    output: ffmpeg::format::context::Output,
    video_stream_index: usize,
    audio_stream_index: Option<usize>,
    header_written: bool,
}

impl Muxer {
    pub fn create(
        path: &Path,
        container: ContainerFormat,
        video_codec: VideoCodec,
        width: u32,
        height: u32,
        frame_rate: u32,
        has_audio: bool,
    ) -> BrailResult<Self> {
        let format_name = match container {
            ContainerFormat::Mkv => "matroska",
            ContainerFormat::Mp4 => "mp4",
            ContainerFormat::WebM => "webm",
        };

        let mut output = ffmpeg::format::output_as(path, format_name)
            .map_err(|e| BrailError::OutputFileError(path.display().to_string(), e.to_string()))?;

        let codec_id = match video_codec {
            VideoCodec::H264 => ffmpeg::codec::Id::H264,
            VideoCodec::Hevc => ffmpeg::codec::Id::HEVC,
            VideoCodec::Av1 => ffmpeg::codec::Id::AV1,
        };

        let video_stream_index = {
            let mut stream = output
                .add_stream(ffmpeg::codec::encoder::find(codec_id))
                .map_err(|e| BrailError::OutputFileError(path.display().to_string(), e.to_string()))?;
            let params = stream.parameters_mut();
            unsafe {
                (*params.as_mut_ptr()).width = width as i32;
                (*params.as_mut_ptr()).height = height as i32;
                (*params.as_mut_ptr()).codec_id = std::mem::transmute(codec_id);
            }
            stream.set_time_base(ffmpeg::Rational::new(1, frame_rate as i32));
            stream.index()
        };

        let audio_stream_index = if has_audio {
            let mut stream = output
                .add_stream(ffmpeg::codec::encoder::find(ffmpeg::codec::Id::AAC))
                .map_err(|e| BrailError::OutputFileError(path.display().to_string(), e.to_string()))?;
            stream.set_time_base(ffmpeg::Rational::new(1, 48_000));
            Some(stream.index())
        } else {
            None
        };

        Ok(Self {
            output,
            video_stream_index,
            audio_stream_index,
            header_written: false,
        })
    }

    pub fn write_header(&mut self) -> BrailResult<()> {
        self.output
            .write_header()
            .map_err(|e| BrailError::OutputFileError("(header)".into(), e.to_string()))?;
        self.header_written = true;
        Ok(())
    }

    pub fn write_packet(&mut self, packet: &EncodedPacket) -> BrailResult<()> {
        if !self.header_written {
            self.write_header()?;
        }

        let stream_index = match packet.stream {
            StreamKind::Video => self.video_stream_index,
            StreamKind::Audio => self
                .audio_stream_index
                .ok_or_else(|| BrailError::Internal("audio packet with no audio stream configured".into()))?,
        };

        let mut av_packet = ffmpeg::Packet::copy(&packet.data);
        av_packet.set_stream(stream_index);
        av_packet.set_pts(Some(packet.pts_100ns / 1000)); // 100ns -> matches stream time_base scaling done by write_interleaved below
        av_packet.set_dts(Some(packet.dts_100ns / 1000));
        if packet.is_keyframe {
            av_packet.set_flags(ffmpeg::packet::Flags::KEY);
        }

        av_packet
            .write_interleaved(&mut self.output)
            .map_err(|e| BrailError::DiskWriteFailed(e.to_string()))?;

        Ok(())
    }

    /// Finalizes the file (writes the trailer/index). For MKV this is a
    /// nice-to-have that speeds up seeking in players but, per the design
    /// note above, is *not* required for the file to be playable — a crash
    /// before this call still leaves a usable recording, which
    /// `brail-recovery` relies on.
    pub fn finalize(mut self) -> BrailResult<()> {
        self.output
            .write_trailer()
            .map_err(|e| BrailError::FinalizationFailed(e.to_string()))?;
        Ok(())
    }
}

/// Remuxes a finalized MKV recording to MP4 without re-encoding — a pure
/// stream-copy operation, so it's fast (seconds, not minutes) and lossless.
/// Used when the user's `RecordingSettings::remux_to` is `Some(Mp4)`.
pub fn remux(input_path: &Path, output_path: &Path) -> BrailResult<()> {
    let mut input = ffmpeg::format::input(input_path)
        .map_err(|e| BrailError::OutputFileError(input_path.display().to_string(), e.to_string()))?;
    let mut output = ffmpeg::format::output(output_path)
        .map_err(|e| BrailError::OutputFileError(output_path.display().to_string(), e.to_string()))?;

    let mut stream_mapping = vec![];
    for stream in input.streams() {
        let out_index = output
            .add_stream(ffmpeg::encoder::find(ffmpeg::codec::Id::None))
            .map_err(|e| BrailError::OutputFileError(output_path.display().to_string(), e.to_string()))?
            .index();
        stream_mapping.push(out_index);
        let _ = stream;
    }

    output
        .write_header()
        .map_err(|e| BrailError::FinalizationFailed(e.to_string()))?;

    for (stream, mut packet) in input.packets() {
        let out_index = stream_mapping[stream.index()];
        packet.set_stream(out_index);
        packet
            .write_interleaved(&mut output)
            .map_err(|e| BrailError::DiskWriteFailed(e.to_string()))?;
    }

    output
        .write_trailer()
        .map_err(|e| BrailError::FinalizationFailed(e.to_string()))?;

    Ok(())
}
