use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::Gdi::{DeleteDC, DeleteObject, SelectObject};
use windows::Win32::UI::WindowsAndMessaging::{
    DrawIconEx, GetCursorInfo, GetIconInfo, CURSORINFO, CURSOR_SHOWING, DI_NORMAL,
};

/// Composites the real system cursor onto a captured frame. Only needed on
/// the Desktop Duplication fallback path (`duplication.rs`) — Windows
/// Graphics Capture composites the cursor itself when
/// `IsCursorCaptureEnabled` is set, so `wgc.rs` never calls this.
///
/// Uses the GDI interop surface on a texture created with
/// `D3D11_RESOURCE_MISC_GDI_COMPATIBLE` (see `brail-capture::d3d` staging
/// texture setup) because that's the only supported way to mix GDI drawing
/// (`DrawIconEx`) with a D3D11 texture without a full custom Direct2D
/// cursor-shape rasterizer.
pub fn composite_cursor_onto_texture(
    texture: &ID3D11Texture2D,
    capture_origin: POINT,
) -> anyhow::Result<()> {
    let mut info = CURSORINFO {
        cbSize: std::mem::size_of::<CURSORINFO>() as u32,
        ..Default::default()
    };

    unsafe { GetCursorInfo(&mut info)? };

    if info.flags != CURSOR_SHOWING {
        return Ok(()); // cursor hidden (e.g. app in fullscreen-exclusive with hidden cursor)
    }

    let dxgi_surface: windows::Win32::Graphics::Dxgi::IDXGISurface1 =
        windows::core::Interface::cast(texture)?;

    unsafe {
        let hdc = dxgi_surface.GetDC(false)?;

        let x = info.ptScreenPos.x - capture_origin.x;
        let y = info.ptScreenPos.y - capture_origin.y;

        let mut icon_info = Default::default();
        if GetIconInfo(info.hCursor, &mut icon_info).is_ok() {
            DrawIconEx(hdc, x, y, info.hCursor, 0, 0, 0, None, DI_NORMAL)?;
            if !icon_info.hbmMask.is_invalid() {
                let _ = DeleteObject(icon_info.hbmMask);
            }
            if !icon_info.hbmColor.is_invalid() {
                let _ = DeleteObject(icon_info.hbmColor);
            }
        }

        dxgi_surface.ReleaseDC(None)?;
        let _ = DeleteDC(hdc);
        let _ = SelectObject; // keep import path documented for future brush-based highlight ring
    }

    Ok(())
}

/// Draws the optional "highlight clicks" ring (a translucent circle at the
/// cursor position when a mouse button is down), toggled by
/// `RecordingSettings`-adjacent UI option `highlight_cursor`. Implemented
/// as a separate pass from cursor compositing since it needs button state
/// (`GetAsyncKeyState`) rather than just cursor shape.
pub fn should_draw_click_highlight() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_LBUTTON, VK_RBUTTON};
    unsafe {
        (GetAsyncKeyState(VK_LBUTTON.0 as i32) as u16 & 0x8000) != 0
            || (GetAsyncKeyState(VK_RBUTTON.0 as i32) as u16 & 0x8000) != 0
    }
}
