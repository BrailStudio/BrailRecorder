# Architecture

## Pipeline shape

```
                         ┌──────────────────┐
                         │  brail-hardware   │  runs once at startup,
                         │ (CPU/GPU/monitor/ │  produces a CapabilityProfile
                         │  encoder probing) │  everything else trusts
                         └────────┬─────────┘
                                  │
                                  v
┌───────────────┐        ┌──────────────────┐
│ brail-capture  │──────▶│  broadcast::      │  every consumer subscribes
│ (WGC session,  │ frames│  Sender<VideoFrame>│ independently; a slow
│  D3D11 device) │        │  (CaptureHandle)  │  consumer drops its own
└───────────────┘        └────────┬─────────┘  frames, never backpressures
                                   │              capture itself
                 ┌─────────────────┼─────────────────┐
                 v                 v                 v
        ┌────────────────┐ ┌────────────────┐ ┌────────────────┐
        │ brail-encoder   │ │ brail-encoder   │ │ brail-replay    │
        │ (recording      │ │ (streaming      │ │ (own encoder +  │
        │  EncodeController)│ EncodeController)│  ring buffer)   │
        └───────┬─────────┘ └───────┬─────────┘ └────────────────┘
                 v                   v
        ┌────────────────┐ ┌────────────────┐
        │ brail-encoder:: │ │ brail-streaming │
        │ Muxer (MKV/MP4) │ │ (RTMP/SRT)      │
        └────────────────┘ └────────────────┘
```

Three independent encoder instances can run simultaneously (main
recording, streaming, instant replay), each subscribing to the same
capture broadcast channel. This is deliberate: they have different
resolution/bitrate/quality needs (per `RecordingSettings` vs
`StreamProfile` vs `InstantReplaySettings`), and coupling them to a single
shared encoder would mean every feature compromises on the others'
settings.

## Why these specific technology choices

- **Windows Graphics Capture over Desktop Duplication**: per-window
  capture, OS-composited cursor toggle, HDR awareness. Desktop Duplication
  remains as a documented fallback (`brail-capture::duplication`) for
  pre-Windows-10-2004 systems that don't have WGC's newer APIs.
- **FFmpeg for all encoding, not three vendor SDKs**: one integration
  surface for NVENC/AMF/QSV/software instead of three, at the cost of not
  exposing every vendor-specific tuning knob. See
  `brail-encoder/src/lib.rs`'s module doc for the full trade-off
  reasoning.
- **MKV as the default recording container, not MP4**: MKV's cluster-based
  format means a file is playable up to the last flushed cluster even if
  the app never gets to write a proper trailer (crash, power loss). MP4
  needs a finalized `moov` atom to be playable at all. MP4 is offered as a
  post-recording remux (fast, lossless stream-copy — see
  `brail-encoder::muxer::remux`), never as the live recording target.
- **One `AppEvent` channel, not one Tauri event per message type**: a
  single frontend listener, one place to log every backend event for
  diagnostics, and no risk of the frontend forgetting to subscribe to a
  rarely-used channel.
- **Per-engine `Mutex`es in `AppState`, not one big lock**: polling
  hardware/resource stats should never block on a recording start/stop in
  progress, and vice versa.

## Threading model

- **Capture**: Windows Graphics Capture delivers frames via a callback on
  its own free-threaded frame pool thread. That callback only forwards the
  frame into a broadcast channel — it never blocks on encoding.
- **Encoding**: each `EncodeController` runs on its own dedicated OS
  thread (not a tokio task), because hardware encoder calls can briefly
  block on driver-internal queues in a way that would stall an async
  runtime worker.
- **Muxing/streaming I/O**: runs as tokio tasks consuming from an `mpsc`
  channel the encoder thread feeds — file/network I/O is kept off the
  encode thread's real-time loop.
- **Hotkeys**: `RegisterHotKey`/`WM_HOTKEY` require a thread with a real
  Win32 message loop, so `brail-hotkeys` gets its own dedicated thread for
  exactly that, forwarding hotkey presses out through a channel.

## Error handling philosophy

Every subsystem's errors get mapped into `brail_core::BrailError` — a
closed set of variants the frontend can render directly without ever
seeing a raw `HRESULT` or FFmpeg errno. Recoverable conditions (a
hardware encoder isn't available, upload bandwidth dropped) are modeled as
degradation the relevant controller (`AdaptiveBitrateController`,
`SelfTuner`) reacts to automatically, always surfaced to the user via an
event rather than silently changing behavior underneath them.
