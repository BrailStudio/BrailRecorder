import { invoke } from "@tauri-apps/api/core";

// These types mirror brail_core::config / brail_core::profile field-for-field
// (serde's default struct serialization keeps Rust's snake_case field
// names, so the field names below match the Rust struct definitions
// exactly rather than being renamed to JS convention).

export interface Resolution {
  width: number;
  height: number;
}

export type FrameRate = "Fps30" | "Fps60" | "Fps90" | "Fps120";
export type VideoCodec = "H264" | "Hevc" | "Av1";
export type EncoderBackend = "Nvenc" | "Amf" | "Qsv" | "Software";
export type ContainerFormat = "Mkv" | "Mp4" | "WebM";

export interface EncoderSettings {
  backend: EncoderBackend;
  codec: VideoCodec;
  rate_control: "Cbr" | "Vbr" | "Cqp";
  bitrate_kbps: number | null;
  cqp_level: number | null;
  keyframe_interval_secs: number;
  preset: "Fastest" | "Fast" | "Balanced" | "Quality" | "MaxQuality";
  profile: "Baseline" | "Main" | "High";
  b_frames: number;
}

export interface RecordingSettings {
  resolution: Resolution;
  frame_rate: FrameRate;
  encoder: EncoderSettings;
  container: ContainerFormat;
  remux_to: ContainerFormat | null;
  output_dir: string;
  capture_cursor: boolean;
  highlight_cursor: boolean;
}

export interface AudioTrackSettings {
  enabled: boolean;
  muted: boolean;
  volume: number;
  device_id: string | null;
}

export interface AudioSettings {
  desktop: AudioTrackSettings;
  microphone: AudioTrackSettings;
  microphone_gain_db: number;
  noise_suppression: boolean;
  auto_gain_control: boolean;
  sample_rate: number;
  separate_tracks: boolean;
}

export interface WebcamOverlaySettings {
  enabled: boolean;
  device_name: string | null;
  width_percent: number;
  anchor: "TopLeft" | "TopRight" | "BottomLeft" | "BottomRight";
  margin_percent: number;
  mirror: boolean;
  corner_radius_px: number;
  border_px: number;
  border_color: string;
  crop: [number, number, number, number];
}

export type AdaptiveMode =
  | "UltraLite" | "LowEnd" | "Balanced" | "Quality" | "Streaming" | "Custom";

export interface PerformanceSettings {
  adaptive_mode: AdaptiveMode;
  auto_optimize: boolean;
  preview_fps: number;
  resource_monitoring: boolean;
  gaming_mode: boolean;
  hardware_acceleration: boolean;
}

export interface GeneralSettings {
  start_with_windows: boolean;
  minimize_to_tray: boolean;
  show_notifications: boolean;
  filename_format: string;
  theme: string;
  language: string;
}

export interface ScreenshotSettings {
  format: "Png" | "Jpeg";
  jpeg_quality: number;
  directory: string;
}

export interface TestResult {
  success: boolean;
  message: string;
  round_trip_ms: number | null;
}

export interface BenchmarkReport {
  duration_secs: number;
  idle_ram_mb: number;
  peak_ram_mb: number;
  mean_ram_mb: number;
  ram_drift_mb: number;
  mean_cpu_percent: number;
  peak_cpu_percent: number;
  mean_gpu_percent: number | null;
  mean_capture_fps: number;
  target_fps: number;
  dropped_frames: number;
  samples_taken: number;
  summary: string;
}

export interface StreamProfile {
  id: string;
  name: string;
  service: "YouTube" | "Twitch" | "Facebook" | "Custom";
  protocol: "Rtmp" | "Rtmps" | "Srt";
  server_url: string;
  resolution: Resolution;
  frame_rate: FrameRate;
  encoder: EncoderSettings;
  reconnect: { enabled: boolean; max_attempts: number; initial_backoff_ms: number; max_backoff_ms: number };
}

export interface AppConfig {
  ui_mode: "Beginner" | "Advanced";
  general: GeneralSettings;
  recording: RecordingSettings;
  audio: AudioSettings;
  webcam: WebcamOverlaySettings;
  overlays: unknown[];
  screenshot: ScreenshotSettings;
  performance: PerformanceSettings;
  stream_and_record: boolean;
  instant_replay: {
    enabled: boolean;
    buffer_seconds: number;
    resolution: Resolution;
    frame_rate: FrameRate;
    encoder: EncoderSettings;
  };
  stream_profiles: StreamProfile[];
  active_stream_profile: string | null;
  hotkeys: Record<string, { ctrl: boolean; shift: boolean; alt: boolean; win: boolean; key: string } | null>;
}

export interface CapabilityProfile {
  cpu_name: string;
  cpu_physical_cores: number;
  cpu_logical_cores: number;
  total_ram_mb: number;
  gpus: { name: string; vendor: string; dedicated_vram_mb: number; is_capture_adapter: boolean }[];
  windows_build: number;
  windows_version_name: string;
  monitors: {
    id: string;
    handle_id: number;
    friendly_name: string;
    resolution: Resolution;
    refresh_rate_hz: number;
    is_primary: boolean;
  }[];
  supported_encoders: {
    backend: EncoderBackend;
    codec: VideoCodec;
    max_resolution: Resolution;
    max_fps_at_max_resolution: number;
    verified: boolean;
  }[];
  recommended_preset: "LowEnd720p30" | "Gaming1080p60" | "HighEnd1440p60" | "Custom";
}

export interface CaptureSourceDto {
  kind: "monitor" | "window";
  id: number;
  label: string;
}

export const api = {
  getHardwareProfile: () => invoke<CapabilityProfile>("get_hardware_profile"),
  getConfig: () => invoke<AppConfig>("get_config"),
  saveConfig: (config: AppConfig) => invoke<void>("save_config", { config }),
  listCapturableWindows: () => invoke<CaptureSourceDto[]>("list_capturable_windows"),

  startRecording: (monitorHandleId: number) => invoke<void>("start_recording", { monitorHandleId }),
  stopRecording: () => invoke<void>("stop_recording"),

  startStreaming: (profileId: string) => invoke<void>("start_streaming", { profileId }),
  stopStreaming: () => invoke<void>("stop_streaming"),

  addStreamProfile: (profile: StreamProfile, streamKey: string) =>
    invoke<void>("add_stream_profile", { profile, streamKey }),
  removeStreamProfile: (profileId: string) => invoke<void>("remove_stream_profile", { profileId }),

  saveReplay: () => invoke<string>("save_replay"),

  getActiveEncoderLabel: () => invoke<string>("get_active_encoder_label"),
  takeScreenshot: (monitorHandleId: number) =>
    invoke<string>("take_screenshot", { monitorHandleId }),
  testStreamConnection: (profileId: string) =>
    invoke<TestResult>("test_stream_connection", { profileId }),
  runBenchmark: () => invoke<BenchmarkReport>("run_benchmark"),
  pauseRecording: () => invoke<void>("pause_recording"),
  resumeRecording: () => invoke<void>("resume_recording"),
};
