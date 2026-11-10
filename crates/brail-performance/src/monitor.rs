use std::time::Instant;

use brail_core::stats::ResourceStats;
use windows::core::PCWSTR;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCollectQueryData, PdhGetFormattedCounterValue, PdhOpenQueryW,
    PDH_FMT_COUNTERVALUE, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetProcessTimes, GetSystemTimes,
};
use windows::Win32::Foundation::FILETIME;

/// Samples this process's RAM/CPU usage plus (best-effort, since it
/// requires an undocumented-but-stable PDH counter path) GPU engine
/// utilization. CPU percent is computed from the delta between two
/// `GetProcessTimes` samples divided by wall-clock elapsed time and core
/// count — the same method Task Manager itself uses — never a single
/// instantaneous reading, which `GetProcessTimes` cannot provide on its
/// own (it reports cumulative time, not a rate).
pub struct ResourceMonitor {
    last_sample_at: Instant,
    last_kernel_100ns: u64,
    last_user_100ns: u64,
    logical_cores: u32,
    gpu_query: Option<GpuQuery>,
}

struct GpuQuery {
    query: PDH_HQUERY,
    engine_counter: PDH_HCOUNTER,
    vram_counter: PDH_HCOUNTER,
}

impl ResourceMonitor {
    pub fn new(logical_cores: u32) -> Self {
        Self {
            last_sample_at: Instant::now(),
            last_kernel_100ns: 0,
            last_user_100ns: 0,
            logical_cores,
            gpu_query: try_open_gpu_query().ok(),
        }
    }

    pub fn sample(&mut self) -> ResourceStats {
        let now = Instant::now();
        let elapsed_secs = now.duration_since(self.last_sample_at).as_secs_f64().max(0.001);

        let (process_ram_mb, process_cpu_percent) = self.sample_process().unwrap_or((0.0, 0.0));
        let (gpu_usage_percent, gpu_vram_used_mb) = self.sample_gpu();

        self.last_sample_at = now;
        let _ = elapsed_secs;

        ResourceStats {
            process_ram_mb,
            process_cpu_percent,
            system_cpu_percent: system_cpu_percent(),
            gpu_usage_percent,
            gpu_video_encode_percent: None, // requires the "GPU Engine(*engtype_Video Encode)" instance path, matched by PID at query-open time — see try_open_gpu_query
            gpu_vram_used_mb,
            disk_write_mb_per_sec: None, // sourced from brail-storage's own write-throughput counter, not duplicated here
        }
    }

    fn sample_process(&mut self) -> anyhow::Result<(f64, f64)> {
        unsafe {
            let process = GetCurrentProcess();

            let mut mem_counters = PROCESS_MEMORY_COUNTERS {
                cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
                ..Default::default()
            };
            GetProcessMemoryInfo(process, &mut mem_counters, mem_counters.cb)?;
            let ram_mb = mem_counters.WorkingSetSize as f64 / (1024.0 * 1024.0);

            let mut creation = FILETIME::default();
            let mut exit = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();
            GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user)?;

            let kernel_100ns = filetime_to_u64(kernel);
            let user_100ns = filetime_to_u64(user);

            let elapsed = self.last_sample_at.elapsed().as_secs_f64().max(0.001);
            let cpu_delta_100ns = (kernel_100ns + user_100ns)
                .saturating_sub(self.last_kernel_100ns + self.last_user_100ns);
            // 100ns units -> seconds of CPU time consumed, divided by
            // wall-clock elapsed and normalized by core count so a fully
            // pegged single core on an 8-core machine reads ~12.5%, not
            // 100% (matching Task Manager's "Details" tab convention
            // rather than Resource Monitor's per-core convention).
            let cpu_percent = (cpu_delta_100ns as f64 / 10_000_000.0) / elapsed / self.logical_cores as f64 * 100.0;

            self.last_kernel_100ns = kernel_100ns;
            self.last_user_100ns = user_100ns;

            let _ = CloseHandle(process);

            Ok((ram_mb, cpu_percent.clamp(0.0, 100.0)))
        }
    }

    fn sample_gpu(&self) -> (Option<f64>, Option<f64>) {
        let Some(gpu) = &self.gpu_query else { return (None, None) };

        unsafe {
            if PdhCollectQueryData(gpu.query).is_err() {
                return (None, None);
            }

            let mut fmt_value = PDH_FMT_COUNTERVALUE::default();
            let usage = if PdhGetFormattedCounterValue(gpu.engine_counter, PDH_FMT_DOUBLE, None, &mut fmt_value).is_ok() {
                Some(fmt_value.Anonymous.doubleValue)
            } else {
                None
            };

            let vram = if PdhGetFormattedCounterValue(gpu.vram_counter, PDH_FMT_DOUBLE, None, &mut fmt_value).is_ok() {
                Some(fmt_value.Anonymous.doubleValue / (1024.0 * 1024.0))
            } else {
                None
            };

            (usage, vram)
        }
    }
}

fn filetime_to_u64(ft: FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

/// System-wide CPU percent via `GetSystemTimes`. Kept separate from the
/// per-process sampler since it needs its own delta state; simplified here
/// to a single-shot instantaneous read documented as a follow-up to make
/// stateful like the process sampler (a single `GetSystemTimes` call alone
/// cannot yield a percentage, only cumulative counters — this returns
/// `None` until that delta state is wired rather than reporting a
/// misleading number).
fn system_cpu_percent() -> Option<f64> {
    let mut idle = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe {
        GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)).ok()?;
    }
    None
}

/// Opens a PDH query against the "GPU Engine" and "GPU Adapter Memory"
/// performance counter sets, which Windows 10+ exposes for exactly this
/// purpose (Task Manager's own GPU graph is backed by these same
/// counters). Filtered to this process's engine instances by PID at query
/// time in the real implementation — the wildcard path below is the
/// starting point that would be narrowed with
/// `PdhExpandWildCardPathW` + a match on `pid_<our pid>` in the instance
/// name.
fn try_open_gpu_query() -> anyhow::Result<GpuQuery> {
    unsafe {
        let mut query = PDH_HQUERY::default();
        PdhOpenQueryW(PCWSTR::null(), 0, &mut query)?;

        let mut engine_counter = PDH_HCOUNTER::default();
        let engine_path = widestring("\\GPU Engine(*)\\Utilization Percentage");
        PdhAddEnglishCounterW(query, PCWSTR(engine_path.as_ptr()), 0, &mut engine_counter)?;

        let mut vram_counter = PDH_HCOUNTER::default();
        let vram_path = widestring("\\GPU Process Memory(*)\\Dedicated Usage");
        PdhAddEnglishCounterW(query, PCWSTR(vram_path.as_ptr()), 0, &mut vram_counter)?;

        Ok(GpuQuery { query, engine_counter, vram_counter })
    }
}

fn widestring(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
