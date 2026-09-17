use brail_core::error::BrailResult;
use brail_core::frame::{EncodedPacket, VideoFrame};

/// Common surface every encoder backend (hardware or software) implements.
/// The rest of the pipeline (recording muxer, RTMP packager, replay buffer)
/// only ever holds a `Box<dyn VideoEncoder>`, so switching from e.g. NVENC
/// to software mid-session (see `brail-encoder::pipeline`'s hardware-loss
/// fallback) doesn't require any downstream code to change.
pub trait VideoEncoder: Send {
    /// Submits a frame for encoding. Encoding is inherently pipelined
    /// (a hardware encoder buffers several frames internally for
    /// lookahead/B-frames), so this does not return a packet 1:1 — call
    /// `receive_packet` in a loop after each `submit_frame` until it
    /// returns `Ok(None)`.
    fn submit_frame(&mut self, frame: &VideoFrame) -> BrailResult<()>;

    /// Drains any packets the encoder is ready to emit. Returns `Ok(None)`
    /// when the encoder needs more input before it can produce output
    /// (normal during the initial lookahead-buffer fill).
    fn receive_packet(&mut self) -> BrailResult<Option<EncodedPacket>>;

    /// Flushes the encoder at end-of-stream, signaling no more frames are
    /// coming so buffered frames get emitted. Must be called exactly once
    /// before dropping the encoder, or the last ~N frames (N = lookahead
    /// depth) are silently lost from the output.
    fn flush(&mut self) -> BrailResult<Vec<EncodedPacket>>;

    /// Adjusts the target bitrate on a live encoder without a full
    /// reinitialize, used by the adaptive-bitrate streaming controller
    /// (`brail-encoder::bitrate`) reacting to upload-bandwidth drops.
    /// Backends that can't do this without reinit (rare, but true of a few
    /// older QSV configurations) return `Err(UnsupportedConfiguration)`
    /// and the caller falls back to a full re-open.
    fn set_bitrate(&mut self, bitrate_kbps: u32) -> BrailResult<()>;

    fn backend_name(&self) -> &'static str;
}
