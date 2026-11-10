//! brail-recovery: detects an unclean previous shutdown and recovers
//! whatever recording was in progress at the time.
//!
//! The mechanism is a small "recording lock" JSON file written to the
//! config directory the instant a recording starts and deleted the instant
//! it finishes cleanly. If that file still exists at the next launch, the
//! previous session ended without reaching the clean-stop code path — a
//! crash, a forced power-off, or Windows Update rebooting the machine —
//! and the in-progress MKV file it points at is still valid up to its last
//! flushed cluster (see `brail-encoder::muxer`'s design note on why MKV is
//! the default specifically for this reason).

use std::path::{Path, PathBuf};

use brail_core::error::{BrailError, BrailResult};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecordingLock {
    pub output_path: PathBuf,
    pub started_at: chrono::DateTime<chrono::Local>,
    pub pid: u32,
}

pub struct RecoveryManager {
    lock_path: PathBuf,
}

impl RecoveryManager {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            lock_path: config_dir.join("recording.lock"),
        }
    }

    /// Call exactly once, at the very start of a recording, before the
    /// muxer's `write_header` is called.
    pub fn mark_recording_started(&self, output_path: &Path) -> BrailResult<()> {
        let lock = RecordingLock {
            output_path: output_path.to_path_buf(),
            started_at: chrono::Local::now(),
            pid: std::process::id(),
        };
        let json = serde_json::to_string_pretty(&lock)
            .map_err(|e| BrailError::Internal(format!("failed to serialize recording lock: {e}")))?;
        std::fs::write(&self.lock_path, json)
            .map_err(|e| BrailError::Internal(format!("failed to write recording lock: {e}")))?;
        Ok(())
    }

    /// Call exactly once, after the muxer's `finalize()` returns
    /// successfully. If this is never called for a given recording, the
    /// next launch treats it as crashed — which is the correct
    /// interpretation even for a "the user force-quit while stopping
    /// looked stuck" case, since the file's integrity in that scenario is
    /// exactly as uncertain as an actual crash.
    pub fn mark_recording_finished(&self) -> BrailResult<()> {
        if self.lock_path.exists() {
            std::fs::remove_file(&self.lock_path)
                .map_err(|e| BrailError::Internal(format!("failed to clear recording lock: {e}")))?;
        }
        Ok(())
    }

    /// Checked once at startup, before any new recording can begin.
    /// Returns the lock contents if a previous session crashed mid
    /// recording, so the caller (the Tauri app's startup sequence) can
    /// surface a "we recovered your last recording" notice pointing at
    /// `output_path`, and then clear the lock itself once the user has
    /// been told.
    pub fn check_for_crashed_recording(&self) -> Option<RecordingLock> {
        let contents = std::fs::read_to_string(&self.lock_path).ok()?;
        let lock: RecordingLock = serde_json::from_str(&contents).ok()?;

        // If the file the lock points at genuinely doesn't exist anymore
        // (e.g. the user manually deleted it, or it was on a since-removed
        // external drive), there's nothing to recover — still worth
        // clearing the stale lock so this check doesn't keep firing every
        // launch.
        if !lock.output_path.exists() {
            let _ = std::fs::remove_file(&self.lock_path);
            return None;
        }

        Some(lock)
    }

    pub fn clear_lock(&self) {
        let _ = std::fs::remove_file(&self.lock_path);
    }
}
