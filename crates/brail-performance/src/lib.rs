//! brail-performance: live resource monitoring + automatic self-tuning.
//!
//! Sampling runs on its own low-frequency timer (1 Hz) independent of the
//! capture/encode/stream pipelines, so measuring performance never itself
//! becomes a performance problem. `self_tune.rs` consumes these samples to
//! implement the spec's "automatically reduce quality under resource
//! pressure rather than degrade the whole system" requirement.

pub mod adaptive;
pub mod benchmark;
pub mod monitor;
pub mod self_tune;

pub use adaptive::{AdaptiveAction, AdaptiveEngine, AdaptiveSample, Recommendation, Severity};
pub use benchmark::{BenchmarkReport, BenchmarkRun};
pub use monitor::ResourceMonitor;
