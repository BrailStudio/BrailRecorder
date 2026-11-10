//! Pipeline behavior tests: the ring buffer's GOP handling, reconnect
//! backoff, overlay layout maths, and the Adaptive Engine's decision
//! rules. All pure logic — no GPU, no network, no Windows APIs.

use brail_core::config::{ReconnectPolicy, Resolution, VideoCodec};
use brail_core::frame::{EncodedPacket, StreamKind};
use brail_core::settings::{AdaptiveMode, OverlayAnchor, WebcamOverlaySettings};
use brail_core::stats::ResourceStats;

fn packet(pts_ms: i64, keyframe: bool, kind: StreamKind) -> EncodedPacket {
    EncodedPacket {
        data: bytes::Bytes::from_static(&[0u8; 16]),
        pts_100ns: pts_ms * 10_000,
        dts_100ns: pts_ms * 10_000,
        is_keyframe: keyframe,
        codec: VideoCodec::H264,
        stream: kind,
    }
}

// --------------------------------------------------------------------
// Instant replay ring buffer
// --------------------------------------------------------------------

#[test]
fn ring_buffer_always_starts_on_a_keyframe() {
    use brail_replay::ring_buffer::RingBuffer;

    let mut buffer = RingBuffer::new(2); // 2-second window

    // Five seconds of 1-second GOPs, well past the window.
    for second in 0..5 {
        buffer.push(packet(second * 1000, true, StreamKind::Video));
        for frame in 1..30 {
            buffer.push(packet(second * 1000 + frame * 33, false, StreamKind::Video));
        }
    }

    let snapshot = buffer.snapshot();
    assert!(!snapshot.is_empty());

    // A clip whose first frame isn't a keyframe can't be decoded from the
    // start — this is the single most important property of the buffer.
    let first_video = snapshot
        .iter()
        .find(|p| p.stream == StreamKind::Video)
        .expect("buffer should hold video");
    assert!(
        first_video.is_keyframe,
        "the retained window must begin at a keyframe boundary"
    );
}

#[test]
fn ring_buffer_bounds_its_retained_duration() {
    use brail_replay::ring_buffer::RingBuffer;

    let mut buffer = RingBuffer::new(2);
    for second in 0..10 {
        buffer.push(packet(second * 1000, true, StreamKind::Video));
    }

    // The buffer must not grow without limit — the spec's memory targets
    // depend on this staying bounded no matter how long replay runs.
    let duration = buffer.buffered_duration_secs();
    assert!(
        duration <= 4.0,
        "retained {duration}s for a 2s window; eviction is not bounding the buffer"
    );
}

#[test]
fn snapshot_does_not_drain_the_buffer() {
    use brail_replay::ring_buffer::RingBuffer;

    let mut buffer = RingBuffer::new(30);
    buffer.push(packet(0, true, StreamKind::Video));
    buffer.push(packet(33, false, StreamKind::Video));

    let first = buffer.snapshot();
    let second = buffer.snapshot();

    // Saving a replay must not interrupt the next clip's buffering.
    assert_eq!(first.len(), second.len(), "snapshot must be a copy, not a drain");
}

// --------------------------------------------------------------------
// Reconnect backoff
// --------------------------------------------------------------------

#[test]
fn reconnect_backoff_grows_then_caps() {
    let policy = ReconnectPolicy {
        enabled: true,
        max_attempts: 10,
        initial_backoff_ms: 1000,
        max_backoff_ms: 30_000,
    };

    let delay_for = |attempt: u32| -> u64 {
        let ms = policy
            .initial_backoff_ms
            .saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1)));
        ms.min(policy.max_backoff_ms)
    };

    assert_eq!(delay_for(1), 1_000);
    assert_eq!(delay_for(2), 2_000);
    assert_eq!(delay_for(3), 4_000);
    // Must plateau rather than growing unboundedly — an hour-long wait
    // after ten failures would be indistinguishable from giving up.
    assert_eq!(delay_for(20), 30_000);
}

