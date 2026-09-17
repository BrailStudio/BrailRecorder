use std::time::Instant;

use brail_core::config::{StreamProfile, StreamingProtocol};
use brail_core::error::{BrailError, BrailResult};
use serde::{Deserialize, Serialize};

use crate::rtmp_client::RtmpClient;

/// Result of a real connection test (§60, §67: the TEST CONNECTION button
/// must actually test the configured endpoint — no fake success messages).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TestResult {
    pub success: bool,
    /// Written for the user, naming the specific failure where one
    /// occurred rather than a generic "connection failed".
    pub message: String,
    /// Time from TCP connect through to the server accepting the publish
    /// request. This is a genuine measurement of ingest round-trip, not an
    /// estimate — `None` only when the test failed before completing.
    pub round_trip_ms: Option<f64>,
}

/// Performs a complete, real connect → handshake → connect → publish →
/// disconnect cycle against the configured ingest endpoint, then tears the
/// connection down without sending any media.
///
/// This is a genuine end-to-end check: it proves the server is reachable,
/// the protocol negotiates, and critically that the *stream key is
/// accepted* — which a plain TCP reachability check would not catch, and
/// which is by far the most common real-world streaming failure.
pub async fn test_connection(profile: &StreamProfile) -> TestResult {
    match profile.protocol {
        StreamingProtocol::Rtmp | StreamingProtocol::Rtmps => test_rtmp(profile).await,
        StreamingProtocol::Srt => TestResult {
            success: false,
            message: "Connection testing for SRT is not implemented yet. RTMP and RTMPS are supported."
                .into(),
            round_trip_ms: None,
        },
    }
}

async fn test_rtmp(profile: &StreamProfile) -> TestResult {
    let Some(stream_key) = profile.stream_key.as_deref() else {
        return TestResult {
            success: false,
            message: "No stream key is set for this profile.".into(),
            round_trip_ms: None,
        };
    };

    if profile.server_url.trim().is_empty() {
        return TestResult {
            success: false,
            message: "No server URL is set for this profile.".into(),
            round_trip_ms: None,
        };
    }

    let started = Instant::now();

    // A publish request that hangs is as much a failure as one that's
    // refused — an ingest endpoint that doesn't answer within 15 seconds
    // will not carry a live stream reliably.
    let attempt = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        RtmpClient::connect(&profile.server_url, stream_key),
    )
    .await;

    match attempt {
        Ok(Ok(client)) => {
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            // Dropping the client closes the socket. Nothing was published,
            // so this never appears as a real (empty) broadcast on the
            // user's channel.
            drop(client);
            TestResult {
                success: true,
                message: format!(
                    "Connected to {} and the stream key was accepted.",
                    host_of(&profile.server_url)
                ),
                round_trip_ms: Some(elapsed),
            }
        }
        Ok(Err(e)) => TestResult {
            success: false,
            message: explain_failure(&e, &profile.server_url),
            round_trip_ms: None,
        },
        Err(_) => TestResult {
            success: false,
            message: format!(
                "{} did not respond within 15 seconds. Check the server URL and your connection.",
                host_of(&profile.server_url)
            ),
            round_trip_ms: None,
        },
    }
}

/// Turns a protocol-level error into something actionable (§35: never show
/// a bare error code; say what to try).
fn explain_failure(error: &BrailError, server_url: &str) -> String {
    let host = host_of(server_url);
    match error {
        BrailError::InvalidStreamCredentials => format!(
            "{host} rejected the stream key. Copy it again from your channel's live dashboard — keys are regenerated when you reset them."
        ),
        BrailError::StreamConnectFailed(detail) if detail.contains("TCP connect") => format!(
            "Could not reach {host}. Check your internet connection, and that a firewall isn't blocking outbound RTMP."
        ),
        BrailError::StreamConnectFailed(detail) if detail.contains("handshake") => format!(
            "{host} accepted the connection but the RTMP handshake failed. The server URL may point at something that isn't an RTMP ingest."
        ),
        BrailError::StreamConnectFailed(detail) => {
            format!("Could not start publishing to {host}: {detail}")
        }
        other => format!("Connection test failed: {other}"),
    }
}

/// Extracts just the hostname for user-facing messages. The stream key is
/// never part of the URL in this codebase (see `rtmp_client::parse_rtmp_url`),
/// so nothing sensitive can leak into a message built from it.
fn host_of(url: &str) -> String {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(url)
        .to_string()
}
