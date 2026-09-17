//! brail-replay: continuous instant-replay ring buffer.
//!
//! Runs its own independent encoder instance (separate from the main
//! recording/streaming encoders, per `InstantReplaySettings` having its
//! own resolution/fps/encoder settings — usually a smaller, cheaper
//! configuration than the main recording, since it must run continuously
//! in the background at near-zero perceived cost). Encoded packets are
//! kept in a bounded, time-windowed ring buffer; "save replay" drains the
//! buffer to a file without interrupting the buffer's continued operation.

pub mod ring_buffer;
pub mod replay_engine;

pub use replay_engine::ReplayEngine;
