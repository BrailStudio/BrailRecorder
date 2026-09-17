use brail_core::frame::{AudioFrame, AudioSource};
use brail_core::settings::{AudioSettings, AudioTrackSettings};

/// Peak and RMS levels for one audio source, published to the UI meters
/// (§17). Computed as a by-product of the gain pass that already walks
/// every sample, so metering costs essentially nothing extra — important
/// given the spec's rule that monitoring must not itself become a
/// performance problem.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct AudioLevels {
    /// Loudest single sample in the block, 0.0 to 1.0.
    pub peak: f32,
    /// Root-mean-square level, which tracks perceived loudness far better
    /// than peak and is what the meter bar should actually show.
    pub rms: f32,
    /// True if any sample hit full scale — surfaced as a clip indicator so
    /// the user can lower gain before it ruins a recording.
    pub clipped: bool,
}

/// Applies per-track volume, mute, and (for the microphone) gain, and
/// returns the resulting levels for metering.
///
/// Muting produces silence rather than skipping the frame: dropping frames
/// entirely would create a timestamp gap the muxer would have to fill,
/// which is a far more expensive problem than writing zeroes.
pub fn apply_track_controls(
    frame: &mut AudioFrame,
    settings: &AudioSettings,
) -> AudioLevels {
    let track: &AudioTrackSettings = match frame.source {
        AudioSource::DesktopLoopback => &settings.desktop,
        AudioSource::Microphone => &settings.microphone,
        AudioSource::Mixed => return measure(&frame.samples),
    };

    let gain_multiplier = if frame.source == AudioSource::Microphone {
        db_to_linear(settings.microphone_gain_db)
    } else {
        1.0
    };

    let scale = if track.muted {
        0.0
    } else {
        track.volume.clamp(0.0, 2.0) * gain_multiplier
    };

    scale_samples_in_place(frame, scale)
}

/// Multiplies every f32 sample by `scale`, measuring as it goes.
fn scale_samples_in_place(frame: &mut AudioFrame, scale: f32) -> AudioLevels {
    let mut buf = frame.samples.to_vec();
    let mut peak = 0.0f32;
    let mut sum_squares = 0.0f64;
    let mut clipped = false;

    for chunk in buf.chunks_exact_mut(4) {
        let mut sample = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]) * scale;

        if sample >= 1.0 || sample <= -1.0 {
            clipped = true;
            sample = sample.clamp(-1.0, 1.0);
        }

        let magnitude = sample.abs();
        if magnitude > peak {
            peak = magnitude;
        }
        sum_squares += (sample as f64) * (sample as f64);

        chunk.copy_from_slice(&sample.to_le_bytes());
    }

    let sample_count = (buf.len() / 4).max(1) as f64;
    frame.samples = bytes::Bytes::from(buf);

    AudioLevels {
        peak,
        rms: (sum_squares / sample_count).sqrt() as f32,
        clipped,
    }
}

fn measure(samples: &bytes::Bytes) -> AudioLevels {
    let mut peak = 0.0f32;
    let mut sum_squares = 0.0f64;

    for chunk in samples.chunks_exact(4) {
        let sample = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        let magnitude = sample.abs();
        if magnitude > peak {
            peak = magnitude;
        }
        sum_squares += (sample as f64) * (sample as f64);
    }

    let sample_count = (samples.len() / 4).max(1) as f64;
    AudioLevels {
        peak,
        rms: (sum_squares / sample_count).sqrt() as f32,
        clipped: peak >= 1.0,
    }
}

fn db_to_linear(db: f32) -> f32 {
    if db == 0.0 {
        1.0
    } else {
        10f32.powf(db / 20.0)
    }
}

/// Converts a linear level to dBFS for display. Silence maps to -inf in
/// the maths, so it's floored at -60 dB, which is where a meter bar should
/// bottom out anyway.
pub fn linear_to_dbfs(level: f32) -> f32 {
    if level <= 0.000_001 {
        -60.0
    } else {
        (20.0 * level.log10()).max(-60.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mute_produces_silence() {
        let samples: Vec<u8> = (0..16).flat_map(|_| 0.5f32.to_le_bytes()).collect();
        let mut frame = AudioFrame {
            samples: bytes::Bytes::from(samples),
            sample_rate: 48_000,
            channels: 2,
            timestamp_100ns: 0,
            source: AudioSource::Microphone,
        };

        let mut settings = AudioSettings::default();
        settings.microphone.muted = true;

        let levels = apply_track_controls(&mut frame, &settings);
        assert_eq!(levels.peak, 0.0);
        assert!(frame.samples.chunks_exact(4).all(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]) == 0.0));
    }

    #[test]
    fn clipping_is_detected_and_limited() {
        let samples: Vec<u8> = (0..8).flat_map(|_| 0.9f32.to_le_bytes()).collect();
        let mut frame = AudioFrame {
            samples: bytes::Bytes::from(samples),
            sample_rate: 48_000,
            channels: 2,
            timestamp_100ns: 0,
            source: AudioSource::Microphone,
        };

        let mut settings = AudioSettings::default();
        settings.microphone.volume = 2.0; // 0.9 * 2.0 = 1.8, well past full scale

        let levels = apply_track_controls(&mut frame, &settings);
        assert!(levels.clipped);
        assert!(levels.peak <= 1.0, "output must be limited to full scale");
    }

    #[test]
    fn dbfs_floor_is_stable_at_silence() {
        assert_eq!(linear_to_dbfs(0.0), -60.0);
        assert!((linear_to_dbfs(1.0) - 0.0).abs() < 0.001);
    }
}
