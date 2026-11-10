use serde::{Deserialize, Serialize};

use crate::error::BrailError;
use crate::stats::{CaptureStats, EncodeStats, RecordingStats, ResourceStats, StreamStats};

/// Everything the backend pipeline can tell the UI. Emitted over a single
/// Tauri event channel (`brail://event`) so the frontend has one listener
/// instead of a dozen ad-hoc channels — see src-tauri/src/events.rs for the
/// emit side and ui/src/lib/events.ts for the listener.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
pub enum AppEvent {
    HardwareDetected(crate::profile::CapabilityProfile),

    RecordingStarted { output_path: String },
    RecordingStopped { output_path: String, final_size_bytes: u64 },
    RecordingPaused,
    RecordingResumed,
    RecordingStatsUpdated(RecordingStats),
    RecordingRecovered { recovered_path: String },

    StreamConnecting,
    StreamConnected,
    StreamDisconnected { reason: String },
    StreamReconnecting { attempt: u32, max_attempts: u32 },
    StreamStatsUpdated(StreamStats),
    StreamTestResult { success: bool, message: String, round_trip_ms: Option<f64> },

    ReplaySaved { output_path: String },
    ScreenshotSaved { output_path: String },
    ReplayBufferFilled,

    CaptureStatsUpdated(CaptureStats),
    EncodeStatsUpdated(EncodeStats),
    ResourceStatsUpdated(ResourceStats),

    CaptureSourceLost { source_name: String },
    MonitorConfigurationChanged,

    /// Live audio meter levels, batched at the same 1 Hz cadence as the
    /// other stats so the meters never drive high-frequency UI renders.
    AudioLevels { microphone_rms: f32, desktop_rms: f32, microphone_peak: f32, desktop_peak: f32 },

    /// A recommendation from the Brail Adaptive Engine. Emitted whether or
    /// not it was auto-applied, so the user is always told (§13: never
    /// silently destroy quality).
    AdaptiveRecommendation { action: String, reason: String, severity: String, auto_applied: bool },

    BenchmarkComplete { summary: String },

    Warning { code: String, message: String },
    Error(BrailError),
}
