use std::sync::Arc;

use brail_core::config::AppConfig;
use brail_core::profile::CapabilityProfile;
use brail_capture::CaptureEngine;
use brail_replay::ReplayEngine;
use brail_storage::config_store::ConfigStore;
use tokio::sync::Mutex;

/// Everything the Tauri command handlers need, held behind one `Arc` so
/// Tauri's `State<'_, AppState>` extractor works from any command without
/// each command re-deriving its own slice of global state.
///
/// Locking granularity: each engine gets its own `Mutex` rather than one
/// big lock over the whole struct, so e.g. polling hardware stats never
/// blocks on a recording start/stop in progress. This does mean callers
/// must be careful never to hold two of these locks at once in an order
/// that could deadlock against another command doing the reverse — the
/// only two commands that ever need more than one are `start_recording`
/// (capture + recording) and `start_streaming` (capture + streaming),
/// both of which always lock `capture` first, by convention documented
/// here.
pub struct AppState {
    pub config_store: ConfigStore,
    pub config: Mutex<AppConfig>,
    pub hardware_profile: Mutex<Option<CapabilityProfile>>,
    pub capture: Mutex<Option<CaptureEngine>>,
    pub recording: Mutex<Option<RecordingHandle>>,
    pub streaming: Mutex<Option<StreamingHandle>>,
    pub replay: Mutex<Option<ReplayEngine>>,
}

/// Holds what's needed to cleanly stop an in-progress recording: the
/// encode controller (so it can be shut down and flushed) and the writer
/// task that owns the muxer. Kept as a small struct rather than inlining
/// these into `AppState` directly so `stop_recording` has one clear thing
/// to consume.
pub struct RecordingHandle {
    pub output_path: std::path::PathBuf,
    pub encoder: brail_encoder::EncodeController,
    pub writer_task: tokio::task::JoinHandle<()>,
    pub started_at: std::time::Instant,
}

pub struct StreamingHandle {
    pub session_task: tokio::task::JoinHandle<()>,
    pub encoder: brail_encoder::EncodeController,
}

impl AppState {
    pub fn new(config_store: ConfigStore, config: AppConfig) -> Arc<Self> {
        Arc::new(Self {
            config_store,
            config: Mutex::new(config),
            hardware_profile: Mutex::new(None),
            capture: Mutex::new(None),
            recording: Mutex::new(None),
            streaming: Mutex::new(None),
            replay: Mutex::new(None),
        })
    }
}