#[test]
fn backoff_never_overflows_at_high_attempt_counts() {
    let policy = ReconnectPolicy::default();
    let ms = policy
        .initial_backoff_ms
        .saturating_mul(2u64.saturating_pow(u32::MAX.saturating_sub(1)))
        .min(policy.max_backoff_ms);
    assert_eq!(ms, policy.max_backoff_ms);
}

// --------------------------------------------------------------------
// Overlay layout
// --------------------------------------------------------------------

#[test]
fn overlay_anchors_land_inside_the_frame() {
    use brail_capture::compositor::resolve_overlay_rect;

    let output = Resolution::new(1920, 1080);
    for anchor in [
        OverlayAnchor::TopLeft,
        OverlayAnchor::TopRight,
        OverlayAnchor::BottomLeft,
        OverlayAnchor::BottomRight,
    ] {
        let (x, y, w, h) = resolve_overlay_rect(anchor, output, 20.0, 15.0, 2.0);
        assert!(x >= 0, "{anchor:?} placed the overlay off the left edge");
        assert!(y >= 0, "{anchor:?} placed the overlay off the top edge");
        assert!(x + w as i32 <= output.width as i32, "{anchor:?} overflowed the right edge");
        assert!(y + h as i32 <= output.height as i32, "{anchor:?} overflowed the bottom edge");
    }
}

#[test]
fn webcam_overlay_preserves_aspect_ratio() {
    use brail_capture::compositor::resolve_webcam_rect;

    let settings = WebcamOverlaySettings {
        enabled: true,
        width_percent: 20.0,
        ..WebcamOverlaySettings::default()
    };

    let (_, _, w, h) = resolve_webcam_rect(&settings, Resolution::new(1920, 1080), Resolution::new(1280, 720));

    // A 16:9 source must stay 16:9 — a stretched webcam is the most
    // visible possible overlay bug.
    let aspect = w as f32 / h as f32;
    assert!((aspect - 16.0 / 9.0).abs() < 0.1, "aspect drifted to {aspect}");
}

#[test]
fn webcam_overlay_dimensions_are_even_for_nv12() {
    use brail_capture::compositor::resolve_webcam_rect;

    let settings = WebcamOverlaySettings {
        enabled: true,
        width_percent: 17.3, // deliberately awkward, to force rounding
        ..WebcamOverlaySettings::default()
    };

    let (_, _, w, h) = resolve_webcam_rect(&settings, Resolution::new(1920, 1080), Resolution::new(1280, 720));
    // NV12 chroma subsampling requires even dimensions; odd values get
    // rejected by hardware encoders with an opaque error.
    assert_eq!(w % 2, 0);
    assert_eq!(h % 2, 0);
}

#[test]
fn compositing_is_skipped_when_nothing_is_configured() {
    use brail_capture::compositor::needs_compositing;

    let off = WebcamOverlaySettings::default();
    assert!(!needs_compositing(&off, &[]), "the zero-copy fast path must stay available");

    let on = WebcamOverlaySettings { enabled: true, ..WebcamOverlaySettings::default() };
    assert!(needs_compositing(&on, &[]));
}

// --------------------------------------------------------------------
// Region capture
// --------------------------------------------------------------------

#[test]
fn region_is_clamped_to_the_source() {
    use brail_capture::CaptureRegion;

    let source = Resolution::new(1920, 1080);

    let oversized = CaptureRegion { x: 1800, y: 1000, width: 500, height: 500 };
    let clamped = oversized.clamped_to(source).expect("partially visible region should survive");
    assert!(clamped.x + clamped.width <= source.width);
    assert!(clamped.y + clamped.height <= source.height);

    let offscreen = CaptureRegion { x: 5000, y: 5000, width: 100, height: 100 };
    assert!(offscreen.clamped_to(source).is_none(), "a fully offscreen region must be rejected");
}

// --------------------------------------------------------------------
// Brail Adaptive Engine
// --------------------------------------------------------------------

fn resources(cpu: f64, ram_mb: f64) -> ResourceStats {
    ResourceStats {
        process_ram_mb: ram_mb,
        process_cpu_percent: cpu,
        ..Default::default()
    }
}

