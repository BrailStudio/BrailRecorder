use std::time::{Duration, Instant};

/// Degradation ladder applied, in order, when sustained upload bandwidth
/// can't keep up with the target bitrate. Each step is tried before moving
/// to the next, and the controller climbs back up the ladder once bandwidth
/// recovers and stays recovered for `RECOVERY_HOLD` — asymmetric on
/// purpose, since flapping between qualities is worse for viewers than
/// staying at a slightly lower one for an extra few seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DegradationStep {
    None,
    ReduceBitrate25Percent,
    ReduceBitrate50Percent,
    HalveFrameRate,
    DropToNextResolutionDown,
}

pub struct AdaptiveBitrateController {
    target_bitrate_kbps: u32,
    current_bitrate_kbps: u32,
    current_step: DegradationStep,
    window: Vec<BandwidthSample>,
    last_step_change: Instant,
}

struct BandwidthSample {
    at: Instant,
    upload_kbps: f64,
    dropped_frames_in_window: u64,
}

const SAMPLE_WINDOW: Duration = Duration::from_secs(10);
const DEGRADE_HOLD: Duration = Duration::from_secs(5);
const RECOVERY_HOLD: Duration = Duration::from_secs(20);
/// Sustained drop rate above this over the sample window is treated as
/// "can't keep up," matching the spec's stream-health warning threshold.
const DROPPED_FRAME_THRESHOLD_PERCENT: f64 = 2.0;

impl AdaptiveBitrateController {
    pub fn new(target_bitrate_kbps: u32) -> Self {
        Self {
            target_bitrate_kbps,
            current_bitrate_kbps: target_bitrate_kbps,
            current_step: DegradationStep::None,
            window: Vec::new(),
            last_step_change: Instant::now(),
        }
    }

    /// Feeds one measurement (call roughly once per second from the
    /// streaming stats loop) and returns `Some(new_bitrate_kbps)` if the
    /// controller decided to change the encoder's target bitrate this
    /// tick, or `None` if it's holding steady.
    pub fn record_sample(&mut self, upload_kbps: f64, dropped_frames_this_tick: u64) -> Option<u32> {
        let now = Instant::now();
        self.window.push(BandwidthSample {
            at: now,
            upload_kbps,
            dropped_frames_in_window: dropped_frames_this_tick,
        });
        self.window.retain(|s| now.duration_since(s.at) <= SAMPLE_WINDOW);

        let total_dropped: u64 = self.window.iter().map(|s| s.dropped_frames_in_window).sum();
        let sample_count = self.window.len().max(1) as f64;
        let avg_drop_percent = (total_dropped as f64 / sample_count).min(100.0);

        let struggling = avg_drop_percent > DROPPED_FRAME_THRESHOLD_PERCENT
            || self.window.iter().all(|s| s.upload_kbps < self.current_bitrate_kbps as f64 * 0.9);

        if struggling && now.duration_since(self.last_step_change) >= DEGRADE_HOLD {
            self.step_down();
            self.last_step_change = now;
            return Some(self.current_bitrate_kbps);
        }

        let recovered = self.window.len() >= 5
            && self
                .window
                .iter()
                .all(|s| s.upload_kbps > self.target_bitrate_kbps as f64 * 1.2 && s.dropped_frames_in_window == 0);

        if recovered
            && self.current_step != DegradationStep::None
            && now.duration_since(self.last_step_change) >= RECOVERY_HOLD
        {
            self.step_up();
            self.last_step_change = now;
            return Some(self.current_bitrate_kbps);
        }

        None
    }

    fn step_down(&mut self) {
        self.current_step = match self.current_step {
            DegradationStep::None => DegradationStep::ReduceBitrate25Percent,
            DegradationStep::ReduceBitrate25Percent => DegradationStep::ReduceBitrate50Percent,
            DegradationStep::ReduceBitrate50Percent => DegradationStep::HalveFrameRate,
            DegradationStep::HalveFrameRate => DegradationStep::DropToNextResolutionDown,
            DegradationStep::DropToNextResolutionDown => DegradationStep::DropToNextResolutionDown,
        };
        self.recompute_bitrate();
        tracing::warn!(step = ?self.current_step, bitrate_kbps = self.current_bitrate_kbps, "stream struggling, degrading quality");
    }

    fn step_up(&mut self) {
        self.current_step = match self.current_step {
            DegradationStep::DropToNextResolutionDown => DegradationStep::HalveFrameRate,
            DegradationStep::HalveFrameRate => DegradationStep::ReduceBitrate50Percent,
            DegradationStep::ReduceBitrate50Percent => DegradationStep::ReduceBitrate25Percent,
            DegradationStep::ReduceBitrate25Percent => DegradationStep::None,
            DegradationStep::None => DegradationStep::None,
        };
        self.recompute_bitrate();
        tracing::info!(step = ?self.current_step, bitrate_kbps = self.current_bitrate_kbps, "upload bandwidth recovered, restoring quality");
    }

    fn recompute_bitrate(&mut self) {
        self.current_bitrate_kbps = match self.current_step {
            DegradationStep::None => self.target_bitrate_kbps,
            DegradationStep::ReduceBitrate25Percent => (self.target_bitrate_kbps as f64 * 0.75) as u32,
            DegradationStep::ReduceBitrate50Percent => (self.target_bitrate_kbps as f64 * 0.5) as u32,
            // Frame-rate/resolution steps keep bitrate at the 50% floor —
            // it's the encoder's spatial/temporal resolution that changes
            // next, handled by the caller (brail-streaming::session) which
            // reads `current_step` directly for those two cases.
            DegradationStep::HalveFrameRate | DegradationStep::DropToNextResolutionDown => {
                (self.target_bitrate_kbps as f64 * 0.5) as u32
            }
        };
    }

    pub fn current_step(&self) -> DegradationStep {
        self.current_step
    }
}
