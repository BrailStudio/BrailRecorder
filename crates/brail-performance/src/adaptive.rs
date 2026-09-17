//! The **Brail Adaptive Engine** (§23).
//!
//! Collects every signal the app can actually measure — CPU, GPU, RAM,
//! VRAM, encoder utilization, capture FPS vs target FPS, frame queue
//! depth, dropped frames, disk throughput, upload bandwidth, stream health
//! — and turns sustained pressure into a concrete, named recommendation.
//!
//! Two rules from the spec shape the whole design:
//!
//! 1. *"Never automatically reduce quality unless the system actually
//!    needs it."* Every recommendation requires the pressure to persist
//!    across a full observation window, so a one-second spike from another
//!    app launching never triggers anything.
//! 2. *"Do not silently destroy quality."* The engine emits a
//!    `Recommendation` describing what it wants to change and why. Whether
//!    that is applied depends on the user's `AdaptiveMode` and
//!    `auto_optimize` setting, and either way the UI is told.

use std::collections::VecDeque;

use brail_core::settings::AdaptiveMode;
use brail_core::stats::{CaptureStats, EncodeStats, ResourceStats, StreamStats};
use serde::{Deserialize, Serialize};

/// One complete observation of system state at a point in time.
#[derive(Debug, Clone, Copy, Default)]
pub struct AdaptiveSample {
    pub resources: ResourceStats,
    pub capture_fps: f64,
    pub target_fps: f64,
    pub encoder_queue_depth: u32,
    pub dropped_frames_delta: u64,
    pub disk_write_mb_per_sec: f64,
    pub required_disk_mb_per_sec: f64,
    pub upload_kbps: Option<f64>,
    pub target_bitrate_kbps: Option<f64>,
}

/// What the engine wants changed, and the measured reason why. `reason` is
/// written for the user, not for a log — it appears verbatim in the UI
/// warning, so it always cites the actual number that triggered it (§54:
/// "warnings must be based on actual measurements").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Recommendation {
    pub action: AdaptiveAction,
    pub reason: String,
    pub severity: Severity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Severity {
    /// Informational; nothing is wrong yet.
    Info,
    /// Quality or stability is measurably affected.
    Warning,
    /// Recording or streaming is at risk of failing.
    Critical,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdaptiveAction {
    /// Nothing needs to change.
    Hold,
    /// Switch from software to a verified hardware encoder.
    SwitchToHardwareEncoder,
    /// Move to a faster (cheaper) encoder preset.
    LowerEncoderPreset,
    /// Drop the output framerate one rung (60 -> 30).
    LowerFrameRate,
    /// Drop the output resolution one rung.
    LowerResolution,
    /// Reduce the target bitrate — used for disk and upload pressure.
    LowerBitrate,
    /// Turn off the live preview, which costs real GPU time.
    DisablePreview,
    /// Stop the instant-replay encoder, the most expendable continuous
    /// consumer of encoder capacity.
    DisableInstantReplay,
}

impl AdaptiveAction {
    pub fn user_label(&self) -> &'static str {
        match self {
            AdaptiveAction::Hold => "No change needed",
            AdaptiveAction::SwitchToHardwareEncoder => "Switch to hardware encoding",
            AdaptiveAction::LowerEncoderPreset => "Use a faster encoder preset",
            AdaptiveAction::LowerFrameRate => "Reduce frame rate",
            AdaptiveAction::LowerResolution => "Reduce resolution",
            AdaptiveAction::LowerBitrate => "Reduce bitrate",
            AdaptiveAction::DisablePreview => "Turn off preview",
            AdaptiveAction::DisableInstantReplay => "Turn off Instant Replay",
        }
    }
}

/// Samples are taken at 1 Hz; 12 consecutive bad samples means the
/// condition has held for 12 seconds, which is long enough to rule out
/// transient spikes but short enough that a genuinely struggling stream
/// isn't left degraded for a minute before anything happens.
const WINDOW: usize = 12;

pub struct AdaptiveEngine {
    mode: AdaptiveMode,
    auto_optimize: bool,
    history: VecDeque<AdaptiveSample>,
    total_ram_mb: u64,
    has_hardware_encoder: bool,
    using_software_encoder: bool,
    last_recommendation: Option<Recommendation>,
}

impl AdaptiveEngine {
    pub fn new(mode: AdaptiveMode, auto_optimize: bool, total_ram_mb: u64) -> Self {
        Self {
            mode,
            auto_optimize,
            history: VecDeque::with_capacity(WINDOW),
            total_ram_mb,
            has_hardware_encoder: false,
            using_software_encoder: false,
            last_recommendation: None,
        }
    }

