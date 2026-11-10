//! brail-hardware: detects what this machine can actually do.
//!
//! Runs once at startup (and on-demand when the user hits "Re-detect
//! hardware" in Settings). Produces a `brail_core::CapabilityProfile` that
//! every other part of the app treats as ground truth — the UI filters
//! presets by it, brail-encoder refuses to open a backend it didn't
//! verify here, and brail-streaming won't offer a quality preset the
//! profile says is unsupported.
//!
//! Every probe in this crate either succeeds with a real measurement or
//! returns `None`/omits the entry — nothing here fabricates a capability.

pub mod cpu;
pub mod gpu;
pub mod monitors;
pub mod audio_devices;
pub mod cameras;
pub mod encoder_probe;
pub mod os_version;
pub mod recommend;

use brail_core::profile::CapabilityProfile;
use tracing::info;

/// Runs the full detection pass. This does real, possibly slow work
/// (opening/closing each hardware encoder costs tens of milliseconds each)
/// so it's called once and cached, not on every UI render.
pub async fn detect_capabilities() -> anyhow::Result<CapabilityProfile> {
    info!("starting hardware capability detection");

    let cpu = cpu::detect_cpu()?;
    let gpus = gpu::enumerate_gpus()?;
    let monitors = monitors::enumerate_monitors()?;
    let (audio_in, audio_out) = audio_devices::enumerate_audio_devices()?;
    let cameras = cameras::enumerate_cameras()?;
    let (os_build, os_name) = os_version::detect_windows_version()?;

    // Encoder probing depends on which GPU vendors are present, since e.g.
    // there's no point attempting to open h264_amf on an NVIDIA-only system.
    let supported_encoders = encoder_probe::probe_encoders(&gpus).await?;

    let profile = CapabilityProfile {
        cpu_name: cpu.name,
        cpu_physical_cores: cpu.physical_cores,
        cpu_logical_cores: cpu.logical_cores,
        total_ram_mb: cpu.total_ram_mb,
        gpus,
        windows_build: os_build,
        windows_version_name: os_name,
        monitors,
        audio_input_devices: audio_in,
        audio_output_devices: audio_out,
        cameras,
        recommended_preset: recommend::recommended_preset(&supported_encoders, cpu.total_ram_mb),
        supported_encoders,
    };

    info!(
        gpus = profile.gpus.len(),
        encoders = profile.supported_encoders.len(),
        monitors = profile.monitors.len(),
        "hardware detection complete"
    );

    Ok(profile)
}
