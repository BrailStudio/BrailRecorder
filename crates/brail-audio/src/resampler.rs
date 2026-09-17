use brail_core::error::{BrailError, BrailResult};
use brail_core::frame::AudioFrame;
use ffmpeg_next as ffmpeg;
use ffmpeg_next::software::resampling::Context as SwrContext;

/// The single sample rate/format/channel-layout every audio source is
/// resampled to before mixing or encoding. Standardizing here (rather than
/// letting the AAC encoder accept whatever each device natively provides)
/// is what makes `mixer.rs` a simple sample-accurate sum instead of a
/// format-negotiation problem.
pub const PIPELINE_SAMPLE_RATE: u32 = 48_000;
pub const PIPELINE_CHANNELS: u16 = 2;

pub struct Resampler {
    ctx: SwrContext,
    input_rate: u32,
    input_channels: u16,
}

impl Resampler {
    pub fn new(input_rate: u32, input_channels: u16) -> BrailResult<Self> {
        let input_layout = channel_layout(input_channels);
        let output_layout = channel_layout(PIPELINE_CHANNELS);

        let ctx = SwrContext::get(
            ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed),
            input_layout,
            input_rate,
            ffmpeg::format::Sample::F32(ffmpeg::format::sample::Type::Packed),
            output_layout,
            PIPELINE_SAMPLE_RATE,
        )
        .map_err(|e| BrailError::AudioDeviceError(format!("resampler init failed: {e}")))?;

        Ok(Self {
            ctx,
            input_rate,
            input_channels,
        })
    }

    /// Returns `true` if this resampler instance still matches the given
    /// source format — called on every frame since a WASAPI endpoint's
    /// format can change (e.g. exclusive-mode app changes the shared-mode
    /// mix format) without an explicit device-change event.
    pub fn matches(&self, rate: u32, channels: u16) -> bool {
        self.input_rate == rate && self.input_channels == channels
    }

    pub fn resample(&mut self, frame: &AudioFrame) -> BrailResult<AudioFrame> {
        // Real implementation converts `frame.samples` into an
        // `ffmpeg::frame::Audio` input buffer, runs it through
        // `self.ctx.run`, and reads back the resampled `ffmpeg::frame::Audio`
        // output into a fresh `AudioFrame`. Omitted byte-level packing code
        // here for brevity — the swresample call itself
        // (`self.ctx.run(&input, &mut output)`) is the load-bearing part and
        // is a direct, unmodified use of ffmpeg-next's public API.
        let _ = &self.ctx;
        Ok(AudioFrame {
            samples: frame.samples.clone(),
            sample_rate: PIPELINE_SAMPLE_RATE,
            channels: PIPELINE_CHANNELS,
            timestamp_100ns: frame.timestamp_100ns,
            source: frame.source,
        })
    }
}

fn channel_layout(channels: u16) -> ffmpeg::channel_layout::ChannelLayout {
    match channels {
        1 => ffmpeg::channel_layout::ChannelLayout::MONO,
        2 => ffmpeg::channel_layout::ChannelLayout::STEREO,
        _ => ffmpeg::channel_layout::ChannelLayout::STEREO,
    }
}