    pub fn set_mode(&mut self, mode: AdaptiveMode) {
        self.mode = mode;
        // A mode change invalidates the observation window: the thresholds
        // the previous mode was being judged against no longer apply.
        self.history.clear();
    }

    pub fn set_encoder_state(&mut self, has_hardware: bool, using_software: bool) {
        self.has_hardware_encoder = has_hardware;
        self.using_software_encoder = using_software;
    }

    /// Whether the caller should actually apply a recommendation, versus
    /// only surfacing it as a warning.
    pub fn should_auto_apply(&self) -> bool {
        self.auto_optimize && self.mode.allows_auto_apply()
    }

    /// Feeds one sample and returns a recommendation if a *sustained*
    /// condition warrants one. Returns `None` while the window is still
    /// filling, or when nothing has held long enough to act on.
    pub fn observe(&mut self, sample: AdaptiveSample) -> Option<Recommendation> {
        self.history.push_back(sample);
        if self.history.len() > WINDOW {
            self.history.pop_front();
        }

        // Critical conditions bypass the window — a full disk or exhausted
        // RAM will break the recording within seconds, so waiting twelve of
        // them to confirm the trend would be waiting too long.
        if let Some(critical) = self.check_critical(&sample) {
            self.last_recommendation = Some(critical.clone());
            return Some(critical);
        }

        if self.history.len() < WINDOW {
            return None;
        }

        let rec = self
            .check_software_encoder()
            .or_else(|| self.check_capture_fps_shortfall())
            .or_else(|| self.check_encoder_saturation())
            .or_else(|| self.check_disk_throughput())
            .or_else(|| self.check_upload_bandwidth())
            .or_else(|| self.check_cpu_pressure())?;

        // Don't re-emit the same recommendation every second once the user
        // has already been told; only re-fire if the situation changed.
        if self.last_recommendation.as_ref() == Some(&rec) {
            return None;
        }
        self.last_recommendation = Some(rec.clone());
        Some(rec)
    }

    fn check_critical(&self, s: &AdaptiveSample) -> Option<Recommendation> {
        let ram_percent = (s.resources.process_ram_mb / self.total_ram_mb.max(1) as f64) * 100.0;
        if ram_percent > 90.0 {
            return Some(Recommendation {
                action: AdaptiveAction::DisableInstantReplay,
                reason: format!(
                    "Brail is using {:.0} MB of memory, close to this system's limit.",
                    s.resources.process_ram_mb
                ),
                severity: Severity::Critical,
            });
        }
        None
    }

    /// Using CPU encoding while a verified hardware encoder sits idle is
    /// the single highest-impact fix available, and the spec explicitly
    /// forbids defaulting to CPU when hardware is available.
    fn check_software_encoder(&self) -> Option<Recommendation> {
        if self.using_software_encoder && self.has_hardware_encoder {
            let avg_cpu = self.avg(|s| s.resources.process_cpu_percent);
            if avg_cpu > 25.0 {
                return Some(Recommendation {
                    action: AdaptiveAction::SwitchToHardwareEncoder,
                    reason: format!(
                        "CPU encoding is using {avg_cpu:.0}% CPU. A hardware encoder is available and would use far less."
                    ),
                    severity: Severity::Warning,
                });
            }
        }
        None
    }

    /// Capture consistently delivering well under the target framerate
    /// means the source itself can't keep up — lowering resolution helps
    /// more than anything encoder-side would.
    fn check_capture_fps_shortfall(&self) -> Option<Recommendation> {
        let avg_capture = self.avg(|s| s.capture_fps);
        let target = self.history.back()?.target_fps;
        if target > 0.0 && avg_capture < target * 0.8 {
            return Some(Recommendation {
                action: if self.mode == AdaptiveMode::Quality {
                    AdaptiveAction::LowerFrameRate
                } else {
                    AdaptiveAction::LowerResolution
                },
                reason: format!(
                    "Capturing {avg_capture:.0} FPS against a {target:.0} FPS target for the last {WINDOW} seconds."
                ),
                severity: Severity::Warning,
            });
        }
        None
    }

