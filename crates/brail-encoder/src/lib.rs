//! brail-encoder: turns raw captured frames into compressed packets.
//!
//! All encoder backends (NVENC/AMF/QSV/software) go through FFmpeg's
//! encoder API rather than three separate vendor SDKs (NVENC SDK, AMF SDK,
//! Intel Media SDK/oneVPL) directly. This costs a small amount of
//! flexibility (FFmpeg's wrapper doesn't expose every vendor-specific
//! tuning knob) in exchange for one integration surface, one dependency to
//! update, and one place to fix bugs — a defensible trade for a project
//! this size, and reversible later (`FfmpegEncoder` is the only thing that
//! would need replacing, since everything else in the pipeline only talks
//! to the `VideoEncoder` trait).

pub mod bitrate;
pub mod ffmpeg_encoder;
pub mod gpu_surface;
pub mod muxer;
pub mod pipeline;
pub mod traits;

pub use muxer::Muxer;
pub use pipeline::EncodeController;
pub use traits::VideoEncoder;
