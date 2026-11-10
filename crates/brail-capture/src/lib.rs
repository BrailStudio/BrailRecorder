//! brail-capture: Windows Graphics Capture based screen/window capture.
//!
//! Deliberately built on Windows.Graphics.Capture (WGC), not the older
//! DXGI Desktop Duplication API: WGC supports per-window capture,
//! HDR-aware capture, and — critically for the "cursor capture toggle"
//! requirement — lets us choose whether the cursor is composited into the
//! frame at the OS level (`IsCursorCaptureEnabled`) instead of drawing it
//! ourselves. Desktop Duplication remains as a documented fallback path
//! (see `duplication.rs`) for the pre-Windows-10-1903 systems the spec
//! explicitly says must degrade gracefully rather than fail outright.

pub mod compositor;
pub mod d3d;
pub mod screenshot;
pub mod sources;
pub mod wgc;
pub mod duplication;
pub mod cursor;
pub mod webcam;
pub mod engine;

pub use compositor::CaptureRegion;
pub use engine::{CaptureEngine, CaptureHandle};
pub use screenshot::ScreenshotCapture;
pub use sources::CaptureSource;