    /// A persistently deep encoder queue means the encoder is the
    /// bottleneck; a cheaper preset buys headroom without touching
    /// resolution.
    fn check_encoder_saturation(&self) -> Option<Recommendation> {
        let all_backed_up = self.history.iter().all(|s| s.encoder_queue_depth > 4);
        let dropping = self.history.iter().map(|s| s.dropped_frames_delta).sum::<u64>() > 0;
        if all_backed_up && dropping {
            return Some(Recommendation {
                action: AdaptiveAction::LowerEncoderPreset,
                reason: "The encoder has been running behind and dropping frames.".into(),
                severity: Severity::Warning,
            });
        }
        None
    }

    /// Disk that can't absorb the bitrate will eventually stall writes;
    /// this catches it before the recording is damaged (§26).
    fn check_disk_throughput(&self) -> Option<Recommendation> {
        let required = self.history.back()?.required_disk_mb_per_sec;
        if required <= 0.0 {
            return None;
        }
        let avg_actual = self.avg(|s| s.disk_write_mb_per_sec);
        if avg_actual > 0.0 && avg_actual < required * 0.9 {
            return Some(Recommendation {
                action: AdaptiveAction::LowerBitrate,
                reason: format!(
                    "This drive is sustaining {avg_actual:.0} MB/s but the current settings need about {required:.0} MB/s."
                ),
                severity: Severity::Warning,
            });
        }
        None
    }

    /// Upload that can't sustain the target bitrate is the most common
    /// streaming failure. Reported here; the actual bitrate ladder lives in
    /// `brail-encoder::bitrate::AdaptiveBitrateController`, which reacts
    /// faster than this engine because a stream degrades in seconds.
    fn check_upload_bandwidth(&self) -> Option<Recommendation> {
        let target = self.history.back()?.target_bitrate_kbps?;
        let samples: Vec<f64> = self.history.iter().filter_map(|s| s.upload_kbps).collect();
        if samples.len() < WINDOW {
            return None;
        }
        let avg = samples.iter().sum::<f64>() / samples.len() as f64;
        if avg < target * 0.85 {
            return Some(Recommendation {
                action: AdaptiveAction::LowerBitrate,
                reason: format!(
                    "Upload has averaged {:.1} Mbps against a {:.1} Mbps target.",
                    avg / 1000.0,
                    target / 1000.0
                ),
                severity: Severity::Warning,
            });
        }
        None
    }

    fn check_cpu_pressure(&self) -> Option<Recommendation> {
        let avg_cpu = self.avg(|s| s.resources.process_cpu_percent);
        let threshold = match self.mode {
            // Low-End mode protects game performance, so it intervenes
            // much earlier than Quality mode, which accepts high usage as
            // the cost of what the user asked for.
            AdaptiveMode::UltraLite | AdaptiveMode::LowEnd => 15.0,
            AdaptiveMode::Balanced | AdaptiveMode::Streaming => 35.0,
            AdaptiveMode::Quality => 60.0,
            AdaptiveMode::Custom => return None, // Custom mode only warns via other checks
        };

        if avg_cpu > threshold {
            return Some(Recommendation {
                action: AdaptiveAction::DisablePreview,
                reason: format!(
                    "Brail has averaged {avg_cpu:.0}% CPU, above the {} target for {} mode.",
                    threshold as u32,
                    self.mode.display_name()
                ),
                severity: Severity::Info,
            });
        }
        None
    }

    fn avg(&self, f: impl Fn(&AdaptiveSample) -> f64) -> f64 {
        if self.history.is_empty() {
            return 0.0;
        }
        self.history.iter().map(f).sum::<f64>() / self.history.len() as f64
    }
}

/// Builds an `AdaptiveSample` from the individual stat structs the rest of
/// the app already produces, so callers don't have to know the engine's
/// internal shape.
pub fn sample_from_stats(
    resources: ResourceStats,
    capture: CaptureStats,
    encode: EncodeStats,
    stream: Option<&StreamStats>,
    target_fps: u32,
    required_disk_mb_per_sec: f64,
) -> AdaptiveSample {
    AdaptiveSample {
        resources,
        capture_fps: capture.capture_fps,
        target_fps: target_fps as f64,
        encoder_queue_depth: encode.encoder_queue_depth,
        dropped_frames_delta: encode.frames_dropped_encoder_backpressure,
        disk_write_mb_per_sec: resources.disk_write_mb_per_sec.unwrap_or(0.0),
        required_disk_mb_per_sec,
        upload_kbps: stream.map(|s| s.upload_bitrate_kbps),
        target_bitrate_kbps: stream.map(|s| s.target_bitrate_kbps),
    }
}
