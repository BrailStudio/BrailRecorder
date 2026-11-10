use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use brail_core::config::EncoderSettings;
use brail_core::error::BrailResult;
use brail_core::frame::{EncodedPacket, VideoFrame};
use brail_core::stats::EncodeStats;
use tokio::sync::{broadcast, mpsc};

use crate::ffmpeg_encoder::FfmpegEncoder;
use crate::traits::VideoEncoder;

/// Bounded so a stalled downstream consumer (muxer blocked on a slow disk,
/// RTMP socket blocked on a slow upload) can't grow memory without limit —
/// backpressure here intentionally causes frame drops rather than an
/// unbounded queue, matching the spec's requirement that a slow disk/network
/// degrade gracefully rather than exhaust RAM.
const PACKET_CHANNEL_CAPACITY: usize = 64;

pub struct EncodeController {
    stats: Arc<EncodeStatsInner>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    /// Pause gates frame *submission*, not the thread itself. The encoder
    /// stays open and the output file stays valid, so resuming continues
    /// the same recording rather than starting a new one.
    paused: Arc<std::sync::atomic::AtomicBool>,
}

struct EncodeStatsInner {
    frames_encoded: AtomicU64,
    frames_dropped: AtomicU64,
}

impl EncodeController {
    /// Spawns the encode loop on a dedicated blocking thread (not a tokio
    /// async task) because hardware encoder `send_frame`/`receive_packet`
    /// calls can briefly block on driver-internal queues, which would stall
    /// the async runtime's worker thread if run as a normal task.
    ///
    /// Returns the controller (for `shutdown`/`stats`) separately from the
    /// packet stream itself, rather than bundling the `mpsc::Receiver`
    /// into this struct: the receiver is moved into whichever task
    /// actually consumes packets (a muxer-writer task for recording, a
    /// `StreamSession` for streaming), while the controller stays with
    /// whatever owns the *lifecycle* of the encode — those are frequently
    /// different owners, and a bundled struct would force one of them to
    /// hold a field it can't use.
    pub fn spawn(
        mut frame_rx: broadcast::Receiver<Arc<VideoFrame>>,
        settings: EncoderSettings,
        width: u32,
        height: u32,
        frame_rate: u32,
    ) -> BrailResult<(Self, mpsc::Receiver<EncodedPacket>)> {
        let (packet_tx, packet_rx) = mpsc::channel(PACKET_CHANNEL_CAPACITY);
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

        let stats = Arc::new(EncodeStatsInner {
            frames_encoded: AtomicU64::new(0),
            frames_dropped: AtomicU64::new(0),
        });
        let stats_for_thread = stats.clone();
        let paused = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let paused_for_thread = paused.clone();

        let mut encoder: Box<dyn VideoEncoder> =
            Box::new(FfmpegEncoder::new(settings, width, height, frame_rate)?);

        // Target frame interval for pacing — if capture delivers faster
        // than this (e.g. a 144Hz monitor with a 60fps recording target),
        // extra frames are dropped here rather than encoded and wasted.
        let target_interval_100ns = 10_000_000i64 / frame_rate as i64;
        let mut last_encoded_ts: i64 = i64::MIN;

        std::thread::Builder::new()
            .name("brail-encode".into())
            .spawn(move || {
                loop {
                    if *shutdown_rx.borrow() {
                        break;
                    }

                    match frame_rx.blocking_recv() {
                        Ok(frame) => {
                            if paused_for_thread.load(Ordering::Relaxed) {
                                // Dropped without counting as a failure —
                                // a paused recording isn't falling behind.
                                continue;
                            }
                            if frame.timestamp_100ns - last_encoded_ts < target_interval_100ns {
                                stats_for_thread.frames_dropped.fetch_add(1, Ordering::Relaxed);
                                continue; // pacing drop, not an error condition
                            }
                            last_encoded_ts = frame.timestamp_100ns;

                            if let Err(e) = encoder.submit_frame(&frame) {
                                tracing::error!("encoder submit_frame failed: {e}");
                                continue;
                            }

                            loop {
                                match encoder.receive_packet() {
                                    Ok(Some(packet)) => {
                                        stats_for_thread.frames_encoded.fetch_add(1, Ordering::Relaxed);
                                        if packet_tx.blocking_send(packet).is_err() {
                                            return; // receiver dropped, pipeline shutting down
                                        }
                                    }
                                    Ok(None) => break,
                                    Err(e) => {
                                        tracing::error!("encoder receive_packet failed: {e}");
                                        break;
                                    }
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            stats_for_thread
                                .frames_dropped
                                .fetch_add(skipped, Ordering::Relaxed);
                            tracing::warn!(skipped, "encoder fell behind capture; frames dropped");
                        }
                        Err(broadcast::error::RecvError::Closed) => break,
                    }
                }

                if let Ok(remaining) = encoder.flush() {
                    for packet in remaining {
                        let _ = packet_tx.blocking_send(packet);
                    }
                }
            })
            .expect("failed to spawn brail-encode thread");

        Ok((Self { stats, shutdown_tx, paused }, packet_rx))
    }

    pub fn stats(&self) -> EncodeStats {
        EncodeStats {
            encode_fps: 0.0, // computed by the caller from a rolling window over frames_encoded
            avg_encode_latency_ms: 0.0,
            frames_encoded: self.stats.frames_encoded.load(Ordering::Relaxed),
            frames_dropped_encoder_backpressure: self.stats.frames_dropped.load(Ordering::Relaxed),
            encoder_queue_depth: 0,
        }
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }
}
