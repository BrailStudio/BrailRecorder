//! Integration tests that require a real Windows machine.
//!
//! Every test here is `#[ignore]`d so `cargo test` stays green on any
//! machine, and they're run deliberately:
//!
//! ```powershell
//! cargo test --test windows_integration -- --ignored --nocapture
//! ```
//!
//! These are not mocked. The whole point is to exercise the real WGC
//! session, the real encoder open, the real file write — a mocked version
//! would pass on the Linux box where this was written and prove nothing
//! about the thing that actually ships. Tests that need credentials or a
//! network endpoint read them from environment variables and skip
//! themselves (rather than fail) when those aren't set, so a contributor
//! without a stream key can still run everything else.

#![cfg(windows)]

use std::time::Duration;

// --------------------------------------------------------------------
// Hardware detection (§63: encoder detection, NVENC/AMF/QSV, fallback)
// --------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires real Windows hardware"]
async fn hardware_detection_finds_a_usable_configuration() {
    let caps = brail_hardware::detect_capabilities()
        .await
        .expect("hardware detection should not fail on a supported system");

    assert!(!caps.cpu_name.is_empty(), "CPU brand string should be readable");
    assert!(caps.cpu_logical_cores > 0);
    assert!(caps.total_ram_mb > 0);
    assert!(!caps.monitors.is_empty(), "at least one display must be enumerable");
    assert!(caps.windows_build > 0, "Windows build should come from the registry");

    // Software encoding has no hardware dependency, so its absence means
    // the FFmpeg build itself is wrong — worth failing loudly on.
    assert!(
        caps.supported_encoders
            .iter()
            .any(|e| e.backend == brail_core::config::EncoderBackend::Software),
        "software fallback must always be available"
    );

    println!("CPU: {}", caps.cpu_name);
    println!("RAM: {} MB", caps.total_ram_mb);
    for gpu in &caps.gpus {
        println!("GPU: {} ({:?}, {} MB VRAM)", gpu.name, gpu.vendor, gpu.dedicated_vram_mb);
    }
    for enc in &caps.supported_encoders {
        println!(
            "Encoder: {} — verified: {}",
            enc.backend.display_name(enc.codec),
            enc.verified
        );
    }
}

#[tokio::test]
#[ignore = "requires a GPU with a hardware encoder"]
async fn at_least_one_hardware_encoder_verifies_on_a_gpu_system() {
    let caps = brail_hardware::detect_capabilities().await.unwrap();

    if caps.gpus.iter().all(|g| g.dedicated_vram_mb == 0) {
        eprintln!("No discrete GPU present; skipping.");
        return;
    }

    let verified_hw: Vec<_> = caps
        .supported_encoders
        .iter()
        .filter(|e| e.verified && e.backend.is_hardware())
        .collect();

    assert!(
        !verified_hw.is_empty(),
        "a system with a discrete GPU should verify at least one hardware encoder. \
         If this fails, check that FFmpeg was built with nvcodec/amf/qsv support \
         (see docs/BUILD.md step 4)."
    );
}

// --------------------------------------------------------------------
// Capture (§63: capture initialization, monitor selection, multi-monitor)
// --------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires a real display"]
async fn capture_session_starts_and_delivers_frames() {
    let caps = brail_hardware::detect_capabilities().await.unwrap();
    let monitor = caps.monitors.first().expect("a display is required");

    let mut engine = brail_capture::CaptureEngine::new().expect("D3D11 device creation failed");
    let handle = engine.handle();
    let mut frames = handle.subscribe();

    engine
        .start_monitor_capture(
            windows::Win32::Graphics::Gdi::HMONITOR(monitor.handle_id as *mut _),
            true,
        )
        .expect("capture should start on the primary monitor");

    // A completely static desktop can take a moment to produce a frame,
    // since WGC only delivers on change.
    let frame = tokio::time::timeout(Duration::from_secs(5), frames.recv())
        .await
        .expect("timed out waiting for a captured frame")
        .expect("frame channel closed unexpectedly");

    assert!(frame.width > 0 && frame.height > 0);
    assert!(frame.timestamp_100ns > 0, "frames must carry a QPC-derived timestamp");

    engine.stop();
}

#[tokio::test]
#[ignore = "requires a real display"]
async fn capture_stops_cleanly_and_releases_its_session() {
    let caps = brail_hardware::detect_capabilities().await.unwrap();
    let monitor = caps.monitors.first().unwrap();

    // Start/stop repeatedly: a session that leaks would show up here as a
    // failure to re-acquire, which is exactly the §65 handle-leak check.
    for iteration in 0..5 {
        let mut engine = brail_capture::CaptureEngine::new().unwrap();
        engine
            .start_monitor_capture(
                windows::Win32::Graphics::Gdi::HMONITOR(monitor.handle_id as *mut _),
                false,
            )
            .unwrap_or_else(|e| panic!("capture failed to start on iteration {iteration}: {e}"));
        assert!(engine.is_capturing());
        engine.stop();
        assert!(!engine.is_capturing());
    }
}

