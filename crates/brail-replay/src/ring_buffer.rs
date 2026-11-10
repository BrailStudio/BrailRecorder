use std::collections::VecDeque;

use brail_core::frame::{EncodedPacket, StreamKind};

/// Holds up to `window_100ns` worth of encoded packets, oldest evicted
/// first. Eviction only removes video packets up to (and stopping at) the
/// next keyframe boundary — cutting a buffer mid-GOP would leave the saved
/// clip's earliest frames undecodable, since every frame in a GOP depends
/// on its keyframe. Audio packets in the evicted span are dropped
/// alongside their corresponding video span so a saved clip's audio track
/// doesn't start before its video track.
pub struct RingBuffer {
    packets: VecDeque<EncodedPacket>,
    window_100ns: i64,
}

impl RingBuffer {
    pub fn new(window_seconds: u32) -> Self {
        Self {
            packets: VecDeque::new(),
            window_100ns: window_seconds as i64 * 10_000_000,
        }
    }

    pub fn push(&mut self, packet: EncodedPacket) {
        self.packets.push_back(packet);
        self.evict_old();
    }

    fn evict_old(&mut self) {
        let Some(newest_pts) = self.packets.back().map(|p| p.pts_100ns) else {
            return;
        };
        let cutoff = newest_pts - self.window_100ns;

        // Find the eviction boundary: the latest video keyframe at or
        // before `cutoff`. Everything strictly before that keyframe is
        // safe to drop; everything from it onward must stay so the buffer
        // always starts on a decodable boundary.
        let mut boundary_index = 0;
        for (i, p) in self.packets.iter().enumerate() {
            if p.stream == StreamKind::Video && p.is_keyframe && p.pts_100ns <= cutoff {
                boundary_index = i;
            }
            if p.pts_100ns > cutoff {
                break;
            }
        }

        if boundary_index > 0 {
            self.packets.drain(..boundary_index);
        }
    }

    /// Snapshots the current buffer contents in presentation order, ready
    /// to hand to a `Muxer`. This is a copy (the buffer keeps running
    /// immediately after this returns), not a drain — instant replay must
    /// keep recording the *next* clip's window uninterrupted.
    pub fn snapshot(&self) -> Vec<EncodedPacket> {
        self.packets.iter().cloned().collect()
    }

    pub fn buffered_duration_secs(&self) -> f64 {
        match (self.packets.front(), self.packets.back()) {
            (Some(first), Some(last)) => (last.pts_100ns - first.pts_100ns) as f64 / 10_000_000.0,
            _ => 0.0,
        }
    }
}
