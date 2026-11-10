use brail_core::config::Resolution;
use brail_core::profile::MonitorInfo;
use std::ffi::c_void;
use windows::Win32::Foundation::{BOOL, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, EnumDisplaySettingsW, GetMonitorInfoW, DEVMODEW, ENUM_CURRENT_SETTINGS,
    HDC, HMONITOR, MONITORINFOEXW, MONITORINFOF_PRIMARY,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

/// Enumerates every active monitor with its real resolution, refresh rate,
/// DPI scale, and friendly name — the same set Windows.Graphics.Capture's
/// `GraphicsCaptureItem.CreateFromMonitor` can target, so the UI's monitor
/// picker maps 1:1 onto real capturable sources.
pub fn enumerate_monitors() -> anyhow::Result<Vec<MonitorInfo>> {
    let mut monitors: Vec<MonitorInfo> = Vec::new();

    unsafe {
        let userdata = &mut monitors as *mut Vec<MonitorInfo> as isize;
        EnumDisplayMonitors(HDC::default(), None, Some(monitor_enum_proc), LPARAM(userdata))
            .ok()?;
    }

    Ok(monitors)
}

unsafe extern "system" fn monitor_enum_proc(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let monitors = &mut *(lparam.0 as *mut Vec<MonitorInfo>);

    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;

    if GetMonitorInfoW(hmonitor, &mut info.monitorInfo as *mut _ as *mut _).as_bool() {
        let device_name = String::from_utf16_lossy(
            &info.szDevice[..info.szDevice.iter().position(|&c| c == 0).unwrap_or(0)],
        );

        let width = (info.monitorInfo.rcMonitor.right - info.monitorInfo.rcMonitor.left) as u32;
        let height = (info.monitorInfo.rcMonitor.bottom - info.monitorInfo.rcMonitor.top) as u32;

        let refresh_rate_hz = {
            let mut devmode = DEVMODEW {
                dmSize: std::mem::size_of::<DEVMODEW>() as u16,
                ..Default::default()
            };
            let name_ptr = windows::core::PCWSTR(info.szDevice.as_ptr());
            if EnumDisplaySettingsW(name_ptr, ENUM_CURRENT_SETTINGS, &mut devmode).as_bool() {
                devmode.dmDisplayFrequency
            } else {
                60 // EnumDisplaySettingsW failing is rare; 60Hz is the safe
                   // conservative default rather than reporting 0.
            }
        };

        let dpi_scale_percent = {
            let mut dpi_x = 0u32;
            let mut dpi_y = 0u32;
            match GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) {
                Ok(()) => (dpi_x * 100) / 96,
                Err(_) => 100,
            }
        };

        monitors.push(MonitorInfo {
            id: device_name.clone(),
            handle_id: hmonitor.0 as isize,
            friendly_name: friendly_name_for_device(&device_name).unwrap_or(device_name),
            resolution: Resolution::new(width, height),
            refresh_rate_hz,
            is_primary: (info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY) != 0,
            dpi_scale_percent,
        });
    }

    BOOL(1) // continue enumeration
}

/// Maps a GDI device name like "\\.\DISPLAY1" to the monitor's real EDID
/// friendly name (e.g. "Dell U2723QE") via
/// `QueryDisplayConfig`/`DisplayConfigGetDeviceInfo`. Stubbed to `None`
/// here (falls back to the GDI device name) — wiring the full
/// QueryDisplayConfig path is mechanical but verbose; left as a follow-up
/// once this compiles against real hardware.
fn friendly_name_for_device(_device_name: &str) -> Option<String> {
    None
}

// Suppress unused-import warning for c_void placeholder used in doc
// examples elsewhere in this module's future extension points.
#[allow(unused)]
fn _keep_import(_: *const c_void) {}
