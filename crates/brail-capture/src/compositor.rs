use brail_core::config::Resolution;
use brail_core::settings::{OverlayAnchor, OverlayItem, WebcamOverlaySettings};
use windows::Win32::Graphics::Direct3D11::{ID3D11Texture2D, D3D11_BOX};

use crate::d3d::D3dContext;

/// A sub-rectangle of a capture source, for region capture (§4). Stored in
/// source pixels, validated against the source dimensions before use so a
/// stale region (e.g. saved against a since-changed monitor resolution)
/// can't produce an out-of-bounds GPU copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CaptureRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

impl CaptureRegion {
    /// Clamps the region to fit inside the given source size. Returns
    /// `None` if the region falls entirely outside the source, which is the
    /// signal for the caller to fall back to full-source capture and warn
    /// the user rather than producing an empty frame.
    pub fn clamped_to(&self, source: Resolution) -> Option<CaptureRegion> {
        if self.x >= source.width || self.y >= source.height {
            return None;
        }
        let width = self.width.min(source.width - self.x);
        let height = self.height.min(source.height - self.y);
        if width == 0 || height == 0 {
            return None;
        }
        // Hardware encoders require even dimensions for NV12 chroma
        // subsampling; rounding down here avoids an encoder-side rejection
        // later with a much less obvious error message.
        Some(CaptureRegion {
            x: self.x,
            y: self.y,
            width: width & !1,
            height: height & !1,
        })
    }
}

/// Crops a captured texture to a region entirely on the GPU via
/// `CopySubresourceRegion` — no CPU readback, no shader pass, which keeps
/// region capture as cheap as full-screen capture.
pub fn crop_texture(
    d3d: &D3dContext,
    source: &ID3D11Texture2D,
    region: CaptureRegion,
    dest: &ID3D11Texture2D,
) {
    let box_ = D3D11_BOX {
        left: region.x,
        top: region.y,
        front: 0,
        right: region.x + region.width,
        bottom: region.y + region.height,
        back: 1,
    };

    unsafe {
        d3d.context
            .CopySubresourceRegion(dest, 0, 0, 0, 0, source, 0, Some(&box_));
    }
}

/// Resolves an overlay's anchor + percentage sizing into absolute output
/// pixels. Kept as pure arithmetic separate from any GPU work so it can be
/// unit tested without a D3D device — see `tests/overlay_layout.rs`.
pub fn resolve_overlay_rect(
    anchor: OverlayAnchor,
    output: Resolution,
    width_percent: f32,
    height_percent: f32,
    margin_percent: f32,
) -> (i32, i32, u32, u32) {
    let w = ((output.width as f32) * (width_percent / 100.0)).round().max(1.0) as u32;
    let h = ((output.height as f32) * (height_percent / 100.0)).round().max(1.0) as u32;
    let margin_x = ((output.width as f32) * (margin_percent / 100.0)).round() as i32;
    let margin_y = ((output.height as f32) * (margin_percent / 100.0)).round() as i32;

    let (x, y) = match anchor {
        OverlayAnchor::TopLeft => (margin_x, margin_y),
        OverlayAnchor::TopRight => (output.width as i32 - w as i32 - margin_x, margin_y),
        OverlayAnchor::BottomLeft => (margin_x, output.height as i32 - h as i32 - margin_y),
        OverlayAnchor::BottomRight => (
            output.width as i32 - w as i32 - margin_x,
            output.height as i32 - h as i32 - margin_y,
        ),
    };

    (x, y, w, h)
}

/// Computes the webcam overlay's destination rectangle, preserving the
/// webcam's source aspect ratio after crop so the picture-in-picture never
/// looks stretched (the user sets width; height follows from the aspect).
pub fn resolve_webcam_rect(
    settings: &WebcamOverlaySettings,
    output: Resolution,
    webcam_source: Resolution,
) -> (i32, i32, u32, u32) {
    let [cl, ct, cr, cb] = settings.crop;
    let cropped_w = (webcam_source.width as f32 * (1.0 - cl - cr)).max(1.0);
    let cropped_h = (webcam_source.height as f32 * (1.0 - ct - cb)).max(1.0);
    let aspect = cropped_w / cropped_h;

    let w = ((output.width as f32) * (settings.width_percent / 100.0)).round().max(1.0) as u32;
    let h = ((w as f32) / aspect).round().max(1.0) as u32;

    let margin_x = ((output.width as f32) * (settings.margin_percent / 100.0)).round() as i32;
    let margin_y = ((output.height as f32) * (settings.margin_percent / 100.0)).round() as i32;

    let (x, y) = match settings.anchor {
        OverlayAnchor::TopLeft => (margin_x, margin_y),
        OverlayAnchor::TopRight => (output.width as i32 - w as i32 - margin_x, margin_y),
        OverlayAnchor::BottomLeft => (margin_x, output.height as i32 - h as i32 - margin_y),
        OverlayAnchor::BottomRight => (
            output.width as i32 - w as i32 - margin_x,
            output.height as i32 - h as i32 - margin_y,
        ),
    };

    (x, y, w & !1, h & !1)
}

/// Whether any compositing work is needed at all this frame. When this
/// returns false the capture texture goes straight to the encoder
/// untouched — the zero-copy fast path. The spec's low-end and Gaming Mode
/// requirements both depend on this check short-circuiting cleanly when
/// the user has no overlays configured, which is the common case.
pub fn needs_compositing(webcam: &WebcamOverlaySettings, overlays: &[OverlayItem]) -> bool {
    webcam.enabled || !overlays.is_empty()
}
