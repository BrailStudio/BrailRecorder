use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW, IsWindowVisible,
    GWL_EXSTYLE, WS_EX_TOOLWINDOW,
};

#[derive(Debug, Clone)]
pub enum CaptureSource {
    Monitor { handle_id: isize, friendly_name: String },
    Window { hwnd_id: isize, title: String },
}

/// Lists real top-level, visible, titled windows — the same population
/// `GraphicsCaptureItem`'s window picker would show. Tool windows
/// (`WS_EX_TOOLWINDOW`, e.g. floating palettes) and untitled windows are
/// excluded since WGC can technically capture them but the result is never
/// what a user means by "capture this window."
pub fn enumerate_capturable_windows() -> anyhow::Result<Vec<CaptureSource>> {
    let mut windows: Vec<CaptureSource> = Vec::new();

    unsafe {
        let userdata = &mut windows as *mut Vec<CaptureSource> as isize;
        EnumWindows(Some(enum_windows_proc), LPARAM(userdata))?;
    }

    Ok(windows)
}

unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let windows = &mut *(lparam.0 as *mut Vec<CaptureSource>);

    if !IsWindowVisible(hwnd).as_bool() {
        return BOOL(1);
    }

    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
        return BOOL(1);
    }

    let len = GetWindowTextLengthW(hwnd);
    if len == 0 {
        return BOOL(1);
    }

    let mut buf = vec![0u16; (len + 1) as usize];
    let copied = GetWindowTextW(hwnd, &mut buf);
    if copied == 0 {
        return BOOL(1);
    }
    let title = String::from_utf16_lossy(&buf[..copied as usize]);

    windows.push(CaptureSource::Window {
        hwnd_id: hwnd.0 as isize,
        title,
    });

    BOOL(1)
}