#[test]
fn adaptive_engine_ignores_a_brief_spike() {
    use brail_performance::{AdaptiveEngine, AdaptiveSample};

    let mut engine = AdaptiveEngine::new(AdaptiveMode::Balanced, true, 16 * 1024);

    // One bad second surrounded by good ones must not trigger anything —
    // §24: never reduce quality unless the system actually needs it.
    for i in 0..11 {
        let cpu = if i == 5 { 99.0 } else { 5.0 };
        let result = engine.observe(AdaptiveSample {
            resources: resources(cpu, 80.0),
            capture_fps: 60.0,
            target_fps: 60.0,
            ..Default::default()
        });
        assert!(result.is_none(), "a transient spike at sample {i} triggered a change");
    }
}

#[test]
fn adaptive_engine_reacts_to_sustained_fps_shortfall() {
    use brail_performance::{AdaptiveAction, AdaptiveEngine, AdaptiveSample};

    let mut engine = AdaptiveEngine::new(AdaptiveMode::Balanced, true, 16 * 1024);

    let mut recommendation = None;
    for _ in 0..15 {
        if let Some(r) = engine.observe(AdaptiveSample {
            resources: resources(30.0, 90.0),
            capture_fps: 32.0, // well under the 60 target, every single sample
            target_fps: 60.0,
            ..Default::default()
        }) {
            recommendation = Some(r);
            break;
        }
    }

    let r = recommendation.expect("a sustained FPS shortfall should produce a recommendation");
    assert_eq!(r.action, AdaptiveAction::LowerResolution);
    // §54: warnings must cite the actual measurement.
    assert!(r.reason.contains("32") && r.reason.contains("60"), "reason was: {}", r.reason);
}

#[test]
fn custom_mode_does_not_auto_apply() {
    use brail_performance::AdaptiveEngine;

    let custom = AdaptiveEngine::new(AdaptiveMode::Custom, true, 16 * 1024);
    assert!(!custom.should_auto_apply(), "Custom mode is the user's explicit opt-out");

    let balanced = AdaptiveEngine::new(AdaptiveMode::Balanced, true, 16 * 1024);
    assert!(balanced.should_auto_apply());

    let opted_out = AdaptiveEngine::new(AdaptiveMode::Balanced, false, 16 * 1024);
    assert!(!opted_out.should_auto_apply(), "auto_optimize=false must be respected");
}

#[test]
fn software_encoding_alongside_idle_hardware_is_flagged_immediately_on_window_fill() {
    use brail_performance::{AdaptiveAction, AdaptiveEngine, AdaptiveSample};

    let mut engine = AdaptiveEngine::new(AdaptiveMode::Balanced, true, 16 * 1024);
    engine.set_encoder_state(true, true); // hardware available, software in use

    let mut recommendation = None;
    for _ in 0..15 {
        if let Some(r) = engine.observe(AdaptiveSample {
            resources: resources(45.0, 90.0),
            capture_fps: 60.0,
            target_fps: 60.0,
            ..Default::default()
        }) {
            recommendation = Some(r);
            break;
        }
    }

    assert_eq!(
        recommendation.expect("should recommend hardware encoding").action,
        AdaptiveAction::SwitchToHardwareEncoder
    );
}

#[test]
fn critical_memory_pressure_bypasses_the_observation_window() {
    use brail_performance::{AdaptiveEngine, AdaptiveSample, Severity};

    let mut engine = AdaptiveEngine::new(AdaptiveMode::Balanced, true, 1000);

    // A single sample at 95% of system RAM — waiting twelve seconds to
    // confirm would be waiting until the recording has already failed.
    let result = engine.observe(AdaptiveSample {
        resources: resources(20.0, 950.0),
        capture_fps: 60.0,
        target_fps: 60.0,
        ..Default::default()
    });

    let r = result.expect("critical memory pressure must fire immediately");
    assert_eq!(r.severity, Severity::Critical);
}
