//! brail-streaming: RTMP(S)/SRT publish with reconnect and adaptive quality.
//!
//! One `StreamSession` per active outbound stream (the spec allows a
//! future multi-destination mode; this crate's types are already
//! per-session so that's additive, not a rewrite). Reconnection,
//! bitrate degradation, and stats reporting all live in `session.rs` so
//! the protocol-specific clients (`rtmp_client.rs`, `srt_client.rs`) stay
//! narrowly focused on wire-format correctness.

pub mod flv_muxer;
pub mod rtmp_client;
pub mod session;
pub mod srt_client;
pub mod test_connection;

pub use session::StreamSession;
pub use test_connection::{test_connection, TestResult};
