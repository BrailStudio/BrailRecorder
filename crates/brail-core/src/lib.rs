//! brail-core: shared types used by every other Brail Recorder crate.
//!
//! Nothing in here touches the OS, GPU, or network. It exists so that
//! capture/encoder/streaming/replay can all agree on the shape of a frame,
//! a pipeline event, and the user's configuration without depending on each
//! other directly. That keeps the dependency graph a star, not a mesh, which
//! is what lets the UI process link against just the thin `brail-core`
//! config types instead of pulling in Direct3D/FFmpeg/RTMP.

pub mod config;
pub mod error;
pub mod events;
pub mod frame;
pub mod presets;
pub mod profile;
pub mod settings;
pub mod stats;

pub use config::*;
pub use error::*;
pub use events::*;
pub use frame::*;
pub use presets::*;
pub use profile::*;
pub use settings::*;
pub use stats::*;