// --------------------------------------------------------------------
// Recording end to end (§63: recording, MKV finalization, output validation)
// --------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires real hardware; writes a file to the temp directory"]
async fn software_recording_produces_a_playable_file() {
    let caps = brail_hardware::detect_capabilities().await.unwrap();
    let monitor = caps.monitors.first().unwrap();

    let output = std::env::temp_dir().join("brail_integration_test.mkv");
    let _ = std::fs::remove_file(&output);

    let mut engine = brail_capture::CaptureEngine::new().unwrap();
    let handle = engine.handle();
    engine
        .start_monitor_capture(
            windows::Win32::Graphics::Gdi::HMONITOR(monitor.handle_id as *mut _),
            true,
        )
        .unwrap();

    // Software encoding deliberately, so this test passes on any machine
    // and isolates the capture->encode->mux chain from hardware variables.
    let settings = brail_core::config::EncoderSettings {
        backend: brail_core::config::EncoderBackend::Software,
        codec: brail_core::config::VideoCodec::H264,
        rate_control: brail_core::config::RateControlMode::Vbr,
        bitrate_kbps: Some(2_000),
        cqp_level: None,
        keyframe_interval_secs: 2.0,
        preset: brail_core::config::EncoderPreset::Fastest,
        profile: brail_core::config::EncoderProfile::High,
        b_frames: 0,
    };

    let (encoder, mut packets) =
        brail_encoder::EncodeController::spawn(handle.subscribe(), settings, 1280, 720, 30)
            .expect("encoder should open");

    let mut muxer = brail_encoder::Muxer::create(
        &output,
        brail_core::config::ContainerFormat::Mkv,
        brail_core::config::VideoCodec::H264,
        1280,
        720,
        30,
        false,
    )
    .expect("muxer should open the output file");

    let writer = tokio::spawn(async move {
        let mut count = 0;
        while let Some(packet) = packets.recv().await {
            muxer.write_packet(&packet).expect("packet write failed");
            count += 1;
        }
        muxer.finalize().expect("finalize failed");
        count
    });

    tokio::time::sleep(Duration::from_secs(5)).await;
    encoder.shutdown();
    engine.stop();

    let packet_count = writer.await.unwrap();

    assert!(packet_count > 0, "no packets were encoded in 5 seconds");
    let size = std::fs::metadata(&output).expect("output file missing").len();
    assert!(size > 1024, "output file is suspiciously small ({size} bytes)");

    println!("Wrote {packet_count} packets, {size} bytes to {}", output.display());
    let _ = std::fs::remove_file(&output);
}

// --------------------------------------------------------------------
// Audio (§63)
// --------------------------------------------------------------------

#[test]
#[ignore = "requires a real audio output device"]
fn wasapi_loopback_opens_and_reports_a_format() {
    let capture = brail_audio::WasapiCapture::open_desktop_loopback()
        .expect("desktop loopback should open on any system with an output device");

    let format = capture.format();
    assert!(format.sample_rate >= 44_100, "unexpected rate: {}", format.sample_rate);
    assert!(format.channels >= 1);

    // Silence still produces packets on a loopback stream, so a read that
    // errors (rather than returning empty) is a real failure.
    std::thread::sleep(Duration::from_millis(200));
    capture.read_available().expect("reading from the loopback stream failed");
}

// --------------------------------------------------------------------
// Security (§63: stream-key security)
// --------------------------------------------------------------------

#[test]
#[ignore = "writes to the real Windows Credential Manager"]
fn stream_keys_round_trip_through_the_credential_vault() {
    use brail_security::CredentialVault;

    let id = uuid::Uuid::new_v4();
    let secret = "test-key-do-not-use-abc123456789";

    CredentialVault::store_stream_key(id, secret).expect("store failed");

    let loaded = CredentialVault::load_stream_key(id)
        .expect("load failed")
        .expect("key should exist after storing");
    assert_eq!(loaded, secret);

    CredentialVault::delete_stream_key(id).expect("delete failed");
    assert!(
        CredentialVault::load_stream_key(id).unwrap().is_none(),
        "key should be gone after deletion"
    );
}

#[test]
#[ignore = "requires the credential vault"]
fn config_file_never_contains_a_stream_key() {
    // StreamProfile::stream_key is #[serde(skip)], so even a profile
    // holding a live key in memory must serialize without it.
    let profile = brail_core::config::StreamProfile {
        id: uuid::Uuid::new_v4(),
        name: "Test".into(),
        service: brail_core::config::StreamingService::YouTube,
        protocol: brail_core::config::StreamingProtocol::Rtmp,
        server_url: "rtmp://a.rtmp.youtube.com/live2".into(),
        stream_key: Some("super-secret-key-9876543210".into()),
        resolution: brail_core::config::Resolution::new(1920, 1080),
        frame_rate: brail_core::config::FrameRate::Fps60,
        encoder: brail_core::config::EncoderSettings {
            backend: brail_core::config::EncoderBackend::Software,
            codec: brail_core::config::VideoCodec::H264,
            rate_control: brail_core::config::RateControlMode::Cbr,
            bitrate_kbps: Some(6000),
            cqp_level: None,
            keyframe_interval_secs: 2.0,
            preset: brail_core::config::EncoderPreset::Balanced,
            profile: brail_core::config::EncoderProfile::High,
            b_frames: 0,
        },
        reconnect: Default::default(),
    };

    let json = serde_json::to_string(&profile).unwrap();
    assert!(
        !json.contains("super-secret-key"),
        "the stream key leaked into serialized config: {json}"
    );
}

