use std::path::{Path, PathBuf};

use brail_core::config::AppConfig;
use brail_core::error::{BrailError, BrailResult};

/// Loads/saves `AppConfig` as JSON under `%APPDATA%\BrailRecorder\config.json`.
/// Stream keys are never part of this file (see `brail-security::vault`) —
/// `StreamProfile::stream_key` is `#[serde(skip)]`, so even if this file is
/// inspected or backed up by the user, it cannot leak a stream key.
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    pub fn new() -> anyhow::Result<Self> {
        let dir = config_dir()?;
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            path: dir.join("config.json"),
        })
    }

    pub fn load(&self) -> BrailResult<Option<AppConfig>> {
        if !self.path.exists() {
            return Ok(None);
        }

        let contents = std::fs::read_to_string(&self.path)
            .map_err(|e| BrailError::Internal(format!("failed to read config: {e}")))?;

        match serde_json::from_str(&contents) {
            Ok(config) => Ok(Some(config)),
            Err(e) => {
                // A corrupt config (e.g. truncated by a crash despite the
                // atomic-write scheme below, or hand-edited badly) should
                // never prevent the app from starting — back up the bad
                // file for diagnostics and fall through to defaults.
                tracing::error!("config.json is corrupt, ignoring and backing up: {e}");
                let backup_path = self.path.with_extension("json.corrupt");
                let _ = std::fs::copy(&self.path, &backup_path);
                Ok(None)
            }
        }
    }

    pub fn save(&self, config: &AppConfig) -> BrailResult<()> {
        let json = serde_json::to_string_pretty(config)
            .map_err(|e| BrailError::Internal(format!("failed to serialize config: {e}")))?;

        // Atomic write: write to a sibling temp file, then rename over the
        // real path. On NTFS, rename-over-existing is atomic at the
        // filesystem level, so a crash mid-write leaves either the old
        // config or the new one intact, never a half-written file.
        let tmp_path = self.path.with_extension("json.tmp");
        std::fs::write(&tmp_path, json)
            .map_err(|e| BrailError::Internal(format!("failed to write temp config: {e}")))?;
        std::fs::rename(&tmp_path, &self.path)
            .map_err(|e| BrailError::Internal(format!("failed to commit config: {e}")))?;

        Ok(())
    }
}

fn config_dir() -> anyhow::Result<PathBuf> {
    let appdata = std::env::var("APPDATA")
        .map_err(|_| anyhow::anyhow!("%APPDATA% is not set (unexpected on Windows)"))?;
    Ok(Path::new(&appdata).join("BrailRecorder"))
}
