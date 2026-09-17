use std::time::{Duration, Instant};

use brail_core::stats::ResourceStats;
use serde::{Deserialize, Serialize};

/// Diagnostic report produced by benchmark mode (§64).
///
/// Every field here is a measurement taken during the run. Nothing is
/// estimated, and any measurement the platform couldn't provide is `None`
/// rather than a plausible-looking number — the spec is explicit: do not
/// fake measurements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkReport {
    pub duration_secs: f64,
    pub idle_ram_mb: f64,
    pub peak_ram_mb: f64,
    pub mean_ram_mb: f64,
    /// Difference between the last and first RAM samples. A number that
    /// keeps climbing across longer runs is the leak signal the spec's
    /// resource-leak test (§65) is looking for.
    pub ram_drift_mb: f64,
    pub mean_cpu_percent: f64,
    pub peak_cpu_percent: f64,
    pub mean_gpu_percent: Option<f64>,
    pub mean_capture_fps: f64,
    pub target_fps: f64,
    pub dropped_frames: u64,
    pub mean_encode_latency_ms: f64,
    pub mean_disk_write_mb_per_sec: Option<f64>,
    pub mean_upload_kbps: Option<f64>,
    pub samples_taken: usize,
    /// One-line plain-language verdict shown in the UI.
    pub summary: String,
}

/// Collects samples over a fixed window and computes the report.
/// Deliberately a passive accumulator rather than something that drives
/// the pipeline itself: the caller runs a genuine recording or stream and
/// feeds real samples in, so the benchmark measures the actual app doing
/// actual work, not a synthetic loop that would prove nothing.
pub struct BenchmarkRun {
    started: Instant,
    target_duration: Duration,
    idle_ram_mb: f64,
    target_fps: f64,
    ram: Vec<f64>,
    cpu: Vec<f64>,
    gpu: Vec<f64>,
    capture_fps: Vec<f64>,
    encode_latency_ms: Vec<f64>,
    disk_mbps: Vec<f64>,
    upload_kbps: Vec<f64>,
    dropped_frames_start: u64,
    dropped_frames_latest: u64,
}

impl BenchmarkRun {
    /// `idle_ram_mb` must be sampled *before* any capture or encoding
    /// starts — it's the baseline the spec's 20–50 MB idle target is
    /// measured against.
    pub fn start(idle_ram_mb: f64, target_fps: u32, duration: Duration) -> Self {
        Self {
            started: Instant::now(),
            target_duration: duration,
            idle_ram_mb,
            target_fps: target_fps as f64,
            ram: Vec::new(),
            cpu: Vec::new(),
            gpu: Vec::new(),
            capture_fps: Vec::new(),
            encode_latency_ms: Vec::new(),
            disk_mbps: Vec::new(),
            upload_kbps: Vec::new(),
            dropped_frames_start: 0,
            dropped_frames_latest: 0,
        }
    }

    pub fn record(
        &mut self,
        resources: ResourceStats,
        capture_fps: f64,
        encode_latency_ms: f64,
        dropped_frames_total: u64,
        upload_kbps: Option<f64>,
    ) {
        if self.ram.is_empty() {
            self.dropped_frames_start = dropped_frames_total;
        }
        self.dropped_frames_latest = dropped_frames_total;

        self.ram.push(resources.process_ram_mb);
        self.cpu.push(resources.process_cpu_percent);
        if let Some(g) = resources.gpu_usage_percent {
            self.gpu.push(g);
        }
        if let Some(d) = resources.disk_write_mb_per_sec {
            self.disk_mbps.push(d);
        }
        if let Some(u) = upload_kbps {
            self.upload_kbps.push(u);
        }
        self.capture_fps.push(capture_fps);
        self.encode_latency_ms.push(encode_latency_ms);
    }

    pub fn is_complete(&self) -> bool {
        self.started.elapsed() >= self.target_duration
    }

    pub fn finish(self) -> BenchmarkReport {
        let mean = |v: &[f64]| if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 };
        let peak = |v: &[f64]| v.iter().cloned().fold(0.0f64, f64::max);
        let optional_mean = |v: &[f64]| if v.is_empty() { None } else { Some(mean(v)) };

