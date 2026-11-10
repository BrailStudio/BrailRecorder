use std::collections::VecDeque;

use brail_core::stats::ResourceStats;

/// What the self-tuner recommends changing. Like
/// `brail-encoder::bitrate::AdaptiveBitrateController`, this only ever
/// *recommends* — the caller (the main recording/streaming orchestrator in
/// `src-tauri`) decides whether to apply it, and always surfaces the
/// change to the user via an `AppEvent::Warning` rather than silently
/// changing settings underneath them, per the spec's "never silently
/// degrade without telling the user" requirement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuningAction {
    None,
    SuggestLowerEncoderPreset,
    SuggestLowerResolution,
    SuggestDisableInstantReplay,
    WarnRamCritical,
}

pub struct SelfTuner {
    history: VecDeque<ResourceStats>,
    max_history: usize,
    total_ram_mb: u64,
}

const RAM_WARNING_PERCENT: f64 = 85.0;
const RAM_CRITICAL_PERCENT: f64 = 95.0;
const CPU_SUSTAINED_HIGH_PERCENT: f64 = 90.0;
const SUSTAINED_SAMPLES_REQUIRED: usize = 15; // ~15 seconds at 1Hz sampling

impl SelfTuner {
    pub fn new(total_ram_mb: u64) -> Self {
        Self {
            history: VecDeque::with_capacity(SUSTAINED_SAMPLES_REQUIRED),
            max_history: SUSTAINED_SAMPLES_REQUIRED,
            total_ram_mb,
        }
    }

    /// Feeds one sample and returns a recommendation. Recommendations only
    /// fire once a pressure condition has been sustained for
    /// `SUSTAINED_SAMPLES_REQUIRED` consecutive samples, specifically to
    /// avoid reacting to a brief, harmless spike (e.g. another app
    /// launching) the way a naive single-sample threshold would.
    pub fn record_sample(&mut self, sample: ResourceStats) -> TuningAction {
        self.history.push_back(sample);
        if self.history.len() > self.max_history {
            self.history.pop_front();
        }

        let ram_percent = (sample.process_ram_mb / self.total_ram_mb as f64) * 100.0;
        if ram_percent > RAM_CRITICAL_PERCENT {
            return TuningAction::WarnRamCritical;
        }

        if self.history.len() < SUSTAINED_SAMPLES_REQUIRED {
            return TuningAction::None; // not enough history yet to call anything "sustained"
        }

        let all_ram_high = self
            .history
            .iter()
            .all(|s| (s.process_ram_mb / self.total_ram_mb as f64) * 100.0 > RAM_WARNING_PERCENT);
        if all_ram_high {
            return TuningAction::SuggestDisableInstantReplay;
        }

        let all_cpu_high = self
            .history
            .iter()
            .all(|s| s.process_cpu_percent > CPU_SUSTAINED_HIGH_PERCENT);
        if all_cpu_high {
            return TuningAction::SuggestLowerEncoderPreset;
        }

        let all_gpu_high = self
            .history
            .iter()
            .all(|s| s.gpu_video_encode_percent.map(|p| p > 95.0).unwrap_or(false));
        if all_gpu_high {
            return TuningAction::SuggestLowerResolution;
        }

        TuningAction::None
    }
}
