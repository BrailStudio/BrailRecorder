use windows::Win32::Foundation::HMONITOR;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::Dxgi::{
    IDXGIAdapter1, IDXGIOutput1, IDXGIOutputDuplication, DXGI_OUTDUPL_FRAME_INFO,
    DXGI_OUTPUT_DESC,
};

use crate::d3d::D3dContext;

/// Fallback capture path for Windows builds older than 10.0.19041 (10
/// 2004), where `Direct3D11CaptureFramePool::SetIsCursorCaptureEnabled` and
/// some WGC border-drawing controls don't exist. Desktop Duplication has
/// been present since Windows 8.1 and is the documented alternative.
///
/// Trade-offs vs WGC, surfaced to the user in Settings when this backend is
/// active (never silently): no per-window capture (monitor only), and the
/// cursor is *not* composited by the OS — this backend must draw it itself
/// (see `brail-capture::cursor::composite_cursor_onto_texture`), which adds
/// a small per-frame GPU cost WGC's built-in path avoids.
pub struct DuplicationSession {
    duplication: IDXGIOutputDuplication,
}

impl DuplicationSession {
    pub fn new(d3d: &D3dContext, monitor: HMONITOR) -> anyhow::Result<Self> {
        let output = find_output_for_monitor(d3d, monitor)?;
        let duplication = unsafe { output.DuplicateOutput(&d3d.device)? };
        Ok(Self { duplication })
    }

    /// Blocking acquire with a timeout, matching `IDXGIOutputDuplication`'s
    /// real semantics: it returns `DXGI_ERROR_WAIT_TIMEOUT` (mapped to
    /// `Ok(None)` here) when no new frame arrived within the window, which
    /// is the normal case at anything above the display's refresh rate.
    pub fn acquire_next_frame(
        &self,
        timeout_ms: u32,
    ) -> anyhow::Result<Option<(ID3D11Texture2D, DXGI_OUTDUPL_FRAME_INFO)>> {
        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource = None;

        let result = unsafe {
            self.duplication
                .AcquireNextFrame(timeout_ms, &mut frame_info, &mut resource)
        };

        match result {
            Ok(()) => {
                let resource = resource.ok_or_else(|| anyhow::anyhow!("no resource returned"))?;
                let texture: ID3D11Texture2D = windows::core::Interface::cast(&resource)?;
                Ok(Some((texture, frame_info)))
            }
            Err(e) if e.code() == windows::Win32::Graphics::Dxgi::DXGI_ERROR_WAIT_TIMEOUT => {
                Ok(None)
            }
            Err(e) => Err(e.into()),
        }
    }

    pub fn release_frame(&self) -> anyhow::Result<()> {
        unsafe { self.duplication.ReleaseFrame()? };
        Ok(())
    }
}

fn find_output_for_monitor(d3d: &D3dContext, monitor: HMONITOR) -> anyhow::Result<IDXGIOutput1> {
    let dxgi_device: windows::Win32::Graphics::Dxgi::IDXGIDevice =
        windows::core::Interface::cast(&d3d.device)?;
    let adapter: IDXGIAdapter1 = unsafe { dxgi_device.GetAdapter()?.cast()? };

    let mut i = 0u32;
    loop {
        let output = unsafe { adapter.EnumOutputs(i) };
        let output = match output {
            Ok(o) => o,
            Err(_) => anyhow::bail!("monitor not found on this adapter's outputs"),
        };
        i += 1;

        let desc: DXGI_OUTPUT_DESC = unsafe {
            let mut d = DXGI_OUTPUT_DESC::default();
            output.GetDesc(&mut d)?;
            d
        };

        if desc.Monitor == monitor {
            return Ok(output.cast()?);
        }
    }
}