        let mean_ram = mean(&self.ram);
        let peak_ram = peak(&self.ram);
        let mean_cpu = mean(&self.cpu);
        let mean_fps = mean(&self.capture_fps);
        let dropped = self.dropped_frames_latest.saturating_sub(self.dropped_frames_start);

        let ram_drift = match (self.ram.first(), self.ram.last()) {
            (Some(first), Some(last)) => last - first,
            _ => 0.0,
        };

        let summary = build_summary(mean_ram, mean_cpu, mean_fps, self.target_fps, dropped, ram_drift);

        BenchmarkReport {
            duration_secs: self.started.elapsed().as_secs_f64(),
            idle_ram_mb: self.idle_ram_mb,
            peak_ram_mb: peak_ram,
            mean_ram_mb: mean_ram,
            ram_drift_mb: ram_drift,
            mean_cpu_percent: mean_cpu,
            peak_cpu_percent: peak(&self.cpu),
            mean_gpu_percent: optional_mean(&self.gpu),
            mean_capture_fps: mean_fps,
            target_fps: self.target_fps,
            dropped_frames: dropped,
            mean_encode_latency_ms: mean(&self.encode_latency_ms),
            mean_disk_write_mb_per_sec: optional_mean(&self.disk_mbps),
            mean_upload_kbps: optional_mean(&self.upload_kbps),
            samples_taken: self.ram.len(),
            summary,
        }
    }
}

fn build_summary(
    mean_ram: f64,
    mean_cpu: f64,
    mean_fps: f64,
    target_fps: f64,
    dropped: u64,
    ram_drift: f64,
) -> String {
    let mut parts = vec![format!("{mean_ram:.0} MB RAM, {mean_cpu:.0}% CPU")];

    if target_fps > 0.0 {
        let fps_ratio = mean_fps / target_fps;
        if fps_ratio < 0.95 {
            parts.push(format!("captured {mean_fps:.0} of {target_fps:.0} FPS"));
        } else {
            parts.push(format!("held {target_fps:.0} FPS"));
        }
    }

    if dropped > 0 {
        parts.push(format!("{dropped} frames dropped"));
    }

    // Only call out drift that's large enough to be meaningful — normal
    // allocator behavior moves the working set by a few MB over a short
    // run and flagging that would just train the user to ignore this.
    if ram_drift > 25.0 {
        parts.push(format!("memory grew {ram_drift:.0} MB during the run"));
    }

    parts.join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(ram: f64, cpu: f64) -> ResourceStats {
        ResourceStats {
            process_ram_mb: ram,
            process_cpu_percent: cpu,
            ..Default::default()
        }
    }

    #[test]
    fn report_computes_means_and_drift() {
        let mut run = BenchmarkRun::start(40.0, 60, Duration::from_secs(1));
        run.record(sample(80.0, 10.0), 60.0, 4.0, 0, None);
        run.record(sample(90.0, 20.0), 60.0, 6.0, 0, None);

        let report = run.finish();
        assert_eq!(report.samples_taken, 2);
        assert!((report.mean_ram_mb - 85.0).abs() < 0.01);
        assert!((report.ram_drift_mb - 10.0).abs() < 0.01);
        assert!((report.mean_cpu_percent - 15.0).abs() < 0.01);
        assert_eq!(report.peak_ram_mb, 90.0);
    }

    #[test]
    fn unavailable_gpu_stays_none_rather_than_zero() {
        let mut run = BenchmarkRun::start(40.0, 60, Duration::from_secs(1));
        run.record(sample(80.0, 10.0), 60.0, 4.0, 0, None);
        let report = run.finish();
        // The spec forbids faking measurements: a GPU we couldn't read
        // must not be reported as 0%.
        assert!(report.mean_gpu_percent.is_none());
        assert!(report.mean_upload_kbps.is_none());
    }

    #[test]
    fn dropped_frames_are_counted_as_a_delta() {
        let mut run = BenchmarkRun::start(40.0, 60, Duration::from_secs(1));
        run.record(sample(80.0, 10.0), 60.0, 4.0, 1000, None);
        run.record(sample(80.0, 10.0), 60.0, 4.0, 1007, None);
        assert_eq!(run.finish().dropped_frames, 7);
    }
}