// --------------------------------------------------------------------
// Streaming (§63: RTMP connection) — needs credentials
// --------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires BRAIL_TEST_RTMP_URL and BRAIL_TEST_STREAM_KEY"]
async fn rtmp_connection_test_reaches_a_real_ingest() {
    let (Ok(url), Ok(key)) = (
        std::env::var("BRAIL_TEST_RTMP_URL"),
        std::env::var("BRAIL_TEST_STREAM_KEY"),
    ) else {
        eprintln!("Set BRAIL_TEST_RTMP_URL and BRAIL_TEST_STREAM_KEY to run this test.");
        return;
    };

    let profile = brail_core::config::StreamProfile {
        id: uuid::Uuid::new_v4(),
        name: "Integration test".into(),
        service: brail_core::config::StreamingService::Custom,
        protocol: brail_core::config::StreamingProtocol::Rtmp,
        server_url: url,
        stream_key: Some(key),
        resolution: brail_core::config::Resolution::new(1280, 720),
        frame_rate: brail_core::config::FrameRate::Fps30,
        encoder: brail_core::config::EncoderSettings {
            backend: brail_core::config::EncoderBackend::Software,
            codec: brail_core::config::VideoCodec::H264,
            rate_control: brail_core::config::RateControlMode::Cbr,
            bitrate_kbps: Some(3000),
            cqp_level: None,
            keyframe_interval_secs: 2.0,
            preset: brail_core::config::EncoderPreset::Fastest,
            profile: brail_core::config::EncoderProfile::Main,
            b_frames: 0,
        },
        reconnect: Default::default(),
    };

    let result = brail_streaming::test_connection(&profile).await;
    println!("Test result: {} — {}", result.success, result.message);
    if let Some(ms) = result.round_trip_ms {
        println!("Round trip: {ms:.0} ms");
    }

    assert!(result.success, "connection test failed: {}", result.message);
}

// --------------------------------------------------------------------
// Resource leak check (§65)
// --------------------------------------------------------------------

#[tokio::test]
#[ignore = "long-running; set BRAIL_LEAK_TEST_MINUTES (default 30)"]
async fn memory_stays_bounded_over_a_long_recording() {
    let minutes: u64 = std::env::var("BRAIL_LEAK_TEST_MINUTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(30);

    let caps = brail_hardware::detect_capabilities().await.unwrap();
    let monitor = caps.monitors.first().unwrap();

    let mut engine = brail_capture::CaptureEngine::new().unwrap();
    let handle = engine.handle();
    engine
        .start_monitor_capture(
            windows::Win32::Graphics::Gdi::HMONITOR(monitor.handle_id as *mut _),
            true,
        )
        .unwrap();

    let settings = brail_core::config::EncoderSettings {
        backend: brail_core::config::EncoderBackend::Software,
        codec: brail_core::config::VideoCodec::H264,
        rate_control: brail_core::config::RateControlMode::Cbr,
        bitrate_kbps: Some(3000),
        cqp_level: None,
        keyframe_interval_secs: 2.0,
        preset: brail_core::config::EncoderPreset::Fastest,
        profile: brail_core::config::EncoderProfile::Main,
        b_frames: 0,
    };

    let (encoder, mut packets) =
        brail_encoder::EncodeController::spawn(handle.subscribe(), settings, 1280, 720, 30).unwrap();
    tokio::spawn(async move { while packets.recv().await.is_some() {} });

    let mut monitor_probe = brail_performance::ResourceMonitor::new(caps.cpu_logical_cores);
    let mut samples = Vec::new();

    for _ in 0..(minutes * 60 / 10) {
        tokio::time::sleep(Duration::from_secs(10)).await;
        let ram = monitor_probe.sample().process_ram_mb;
        samples.push(ram);
        println!("RAM: {ram:.1} MB");
    }

    encoder.shutdown();
    engine.stop();

    let first_quarter: f64 = samples[..samples.len() / 4].iter().sum::<f64>() / (samples.len() / 4) as f64;
    let last_quarter: f64 =
        samples[samples.len() * 3 / 4..].iter().sum::<f64>() / (samples.len() - samples.len() * 3 / 4) as f64;
    let growth = last_quarter - first_quarter;

    println!("Mean RAM: first quarter {first_quarter:.1} MB, last quarter {last_quarter:.1} MB");
    assert!(
        growth < 50.0,
        "memory grew {growth:.1} MB over {minutes} minutes, which suggests a leak"
    );
}
