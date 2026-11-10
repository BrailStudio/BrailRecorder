use std::sync::Arc;

use brail_core::error::{BrailError, BrailResult};
use brail_core::frame::VideoFrame;
use tokio::sync::broadcast;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Gdi::HMONITOR;

use crate::d3d::D3dContext;
use crate::wgc::{CaptureTarget, WgcSession};

/// Bounded broadcast channel capacity for the raw capture frame stream.
/// Every consumer (main encoder, instant-replay encoder, preview thumbnail)
/// subscribes independently; a slow subscriber drops its own frames
/// (`broadcast::error::RecvError::Lagged`) rather than backpressuring
/// capture, since capture must never stall regardless of what downstream
/// stages are doing.
const FRAME_CHANNEL_CAPACITY: usize = 8;

pub struct CaptureEngine {
    d3d: Arc<D3dContext>,
    frame_tx: broadcast::Sender<Arc<VideoFrame>>,
    active_session: Option<WgcSession>,
}

/// Cheap, cloneable handle for other subsystems (encoder, replay buffer,
/// preview) to subscribe to the live frame stream without owning the
/// capture engine itself.
#[derive(Clone)]
pub struct CaptureHandle {
    frame_tx: broadcast::Sender<Arc<VideoFrame>>,
}

impl CaptureHandle {
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<VideoFrame>> {
        self.frame_tx.subscribe()
    }
}

impl CaptureEngine {
    pub fn new() -> anyhow::Result<Self> {
        let d3d = Arc::new(D3dContext::new()?);
        let (frame_tx, _) = broadcast::channel(FRAME_CHANNEL_CAPACITY);
        Ok(Self {
            d3d,
            frame_tx,
            active_session: None,
        })
    }

    pub fn handle(&self) -> CaptureHandle {
        CaptureHandle {
            frame_tx: self.frame_tx.clone(),
        }
    }

    pub fn start_monitor_capture(
        &mut self,
        monitor: HMONITOR,
        capture_cursor: bool,
    ) -> BrailResult<()> {
        self.stop();

        let tx = self.frame_tx.clone();
        let session = WgcSession::start(
            self.d3d.clone(),
            CaptureTarget::Monitor(monitor),
            capture_cursor,
            move |frame| {
                // A broadcast send error here only means there are zero
                // subscribers yet (e.g. encoder pipeline still spinning
                // up) — not a capture failure, so it's dropped silently
                // rather than surfaced as an error event.
                let _ = tx.send(Arc::new(frame));
            },
        )
        .map_err(|e| BrailError::CaptureInitFailed(e.to_string()))?;

        self.active_session = Some(session);
        Ok(())
    }

    pub fn start_window_capture(&mut self, hwnd: HWND, capture_cursor: bool) -> BrailResult<()> {
        self.stop();

        let tx = self.frame_tx.clone();
        let session = WgcSession::start(
            self.d3d.clone(),
            CaptureTarget::Window(hwnd),
            capture_cursor,
            move |frame| {
                let _ = tx.send(Arc::new(frame));
            },
        )
        .map_err(|e| BrailError::CaptureInitFailed(e.to_string()))?;

        self.active_session = Some(session);
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(session) = self.active_session.take() {
            if let Err(e) = session.stop() {
                tracing::warn!("error stopping capture session: {e}");
            }
        }
    }

    pub fn is_capturing(&self) -> bool {
        self.active_session.is_some()
    }
}

impl Drop for CaptureEngine {
    fn drop(&mut self) {
        self.stop();
    }
}
