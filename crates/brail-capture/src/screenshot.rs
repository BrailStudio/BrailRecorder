use std::path::{Path, PathBuf};

use brail_core::error::{BrailError, BrailResult};
use brail_core::settings::{ScreenshotFormat, ScreenshotSettings};
use windows::Win32::Graphics::Direct3D11::{
    ID3D11Texture2D, D3D11_CPU_ACCESS_READ, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};

use crate::d3d::D3dContext;

/// Captures a single frame and writes it to disk as PNG or JPEG (§22).
///
/// The GPU->CPU copy here is unavoidable — the pixels have to reach system
/// memory to be run through an image encoder — but it's a one-shot cost on
/// an explicit user action, not a per-frame cost, so it doesn't conflict
/// with the zero-copy requirement that governs the video pipeline. The
/// staging-texture path below is the efficient way to do it: one
/// `CopyResource` into a CPU-readable staging texture, one `Map`, no
/// intermediate render pass.
pub struct ScreenshotCapture;

impl ScreenshotCapture {
    /// Reads a captured D3D11 texture back to CPU memory as BGRA8 bytes.
    pub fn read_texture_to_bgra(
        d3d: &D3dContext,
        texture: &ID3D11Texture2D,
    ) -> BrailResult<(Vec<u8>, u32, u32)> {
        unsafe {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);

            // A staging texture is the only D3D11 usage that permits CPU
            // reads. It cannot be bound to the pipeline, which is exactly
            // what we want — this is pure readback, no rendering.
            let staging_desc = D3D11_TEXTURE2D_DESC {
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
                ..desc
            };

            let mut staging: Option<ID3D11Texture2D> = None;
            d3d.device
                .CreateTexture2D(&staging_desc, None, Some(&mut staging))
                .map_err(|e| BrailError::Internal(format!("staging texture creation failed: {e}")))?;
            let staging = staging
                .ok_or_else(|| BrailError::Internal("staging texture was null".into()))?;

            d3d.context.CopyResource(&staging, texture);

            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            d3d.context
                .Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(|e| BrailError::Internal(format!("texture map failed: {e}")))?;

            let width = desc.Width;
            let height = desc.Height;
            let row_pitch = mapped.RowPitch as usize;
            let tight_row = (width * 4) as usize;

            // RowPitch is frequently larger than width*4 due to GPU row
            // alignment, so rows are copied individually rather than as one
            // contiguous block — copying the whole mapped region would
            // include padding bytes and skew the image.
            let mut out = Vec::with_capacity(tight_row * height as usize);
            let src = mapped.pData as *const u8;
            for row in 0..height as usize {
                let row_start = src.add(row * row_pitch);
                out.extend_from_slice(std::slice::from_raw_parts(row_start, tight_row));
            }

            d3d.context.Unmap(&staging, 0);

            Ok((out, width, height))
        }
    }

    /// Encodes BGRA8 bytes to a file in the configured format.
    pub fn write_image(
        bgra: &[u8],
        width: u32,
        height: u32,
        settings: &ScreenshotSettings,
        output_path: &Path,
    ) -> BrailResult<PathBuf> {
        // BGRA -> RGBA: the `image` crate's encoders expect RGBA channel
        // order, while every Windows capture path produces BGRA.
        let mut rgba = Vec::with_capacity(bgra.len());
        for px in bgra.chunks_exact(4) {
            rgba.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }

        let buffer = image::RgbaImage::from_raw(width, height, rgba).ok_or_else(|| {
            BrailError::Internal("pixel buffer did not match the stated dimensions".into())
        })?;

        match settings.format {
            ScreenshotFormat::Png => {
                buffer.save_with_format(output_path, image::ImageFormat::Png)
            }
            ScreenshotFormat::Jpeg => {
                // JPEG has no alpha channel; dropping it explicitly avoids
                // the encoder silently producing a black or inverted image.
                let rgb = image::DynamicImage::ImageRgba8(buffer).to_rgb8();
                let mut file = std::fs::File::create(output_path).map_err(|e| {
                    BrailError::OutputFileError(output_path.display().to_string(), e.to_string())
                })?;
                let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(
                    &mut file,
                    settings.jpeg_quality,
                );
                encoder
                    .encode_image(&rgb)
                    .map(|_| ())
                    .map_err(|e| image::ImageError::IoError(std::io::Error::other(e.to_string())))
            }
        }
        .map_err(|e| BrailError::OutputFileError(output_path.display().to_string(), e.to_string()))?;

        Ok(output_path.to_path_buf())
    }
}

/// Captures one frame of the given monitor and writes it to `output_path`.
///
/// This spins up a short-lived WGC session rather than reusing the
/// recording pipeline's, because a screenshot must work whether or not a
/// recording is in progress, and because grabbing a single frame and
/// tearing down is cheaper than keeping a second session alive for an
/// action the user takes occasionally.
pub fn capture_monitor_to_file(
    monitor_handle_id: isize,
    settings: &ScreenshotSettings,
    output_path: &Path,
) -> BrailResult<PathBuf> {
    use std::sync::mpsc;
    use windows::Win32::Graphics::Gdi::HMONITOR;

    let d3d = std::sync::Arc::new(
        D3dContext::new().map_err(|e| BrailError::CaptureInitFailed(e.to_string()))?,
    );

    let (tx, rx) = mpsc::channel::<(Vec<u8>, u32, u32)>();
    let d3d_for_callback = d3d.clone();

    let session = crate::wgc::WgcSession::start(
        d3d.clone(),
        crate::wgc::CaptureTarget::Monitor(HMONITOR(monitor_handle_id as *mut _)),
        true, // screenshots include the cursor by default, matching the OS Print Screen behavior
        move |frame| {
            // Only the first frame is needed; later sends fail harmlessly
            // once the receiver is dropped.
            if let brail_core::frame::VideoFramePayload::GpuTexture { handle, .. } = &frame.payload {
                if let Some(texture) = handle.downcast_ref::<ID3D11Texture2D>() {
                    if let Ok(result) =
                        ScreenshotCapture::read_texture_to_bgra(&d3d_for_callback, texture)
                    {
                        let _ = tx.send(result);
                    }
                }
            }
        },
    )
    .map_err(|e| BrailError::CaptureInitFailed(e.to_string()))?;

    // WGC delivers frames on its own thread; a monitor with completely
    // static content can take a moment to produce one, so this waits
    // rather than assuming a frame is immediately available.
    let received = rx.recv_timeout(std::time::Duration::from_secs(3));
    let _ = session.stop();

    let (bgra, width, height) = received.map_err(|_| {
        BrailError::CaptureSourceLost
    })?;

    ScreenshotCapture::write_image(&bgra, width, height, settings, output_path)
}
