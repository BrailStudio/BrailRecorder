use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use brail_core::config::{ContainerFormat, InstantReplaySettings};
use brail_core::error::{BrailError, BrailResult};
use brail_core::frame::VideoFrame;
use brail_encoder::muxer::Muxer;
use brail_encoder::traits::VideoEncoder;
use brail_encoder::ffmpeg_encoder::FfmpegEncoder;
use tokio::sync::broadcast;

use crate::ring_buffer::RingBuffer;

pub struct ReplayEngine {
    buffer: Arc<Mutex<RingBuffer>>,
    shutdown_tx: tokio::sync::watch::Sender<bool>,
}

impl ReplayEngine {
    /// Spawns the background encode-into-ring-buffer loop. Mirrors
    /// `brail-encoder::EncodeController`'s structure deliberately (dedicated
    /// OS thread, same pacing/backpressure handling) since the two have
    /// identical real-time constraints — the only difference is the sink
    /// (`RingBuffer` instead of an `mpsc` channel to a muxer/streamer).
    pub fn spawn(
        mut frame_rx: broadcast::Receiver<Arc<VideoFrame>>,
        settings: InstantReplaySettings,
        frame_rate: u32,
    ) -> BrailResult<Self> {
        let buffer = Arc::new(Mutex::new(RingBuffer::new(settings.buffer_seconds)));
        let buffer_for_thread = buffer.clone();
        let (shutdown_tx, mut shutdown_rx) = tokio::sync::watch::channel(false);

        let mut encoder: Box<dyn VideoEncoder> = Box::new(FfmpegEncoder::new(
            settings.encoder,
            settings.resolution.width,
            settings.resolution.height,
            frame_rate,
        )?);

        std::thread::Builder::new()
            .name("brail-replay-encode".into())
            .spawn(move || loop {
                if *shutdown_rx.borrow() {
                    break;
                }

                match frame_rx.blocking_recv() {
                    Ok(frame) => {
                        if let Err(e) = encoder.submit_frame(&frame) {
                            tracing::warn!("replay encoder submit_frame failed: {e}");
                            continue;
                        }
                        while let Ok(Some(packet)) = encoder.receive_packet() {
                            buffer_for_thread.lock().unwrap().push(packet);
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            })
            .expect("failed to spawn brail-replay-encode thread");

        Ok(Self { buffer, shutdown_tx })
    }

    /// Drains the current buffer window into a new file at `output_dir`.
    /// Returns the path written. The buffer keeps running unaffected —
    /// this only takes a snapshot copy, per `RingBuffer::snapshot`'s
    /// contract.
    pub fn save_replay(&self, output_dir: &std::path::Path, has_audio: bool) -> BrailResult<PathBuf> {
        let packets = self.buffer.lock().unwrap().snapshot();
        if packets.is_empty() {
            return Err(BrailError::Internal("replay buffer is empty (nothing to save yet)".into()));
        }

        let filename = format!("Replay_{}.mkv", chrono::Local::now().format("%Y-%m-%d_%H-%M-%S"));
        let path = output_dir.join(filename);

        let first_video = packets
            .iter()
            .find(|p| p.stream == brail_core::frame::StreamKind::Video)
            .ok_or_else(|| BrailError::Internal("replay buffer has no video packets".into()))?;

        let mut muxer = Muxer::create(
            &path,
            ContainerFormat::Mkv,
            first_video.codec,
            0, // width/height are informational for the muxer's stream
            0, // parameters here; real wiring reads them from the replay
               // encoder's configured InstantReplaySettings::resolution
               // rather than duplicating them onto EncodedPacket.
            60,
            has_audio,
        )?;

        for packet in &packets {
            muxer.write_packet(packet)?;
        }
        muxer.finalize()?;

        Ok(path)
    }

    pub fn buffered_duration_secs(&self) -> f64 {
        self.buffer.lock().unwrap().buffered_duration_secs()
    }

    pub fn shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }
}
