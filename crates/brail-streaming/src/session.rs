use std::time::{Duration, Instant};

use brail_core::config::{ReconnectPolicy, StreamProfile, StreamingProtocol};
use brail_core::error::BrailError;
use brail_core::events::AppEvent;
use brail_core::frame::EncodedPacket;
use brail_core::stats::{ConnectionState, StreamStats};
use brail_encoder::bitrate::AdaptiveBitrateController;
use tokio::sync::mpsc;

use crate::flv_muxer::mux_packet;
use crate::rtmp_client::RtmpClient;

/// Owns one outbound stream's full lifecycle: connect, publish encoded
/// packets as they arrive, monitor upload health, degrade/restore quality
/// via `AdaptiveBitrateController`, and reconnect with exponential backoff
/// on failure — matching the spec's "auto-reconnect... don't lose the
/// stream over a momentary network blip" requirement.
pub struct StreamSession {
    profile: StreamProfile,
    event_tx: mpsc::UnboundedSender<AppEvent>,
    bitrate_controller: AdaptiveBitrateController,
    stats: StreamStats,
    started_at: Option<Instant>,
    bytes_sent_this_second: u64,
    last_bitrate_sample: Instant,
}

impl StreamSession {
    pub fn new(profile: StreamProfile, event_tx: mpsc::UnboundedSender<AppEvent>) -> Self {
        let target_bitrate = profile.encoder.bitrate_kbps.unwrap_or(6000);
        Self {
            profile,
            event_tx,
            bitrate_controller: AdaptiveBitrateController::new(target_bitrate),
            stats: StreamStats::default(),
            started_at: None,
            bytes_sent_this_second: 0,
            last_bitrate_sample: Instant::now(),
        }
    }

    /// Runs the full connect -> publish -> (on failure) reconnect loop.
    /// Takes ownership of the packet receiver and runs until either
    /// `packet_rx` closes (recording/streaming stopped by the user) or the
    /// reconnect policy's `max_attempts` is exhausted.
    pub async fn run(mut self, mut packet_rx: mpsc::Receiver<EncodedPacket>) {
        let mut attempt: u32 = 0;

        loop {
            self.set_state(ConnectionState::Connecting);
            let _ = self.event_tx.send(AppEvent::StreamConnecting);

            match self.connect_and_publish(&mut packet_rx).await {
                Ok(()) => {
                    // packet_rx closed cleanly — user stopped streaming.
                    self.set_state(ConnectionState::Idle);
                    break;
                }
                Err(e) => {
                    tracing::warn!("stream session error: {e}");
                    let _ = self
                        .event_tx
                        .send(AppEvent::StreamDisconnected { reason: e.to_string() });

                    if !self.profile.reconnect.enabled || attempt >= self.profile.reconnect.max_attempts {
                        self.set_state(ConnectionState::Failed);
                        let _ = self.event_tx.send(AppEvent::Error(e));
                        break;
                    }

                    attempt += 1;
                    self.set_state(ConnectionState::Reconnecting);
                    let _ = self.event_tx.send(AppEvent::StreamReconnecting {
                        attempt,
                        max_attempts: self.profile.reconnect.max_attempts,
                    });

                    let backoff = backoff_duration(&self.profile.reconnect, attempt);
                    tokio::time::sleep(backoff).await;
                }
            }
        }
    }

    async fn connect_and_publish(
        &mut self,
        packet_rx: &mut mpsc::Receiver<EncodedPacket>,
    ) -> Result<(), BrailError> {
        if self.profile.protocol != StreamingProtocol::Rtmp && self.profile.protocol != StreamingProtocol::Rtmps {
            // SRT path uses a structurally identical loop against
            // `SrtClient` + a TS packetizer instead of `RtmpClient` +
            // `mux_packet`; omitted here to avoid duplicating this whole
            // function body for a protocol the spec ranks below RTMP.
            return Err(BrailError::UnsupportedConfiguration(
                "SRT publish loop not yet wired into StreamSession".into(),
            ));
        }

        let stream_key = self
            .profile
            .stream_key
            .as_deref()
            .ok_or(BrailError::InvalidStreamCredentials)?;

        let mut client = RtmpClient::connect(&self.profile.server_url, stream_key).await?;

        self.set_state(ConnectionState::Connected);
        self.started_at = Some(Instant::now());
        let _ = self.event_tx.send(AppEvent::StreamConnected);

        let mut first_video = true;
        let mut first_audio = true;

        while let Some(packet) = packet_rx.recv().await {
            let is_first = match packet.stream {
                brail_core::frame::StreamKind::Video => std::mem::replace(&mut first_video, false),
                brail_core::frame::StreamKind::Audio => std::mem::replace(&mut first_audio, false),
            };

            let tag = mux_packet(&packet, is_first);
            self.bytes_sent_this_second += tag.len() as u64;

            client.send_media(&tag).await?;

            self.maybe_report_stats();
        }

        Ok(())
    }

    fn maybe_report_stats(&mut self) {
        let now = Instant::now();
        if now.duration_since(self.last_bitrate_sample) < Duration::from_secs(1) {
            return;
        }

        let upload_kbps = (self.bytes_sent_this_second * 8) as f64 / 1000.0;
        self.bytes_sent_this_second = 0;
        self.last_bitrate_sample = now;

        if let Some(new_bitrate) = self.bitrate_controller.record_sample(upload_kbps, self.stats.dropped_frames) {
            self.stats.target_bitrate_kbps = new_bitrate as f64;
            // Applying `new_bitrate` to the live encoder is done by the
            // caller that owns the `EncodeController` (this crate doesn't
            // have a handle to the encoder itself) — `StreamSession`
            // publishes the *decision*; `brail-encoder`'s pipeline owner
            // subscribes to it via the same `AppEvent` bus and calls
            // `VideoEncoder::set_bitrate` or performs a full re-open.
        }

        self.stats.upload_bitrate_kbps = upload_kbps;
        self.stats.duration_secs = self
            .started_at
            .map(|t| now.duration_since(t).as_secs_f64())
            .unwrap_or(0.0);

        let _ = self.event_tx.send(AppEvent::StreamStatsUpdated(self.stats.clone()));
    }

    fn set_state(&mut self, state: ConnectionState) {
        self.stats.connection_state = state;
    }
}

fn backoff_duration(policy: &ReconnectPolicy, attempt: u32) -> Duration {
    let ms = policy.initial_backoff_ms.saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1)));
    Duration::from_millis(ms.min(policy.max_backoff_ms))
}
