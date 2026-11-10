use windows::Win32::System::SystemInformation::{
    GetLogicalProcessorInformation, GetSystemInfo, GlobalMemoryStatusEx, MEMORYSTATUSEX,
    SYSTEM_INFO, RelationProcessorCore,
};

pub struct CpuInfo {
    pub name: String,
    pub physical_cores: u32,
    pub logical_cores: u32,
    pub total_ram_mb: u64,
}

/// Detects CPU brand string, physical/logical core counts, and total
/// physical RAM using real Win32 calls — no `/proc/cpuinfo` shortcuts
/// (this is Windows-only) and no hardcoded numbers.
pub fn detect_cpu() -> anyhow::Result<CpuInfo> {
    let name = cpu_brand_string();

    let logical_cores = unsafe {
        let mut info = SYSTEM_INFO::default();
        GetSystemInfo(&mut info);
        info.dwNumberOfProcessors
    };

    let physical_cores = physical_core_count().unwrap_or(logical_cores);

    let total_ram_mb = unsafe {
        let mut status = MEMORYSTATUSEX {
            dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
            ..Default::default()
        };
        GlobalMemoryStatusEx(&mut status)?;
        status.ullTotalPhys / (1024 * 1024)
    };

    Ok(CpuInfo {
        name,
        physical_cores,
        logical_cores,
        total_ram_mb,
    })
}

/// Physical core count via GetLogicalProcessorInformation, counting
/// distinct RelationProcessorCore entries. Falls back to logical count
/// (i.e. assumes no SMT) if the call fails on unusual hardware.
fn physical_core_count() -> Option<u32> {
    unsafe {
        let mut needed: u32 = 0;
        // First call intentionally fails with ERROR_INSUFFICIENT_BUFFER to
        // discover the required buffer size.
        let _ = GetLogicalProcessorInformation(None, &mut needed);
        if needed == 0 {
            return None;
        }

        let count = needed as usize
            / std::mem::size_of::<windows::Win32::System::SystemInformation::SYSTEM_LOGICAL_PROCESSOR_INFORMATION>();
        let mut buffer = vec![
            windows::Win32::System::SystemInformation::SYSTEM_LOGICAL_PROCESSOR_INFORMATION::default();
            count
        ];

        GetLogicalProcessorInformation(Some(buffer.as_mut_ptr()), &mut needed).ok()?;

        let physical = buffer
            .iter()
            .filter(|e| e.Relationship == RelationProcessorCore)
            .count() as u32;

        if physical == 0 {
            None
        } else {
            Some(physical)
        }
    }
}

/// CPU brand string via the `cpuid` instruction (leaves 0x80000002-4), the
/// same mechanism Windows itself uses to populate the registry's
/// ProcessorNameString value — done directly here so detection doesn't
/// depend on registry access/permissions.
fn cpu_brand_string() -> String {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::__cpuid;
        unsafe {
            let mut bytes = Vec::with_capacity(48);
            for leaf in 0x80000002u32..=0x80000004u32 {
                let r = __cpuid(leaf);
                for reg in [r.eax, r.ebx, r.ecx, r.edx] {
                    bytes.extend_from_slice(&reg.to_le_bytes());
                }
            }
            String::from_utf8_lossy(&bytes)
                .trim_matches(char::from(0))
                .trim()
                .to_string()
        }
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        "Unknown CPU (non-x86_64 target)".to_string()
    }
}
