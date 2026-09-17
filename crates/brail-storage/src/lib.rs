//! brail-storage: config persistence, output path management, disk space
//! checks, and atomic file writes.
//!
//! Config is written atomically (write to a temp file, then rename) so a
//! crash or power loss mid-save can never leave `config.json` half-written
//! and unparseable on next launch — that failure mode would be a uniquely
//! bad first-run experience for a returning user and is cheap to prevent.

pub mod config_store;
pub mod disk_space;
pub mod output_paths;
