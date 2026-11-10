# Streaming

## Pipeline

```
capture (WGC) → GPU texture → hardware encoder → encoded packets
    → FLV tag muxing → RTMP chunk framing → TCP/TLS → ingest
```

No stage copies a video frame to the CPU. Packets reach the network as
encoded data that was never decompressed.

## Protocol support

| Protocol | Status | Used for |
|---|---|---|
| RTMP | implemented | YouTube, Twitch, Facebook, custom servers |
| RTMPS | implemented (RTMP over TLS) | Facebook, anything requiring TLS |
| SRT | scaffolded, not complete | custom low-latency contribution |

SRT is deliberately last. The spec ranks YouTube highest and SRT as
"where practical"; RTMP covers every named service. The SRT client is a
real shell with a documented gap rather than something that claims to work.

## FLV muxing and codec support

RTMP transports FLV tags. H.264 uses the classic FLV codec ID 7. HEVC and
AV1 have no classic FLV codec ID at all, so they use the Enhanced RTMP
extension — a `fourcc`-based header both YouTube and Twitch's newer ingest
support. `brail-core::config::StreamProfile` validation is responsible for
refusing a codec/service combination the service can't actually accept,
before a connection is ever attempted.

## Connection testing

The TEST CONNECTION button performs a real connect → handshake → connect →
publish → disconnect cycle. It proves three things a ping cannot:

1. the server is reachable,
2. the RTMP handshake negotiates (the URL really is an RTMP ingest),
3. **the stream key is accepted** — by far the most common real failure.

No media is sent, so testing never produces a stray empty broadcast on
your channel. Failures are explained specifically: a rejected key, an
unreachable host, and a URL that isn't an RTMP endpoint each produce a
different message with a different suggested fix.

## Reconnection

Disconnects are detected at the socket layer and trigger exponential
backoff: 1s, 2s, 4s, 8s… capped at 30 seconds, up to 10 attempts by
default. All settings are preserved across a reconnect — the spec is
explicit that a reconnect must not silently change what the user
configured.

Every state change is reported: `StreamConnecting`, `StreamConnected`,
`StreamReconnecting { attempt, max_attempts }`, `StreamDisconnected
{ reason }`. The UI shows the attempt count so a reconnect in progress is
visibly different from a dead stream.

## Adaptive bitrate

Two independent controllers operate at different timescales, because
network and system problems develop at different speeds:

**`brail-encoder::bitrate::AdaptiveBitrateController`** reacts in seconds
to upload shortfall or sustained frame drops, walking a ladder: reduce
bitrate 25% → reduce 50% → halve framerate → drop one resolution rung. It
requires 5 seconds of sustained trouble before stepping down, and 20
seconds of sustained recovery before stepping back up. That asymmetry is
deliberate — flapping between qualities is worse for viewers than sitting
one rung low for an extra few seconds.

**`brail-performance::AdaptiveEngine`** works over a 12-second window on
system-level signals: CPU, GPU, capture FPS, encoder saturation, disk
throughput. It reports what it wants to change and why; whether that is
applied depends on the user's mode, and `AdaptiveMode::Custom` never
auto-applies anything.

Both announce every change. Neither silently degrades quality.

## Bitrate defaults

Derived from resolution, framerate, codec, and service rather than
hardcoded. HEVC gets roughly 75% of the H.264 rate, AV1 roughly 65%, since
both achieve comparable quality at lower bitrates. Higher framerates get
1.5x rather than 2x — motion between frames is smaller at 60fps, so
inter-frame prediction is more efficient.

Twitch is clamped to 6000 kbps because its non-partner ingest rejects or
badly transcodes above that. Offering 4K-grade bitrate there would be a
broken default, not a generous one.

## Stream key handling

- Stored via Windows Credential Manager (DPAPI-backed), never in a file.
- `StreamProfile::stream_key` is `#[serde(skip)]`, so it cannot reach
  `config.json` even accidentally.
- Never concatenated into a URL, because URLs get logged.
- The logging layer redacts registered secrets at the writer, plus a
  pattern-matching pass for key-shaped tokens that were never registered.

See SECURITY.md for the full threat model.

## Stream + Record

Two modes:

**Shared encode** (`stream_and_record: true`): one encoder, packets routed
to both the RTMP client and the muxer. Almost free — the second consumer
costs a channel send. Only available when stream and recording settings
match.

**Separate encodes**: two encoder instances when the settings differ (e.g.
streaming 1080p but archiving 1440p). The UI states the additional cost
rather than hiding it.
