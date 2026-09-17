use brail_core::profile::{GpuInfo, GpuVendor};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};

const VENDOR_NVIDIA: u32 = 0x10DE;
const VENDOR_AMD: u32 = 0x1002;
const VENDOR_AMD_ALT: u32 = 0x1022; // some older AMD/ATI parts report this
const VENDOR_INTEL: u32 = 0x8086;

/// Enumerates every DXGI adapter (real GPUs, not a hardcoded list) and
/// reports vendor + dedicated VRAM straight from `DXGI_ADAPTER_DESC1`. The
/// adapter Windows Graphics Capture will actually use is resolved
/// separately in `brail-capture::d3d` once a specific monitor/window is
/// selected (relevant on hybrid iGPU/dGPU laptops), so `is_capture_adapter`
/// here is a best-effort default (first non-software adapter with the most
/// VRAM), not a guarantee.
pub fn enumerate_gpus() -> anyhow::Result<Vec<GpuInfo>> {
    let mut gpus = Vec::new();

    unsafe {
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        let mut index = 0u32;
        loop {
            let adapter = match factory.EnumAdapters1(index) {
                Ok(a) => a,
                Err(_) => break, // DXGI_ERROR_NOT_FOUND — enumerated all adapters
            };
            index += 1;

            let desc = adapter.GetDesc1()?;

            // Skip the Microsoft Basic Render Driver (software rasterizer,
            // vendor id 0x1414) — it can't do hardware capture or encode.
            if desc.VendorId == 0x1414 {
                continue;
            }

            let name = String::from_utf16_lossy(
                &desc.Description[..desc
                    .Description
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(desc.Description.len())],
            );

            let vendor = match desc.VendorId {
                VENDOR_NVIDIA => GpuVendor::Nvidia,
                VENDOR_AMD | VENDOR_AMD_ALT => GpuVendor::Amd,
                VENDOR_INTEL => GpuVendor::Intel,
                _ => GpuVendor::Other,
            };

            gpus.push(GpuInfo {
                name,
                vendor,
                dedicated_vram_mb: (desc.DedicatedVideoMemory as u64) / (1024 * 1024),
                driver_version: driver_version_string(&adapter),
                is_capture_adapter: false, // resolved below
            });
        }
    }

    // Mark the adapter with the most dedicated VRAM as the default capture
    // adapter. This is a heuristic default surfaced in the UI's hardware
    // panel; actual capture always re-resolves the adapter that owns the
    // selected monitor at capture-start time.
    if let Some(best) = gpus
        .iter_mut()
        .max_by_key(|g| g.dedicated_vram_mb)
    {
        best.is_capture_adapter = true;
    }

    Ok(gpus)
}

fn driver_version_string(adapter: &windows::Win32::Graphics::Dxgi::IDXGIAdapter1) -> String {
    // DXGI doesn't expose a friendly driver version directly; the accurate
    // value (UMD driver version, e.g. "31.0.15.3623") comes from
    // IDXGIAdapter::CheckInterfaceSupport / the registry driver store in
    // production. Kept as a best-effort placeholder here so the profile
    // still reports *something* rather than silently omitting the field —
    // wire this to CheckInterfaceSupport(D3D11) + registry lookup during
    // the Windows build pass.
    let _ = adapter;
    "unknown (resolve via driver store on target machine)".to_string()
}
