use std::path::Path;

use windows::core::HSTRING;
use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

/// Real free-space check via `GetDiskFreeSpaceExW` (not a naive
/// `fs::metadata` walk, which doesn't report free space at all). Used both
/// before starting a recording (the spec requires refusing to start if
/// there isn't even a minute's headroom at the selected bitrate) and
/// periodically during recording to trigger the low-disk-space warning.
pub fn free_space_bytes(path: &Path) -> anyhow::Result<u64> {
    let path_str = path.to_string_lossy().to_string();
    let wide = HSTRING::from(path_str.as_str());

    let mut free_available = 0u64;
    unsafe {
        GetDiskFreeSpaceExW(&wide, Some(&mut free_available), None, None)?;
    }
    Ok(free_available)
}

/// Estimates remaining recordable seconds at a given bitrate, used to
/// populate the UI's "~2h 14m remaining at current quality" indicator and
/// to decide when to fire the low-disk-space warning threshold.
pub fn estimated_seconds_remaining(free_bytes: u64, total_bitrate_kbps: u32) -> f64 {
    if total_bitrate_kbps == 0 {
        return f64::INFINITY;
    }
    let bytes_per_sec = (total_bitrate_kbps as f64 * 1000.0) / 8.0;
    free_bytes as f64 / bytes_per_sec
}

/// Minimum free space required to *start* a recording at all — refuses to
/// start below one minute of estimated headroom rather than starting and
/// failing seconds later, per the spec's "warn before, not after" framing.
pub fn has_minimum_headroom(path: &Path, bitrate_kbps: u32) -> anyhow::Result<bool> {
    const MIN_SECONDS: f64 = 60.0;
    let free = free_space_bytes(path)?;
    Ok(estimated_seconds_remaining(free, bitrate_kbps) >= MIN_SECONDS)
}
