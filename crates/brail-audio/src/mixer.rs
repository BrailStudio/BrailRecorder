use brail_core::frame::{AudioFrame, AudioSource};

/// Mixes desktop-loopback and microphone audio (both already resampled to
/// `resampler::PIPELINE_SAMPLE_RATE`/`PIPELINE_CHANNELS`) into a single
/// stream. Runs as pure sample addition with clipping protection, not a
/// perceptual loudness mix — matching what every competing recorder does
/// for a "desktop + mic" combined track, and left as a fixed behavior
/// rather than exposing a mix-ratio slider, since the spec's recording
/// settings don't call for one (per-source mute toggles are handled
/// upstream by simply not feeding a muted source's frames into the mixer
/// at all).
pub struct AudioMixer {
    desktop_buffer: Vec<f32>,
    mic_buffer: Vec<f32>,
}

impl AudioMixer {
    pub fn new() -> Self {
        Self {
            desktop_buffer: Vec::new(),
            mic_buffer: Vec::new(),
        }
    }

    pub fn push(&mut self, frame: &AudioFrame) {
        let samples = bytes_to_f32(&frame.samples);
        match frame.source {
            AudioSource::DesktopLoopback => self.desktop_buffer.extend_from_slice(&samples),
            AudioSource::Microphone => self.mic_buffer.extend_from_slice(&samples),
            AudioSource::Mixed => {
                // Already mixed upstream; treated as desktop-equivalent so
                // repeated mixing (e.g. a saved instant-replay clip that
                // re-mixes an already-mixed track) doesn't double-count it
                // against a separate mic buffer.
                self.desktop_buffer.extend_from_slice(&samples);
            }
        }
    }

    /// Drains and mixes as many complete sample-pairs as both buffers have
    /// in common. Whichever source has fewer buffered samples right now
    /// determines how much can be mixed this call — the remainder stays
    /// buffered until its counterpart catches up, which is the expected
    /// steady state since desktop audio and mic packets don't arrive on
    /// synchronized schedules.
    pub fn drain_mixed(&mut self, timestamp_100ns: i64) -> Option<AudioFrame> {
        let len = self.desktop_buffer.len().min(self.mic_buffer.len());
        if len == 0 {
            return None;
        }

        let mut mixed = Vec::with_capacity(len);
        for i in 0..len {
            let sum = self.desktop_buffer[i] + self.mic_buffer[i];
            mixed.push(sum.clamp(-1.0, 1.0)); // hard-limit to prevent clipping distortion
        }

        self.desktop_buffer.drain(..len);
        self.mic_buffer.drain(..len);

        Some(AudioFrame {
            samples: f32_to_bytes(&mixed),
            sample_rate: crate::resampler::PIPELINE_SAMPLE_RATE,
            channels: crate::resampler::PIPELINE_CHANNELS,
            timestamp_100ns,
            source: AudioSource::Mixed,
        })
    }
}

impl Default for AudioMixer {
    fn default() -> Self {
        Self::new()
    }
}

fn bytes_to_f32(data: &bytes::Bytes) -> Vec<f32> {
    data.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

fn f32_to_bytes(samples: &[f32]) -> bytes::Bytes {
    let mut buf = Vec::with_capacity(samples.len() * 4);
    for s in samples {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    bytes::Bytes::from(buf)
}
