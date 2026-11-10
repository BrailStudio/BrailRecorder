use std::path::{Path, PathBuf};

/// Generates a collision-free output filename following the spec's default
/// naming convention (`<Type>_<YYYY-MM-DD>_<HH-MM-SS>.<ext>`), and falls
/// back to appending `_1`, `_2`, ... in the rare case two recordings start
/// within the same second (e.g. instant-replay save + manual recording
/// start racing each other).
pub fn generate_output_path(dir: &Path, prefix: &str, extension: &str) -> PathBuf {
    let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
    let base_name = format!("{prefix}_{timestamp}");

    let candidate = dir.join(format!("{base_name}.{extension}"));
    if !candidate.exists() {
        return candidate;
    }

    for i in 1..1000 {
        let candidate = dir.join(format!("{base_name}_{i}.{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }

    // Effectively unreachable (1000 recordings starting in the same
    // second), but returning a deterministic path beats panicking.
    dir.join(format!("{base_name}_overflow.{extension}"))
}

pub fn default_output_dir() -> anyhow::Result<PathBuf> {
    let user_profile = std::env::var("USERPROFILE")
        .map_err(|_| anyhow::anyhow!("%USERPROFILE% is not set"))?;
    let dir = Path::new(&user_profile).join("Videos").join("Brail Recorder");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}
