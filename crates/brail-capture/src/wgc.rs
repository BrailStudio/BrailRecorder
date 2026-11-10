use std::sync::Arc;

use windows::core::Interface;
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

use brail_core::frame::{PixelFormat, VideoFrame, VideoFramePayload};

use crate::d3d::{texture_from_surface, D3dContext};

/// Which real OS object a `GraphicsCaptureItem` was created from. WGC
/// itself only exposes the resulting item, but the app needs to remember
/// this for "capture source lost" handling (a captured window closing
/// fires `Closed`; a captured monitor disconnecting does not, and is
/// instead detected via `brail-hardware`'s monitor-change notifications).
pub enum CaptureTarget {
    Monitor(HMONITOR),
    Window(HWND),
}

/// A running Windows Graphics Capture session. Frames are pushed to
/// `on_frame` from WGC's own frame-arrived thread (free-threaded frame
/// pool), so `on_frame` must be cheap — it should only forward the frame
/// into a channel, never block on encoding.
pub struct WgcSession {
    _item: GraphicsCaptureItem,
    frame_pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
}

impl WgcSession {
    pub fn start(
        d3d: Arc<D3dContext>,
        target: CaptureTarget,
        capture_cursor: bool,
        mut on_frame: impl FnMut(VideoFrame) + Send + 'static,
    ) -> anyhow::Result<Self> {
        let item = create_capture_item(&target)?;
        let size = item.Size()?;

        let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &d3d.winrt_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2, // double-buffered; WGC recommends >=1, 2 avoids tearing stalls
            size,
        )?;

        let session = frame_pool.CreateCaptureSession(&item)?;

        // Requires Windows 10 2004+ (build 19041); on older builds this
        // property isn't present and the cursor is always composited by
        // the OS into the frame with no way to hide it — brail-hardware's
        // `windows_build` field is what the UI checks before offering this
        // toggle at all, per the spec's graceful-degradation requirement.
        if let Err(e) = session.SetIsCursorCaptureEnabled(capture_cursor) {
            tracing::warn!("cursor capture toggle unsupported on this Windows build: {e}");
        }

        let d3d_for_callback = d3d.clone();
        let frame_index = std::sync::atomic::AtomicU64::new(0);

        frame_pool.FrameArrived(&TypedEventHandler::new(
            move |pool: &Option<Direct3D11CaptureFramePool>, _| {
                let Some(pool) = pool else { return Ok(()) };
                let frame = pool.TryGetNextFrame()?;

                let surface = frame.Surface()?;
                let timestamp = frame.SystemRelativeTime()?.Duration;

                match texture_from_surface(&surface) {
                    Ok(texture) => {
                        let idx = frame_index.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let content_size = frame.ContentSize().unwrap_or(size);

                        // The texture is only valid GPU-side; wrapping it in
                        // `Arc<dyn Any>` keeps it alive for as long as any
                        // downstream stage (encoder, replay buffer) holds a
                        // reference, without brail-core needing to know
                        // anything about D3D11 types.
                        let video_frame = VideoFrame {
                            payload: VideoFramePayload::GpuTexture {
                                handle: Arc::new(texture),
                                format: PixelFormat::Bgra8,
                            },
                            width: content_size.Width as u32,
                            height: content_size.Height as u32,
                            timestamp_100ns: timestamp,
                            frame_index: idx,
                        };

                        on_frame(video_frame);
                    }
                    Err(e) => {
                        tracing::warn!("failed to extract D3D11 texture from capture frame: {e}");
                    }
                }

                let _ = d3d_for_callback; // keep device alive for callback lifetime
                Ok(())
            },
        ))?;

        session.StartCapture()?;

        Ok(Self {
            _item: item,
            frame_pool,
            session,
        })
    }

    /// Handles a resize (monitor resolution change, or window resize when
    /// capturing a window) by recreating the frame pool at the new size —
    /// WGC requires this explicitly, it does not auto-resize the pool.
    pub fn handle_size_change(&self, new_size: windows::Graphics::SizeInt32) -> anyhow::Result<()> {
        self.frame_pool.Recreate(
            None::<&windows::Graphics::DirectX::Direct3D11::IDirect3DDevice>,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            2,
            new_size,
        )?;
        Ok(())
    }

    pub fn stop(&self) -> anyhow::Result<()> {
        self.session.Close()?;
        self.frame_pool.Close()?;
        Ok(())
    }
}

fn create_capture_item(target: &CaptureTarget) -> anyhow::Result<GraphicsCaptureItem> {
    // GraphicsCaptureItem has no public constructor from a raw Win32
    // window/monitor handle — the bridge is the WinRT/Win32 interop
    // factory `IGraphicsCaptureItemInterop`, obtained by activating the
    // GraphicsCaptureItem runtime class as a factory.
    let interop: IGraphicsCaptureItemInterop =
        windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()?;

    let item = match target {
        CaptureTarget::Monitor(hmonitor) => unsafe { interop.CreateForMonitor(*hmonitor)? },
        CaptureTarget::Window(hwnd) => unsafe { interop.CreateForWindow(*hwnd)? },
    };

    Ok(item)
}
