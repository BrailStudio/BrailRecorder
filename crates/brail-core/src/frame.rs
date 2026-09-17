use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Pixel format a captured/encoded video frame can be in. Kept intentionally
/// small: capture emits BGRA8 (native WGC/DXGI surface format) or NV12
/// (what most hardware encoders want directly, avoiding a conversion).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PixelFormat {
    Bgra8,
    Nv12,
}

/// A video frame moving through the pipeline.
///
/// `GpuTexture` wraps an opaque handle (a `*mut ID3D11Texture2D` pointer
/// value + the D3D device/context that own it, held behind the platform
/// layer) so frames can be handed from capture -> encoder without a
/// CPU round-trip. `CpuBuffer` is the fallback path used by the software
/// encoder and by any capture backend (e.g. Desktop Duplication on some
/// driver configurations) that cannot hand out a shareable GPU handle.
///
/// Real texture lifetime/ownership (AddRef/Release, keyed mutex
/// acquire/release for cross-thread access) is implemented in
/// `brail-capture::d3d` and `brail-encoder::gpu_surface`; this type is the
/// cross-crate contract, not the implementation.
pub enum VideoFramePayload {
    GpuTexture {
        /// Opaque handle to the shared D3D11 texture (see brail-capture::d3d::SharedTexture).
        handle: Arc<dyn std::any::Any + Send + Sync>,
        format: PixelFormat,
    },
    CpuBuffer {
        data: bytes::Bytes,
        format: PixelFormat,
        stride: u32,
    },
}

pub struct VideoFrame {
    pub payload: VideoFramePayload,
    pub width: u32,
    pub height: u32,
    /// Monotonic capture timestamp in 100ns units (matches Win32 QPC-derived
    /// timestamps from Windows.Graphics.Capture's FrameArrived event), used
    /// for PTS assignment and A/V sync — never wall-clock time, which drifts.
    pub timestamp_100ns: i64,
    pub frame_index: u64,
}

impl std::fmt::Debug for VideoFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VideoFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("timestamp_100ns", &self.timestamp_100ns)
            .field("frame_index", &self.frame_index)
            .finish()
    }
}

/// Interleaved PCM audio, captured at the device's native format and
/// resampled once (not per-consumer) before fan-out to recording/streaming.
#[derive(Clone)]
pub struct AudioFrame {
    pub samples: bytes::Bytes,
    pub sample_rate: u32,
    pub channels: u16,
    pub timestamp_100ns: i64,
    pub source: AudioSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioSource {
    DesktopLoopback,
    Microphone,
    /// Already mixed by brail-audio::mixer.
    Mixed,
}

/// A compressed access unit coming out of an encoder, ready to be muxed
/// (recording) and/or packaged into FLV tags (streaming). Kept format
/// agnostic (H.264/HEVC/AV1 all produce this same shape) so the muxer and
/// RTMP packager don't need per-codec branches beyond reading `codec`.
#[derive(Clone)]
pub struct EncodedPacket {
    pub data: bytes::Bytes,
    pub pts_100ns: i64,
    pub dts_100ns: i64,
    pub is_keyframe: bool,
    pub codec: super::config::VideoCodec,
    pub stream: StreamKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKind {
    Video,
    Audio,
}
