use serde::{Deserialize, Serialize};

/// Live resource usage, sampled on a low-frequency timer (1 Hz by default —
/// see brail-performance::monitor) and pushed to the UI. Every field here is
/// a real measurement, never a placeholder; if a measurement isn't available
/// on a given Windows build, the field is `None` rather than a fake zero.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct ResourceStats {
    pub process_ram_mb: f64,
    pub process_cpu_percent: f64,
    pub system_cpu_percent: Option<f64>,
    pub gpu_usage_percent: Option<f64>,
    pub gpu_video_encode_percent: Option<f64>,
    pub gpu_vram_used_mb: Option<f64>,
    pub disk_write_mb_per_sec: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct CaptureStats {
    pub capture_fps: f64,
    pub frames_captured: u64,
    pub frames_dropped_capture: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct EncodeStats {
    pub encode_fps: f64,
    pub avg_encode_latency_ms: f64,
    pub frames_encoded: u64,
    pub frames_dropped_encoder_backpressure: u64,
    pub encoder_queue_depth: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub struct RecordingStats {
    pub duration_secs: f64,
    pub file_size_bytes: u64,
    pub disk_write_mb_per_sec: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConnectionState {
    Idle,
    Connecting,
    Connected,
    Reconnecting,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamStats {
    pub connection_state: ConnectionState,
    pub duration_secs: f64,
    pub upload_bitrate_kbps: f64,
    pub target_bitrate_kbps: f64,
    pub dropped_frames: u64,
    pub dropped_frames_percent: f64,
    pub encoder_queue_depth: u32,
    pub round_trip_latency_ms: Option<f64>,
    pub reconnect_attempts: u32,
    pub last_error: Option<String>,
}

impl Default for StreamStats {
    fn default() -> Self {
        Self {
            connection_state: ConnectionState::Idle,
            duration_secs: 0.0,
            upload_bitrate_kbps: 0.0,
            target_bitrate_kbps: 0.0,
            dropped_frames: 0,
            dropped_frames_percent: 0.0,
            encoder_queue_depth: 0,
            round_trip_latency_ms: None,
            reconnect_attempts: 0,
            last_error: None,
        }
    }
}
