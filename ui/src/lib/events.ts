import { listen } from "@tauri-apps/api/event";

// Mirrors brail_core::events::AppEvent's #[serde(tag = "type", content = "data")]
// representation: every event arrives as { type: "...", data: {...} }.
export type AppEvent =
  | { type: "HardwareDetected"; data: unknown }
  | { type: "RecordingStarted"; data: { output_path: string } }
  | { type: "RecordingStopped"; data: { output_path: string; final_size_bytes: number } }
  | { type: "RecordingPaused"; data: null }
  | { type: "RecordingResumed"; data: null }
  | { type: "RecordingStatsUpdated"; data: { duration_secs: number; file_size_bytes: number; disk_write_mb_per_sec: number } }
  | { type: "RecordingRecovered"; data: { recovered_path: string } }
  | { type: "StreamConnecting"; data: null }
  | { type: "StreamConnected"; data: null }
  | { type: "StreamDisconnected"; data: { reason: string } }
  | { type: "StreamReconnecting"; data: { attempt: number; max_attempts: number } }
  | {
      type: "StreamStatsUpdated";
      data: {
        connection_state: "Idle" | "Connecting" | "Connected" | "Reconnecting" | "Failed";
        duration_secs: number;
        upload_bitrate_kbps: number;
        target_bitrate_kbps: number;
        dropped_frames: number;
        dropped_frames_percent: number;
      };
    }
  | { type: "ReplaySaved"; data: { output_path: string } }
  | { type: "ScreenshotSaved"; data: { output_path: string } }
  | {
      type: "AudioLevels";
      data: { microphone_rms: number; desktop_rms: number; microphone_peak: number; desktop_peak: number };
    }
  | {
      type: "AdaptiveRecommendation";
      data: { action: string; reason: string; severity: string; auto_applied: boolean };
    }
  | { type: "BenchmarkComplete"; data: { summary: string } }
  | { type: "CaptureStatsUpdated"; data: { capture_fps: number; frames_captured: number; frames_dropped_capture: number } }
  | {
      type: "ResourceStatsUpdated";
      data: {
        process_ram_mb: number;
        process_cpu_percent: number;
        gpu_usage_percent: number | null;
        gpu_vram_used_mb: number | null;
      };
    }
  | { type: "CaptureSourceLost"; data: { source_name: string } }
  | { type: "Warning"; data: { code: string; message: string } }
  | { type: "Error"; data: unknown };

export function onAppEvent(handler: (event: AppEvent) => void): Promise<() => void> {
  return listen<AppEvent>("brail://event", (e) => handler(e.payload));
}
