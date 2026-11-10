//! brail-audio: WASAPI capture, resampling, mixing, and AAC encoding.
//!
//! Desktop audio and microphone are captured as two independent WASAPI
//! streams (loopback capture on a render endpoint, normal capture on a
//! capture endpoint) because they run on different clocks — mixing them
//! correctly requires resampling each to a common rate/format first
//! (`resampler.rs`) and then summing sample-accurately (`mixer.rs`), rather
//! than assuming they're already aligned.

pub mod aac_encoder;
pub mod controls;
pub mod mixer;
pub mod resampler;
pub mod wasapi;

pub use controls::{apply_track_controls, AudioLevels};
pub use wasapi::WasapiCapture;
