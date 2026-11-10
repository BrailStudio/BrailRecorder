use brail_core::profile::{EncoderCapability, RecommendedPreset};

/// Turns a verified capability list + RAM size into one of the spec's
/// smart-default buckets. This is only ever a *starting point* the user
/// sees pre-selected in onboarding — every value it implies (resolution,
/// fps, encoder) remains individually editable, per the "always
/// overridable" requirement.
pub fn recommended_preset(encoders: &[EncoderCapability], total_ram_mb: u64) -> RecommendedPreset {
    let has_verified_hw_encoder = encoders
        .iter()
        .any(|e| e.verified && e.backend.is_hardware());

    let best_hw_max_pixels = encoders
        .iter()
        .filter(|e| e.verified && e.backend.is_hardware())
        .map(|e| e.max_resolution.pixel_count())
        .max()
        .unwrap_or(0);

    const RAM_LOW_END_MB: u64 = 8 * 1024;
    const RAM_HIGH_END_MB: u64 = 16 * 1024;
    const PIXELS_1440P: u64 = 2560 * 1440;

    if !has_verified_hw_encoder || total_ram_mb < RAM_LOW_END_MB {
        RecommendedPreset::LowEnd720p30
    } else if total_ram_mb >= RAM_HIGH_END_MB && best_hw_max_pixels >= PIXELS_1440P {
        RecommendedPreset::HighEnd1440p60
    } else {
        RecommendedPreset::Gaming1080p60
    }
}
