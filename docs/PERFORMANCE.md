# Performance

Brail Recorder's whole reason to exist is using less than OBS does. This
document explains where the budget goes, which design decisions are load
bearing, and how to verify any of it yourself.

## Targets

| State | Target RAM | Notes |
|---|---|---|
| Idle | 20–50 MB | UI open, nothing capturing |
| Recording 1080p60 | under 100 MB | hardware encoder, no overlays |
| Streaming 1080p60 | under 100 MB | includes the network send buffer |
| Recording + streaming | under 130 MB | two encoders, one capture |

These are engineering targets, not guarantees. A machine with no hardware
encoder falls back to libx264, which uses substantially more of everything;
the app says so rather than quietly missing the target. Benchmark mode
(Settings → Performance → Run Benchmark) reports the real numbers on the
machine it's running on.

## Where the budget goes

At 1080p60, a single uncompressed BGRA frame is about 8 MB. Holding even a
one-second buffer of them would be 500 MB on its own, which is why the
architecture never holds raw frames:

- **Capture frames are GPU textures**, reference-counted and handed
  straight to the encoder. A frame that reaches the encoder without a CPU
  copy costs nothing in process RAM.
- **The capture channel is bounded at 8 frames** and is a broadcast
  channel: a slow consumer drops its own frames rather than causing the
  buffer to grow. Capture never stalls and never accumulates.
- **The encoded packet channel is bounded at 64 packets.** Encoded 1080p60
  packets average a few hundred KB/s, so this is a few MB at worst, and a
  stalled disk or network causes frame drops rather than unbounded growth.
- **The replay ring buffer is the one intentional allocation.** 30 seconds
  of 720p30 at 3 Mbps is roughly 11 MB of encoded data. It scales linearly
  with the configured duration, which is why the UI states that longer
  buffers cost proportionally more memory.

## The decisions that actually matter

**Zero-copy GPU path.** Windows Graphics Capture hands back a D3D11
texture. That same texture goes to the hardware encoder via FFmpeg's
D3D11VA hardware frames context. At 1080p60 a CPU round-trip would be
~500 MB/s of memory bandwidth and a large chunk of a core, purely to move
pixels that never needed to leave the GPU. This is implemented in
`brail-encoder::gpu_surface` and is the single highest-leverage thing in
the codebase.

**No preview by default at high framerates.** Rendering the captured frame
back into the UI is a second full pipeline. Preview runs at 10 FPS by
default, and Gaming Mode turns it off entirely. This is why the home screen
shows numbers rather than a live thumbnail.

**Tauri, not Electron.** The UI is a system WebView2 process, not a bundled
Chromium. That's roughly 40–60 MB rather than 150–250 MB before any
application code runs.

**The UI is not on the hot path.** The recording engine runs entirely in
Rust on its own threads. The frontend receives batched stat events at 1 Hz.
If the webview froze completely, the recording would continue uninterrupted.

**Encoder threads are OS threads, not async tasks.** Hardware encoder calls
can block on driver-internal queues. Running them as tokio tasks would stall
a runtime worker that other work depends on.

## How each subsystem earns its cost

| Subsystem | When it runs | Cost when idle |
|---|---|---|
| Capture | only while recording/streaming/replay | zero — session closed |
| Encoder | one instance per active output | zero |
| Replay buffer | continuously, if enabled | proportional to duration |
| Resource monitor | 1 Hz sampling | negligible |
| Adaptive engine | consumes existing samples | none — no extra measurement |
| Audio meters | computed during the gain pass | none — no extra walk |
| Hotkeys | one blocked thread on GetMessage | one thread, no polling |

The metering point matters: computing peak and RMS during the gain pass
that already touches every sample means metering is free, rather than a
second walk over the buffer. The same principle governs the adaptive
engine, which consumes samples the monitor already takes rather than
measuring anything itself.

## Measuring it yourself

Benchmark mode samples at 1 Hz for 30 seconds and reports mean and peak RAM,
mean and peak CPU, GPU utilization, capture FPS against target, dropped
frames, and RAM drift across the run. Drift is the leak signal — a number
that keeps growing across longer runs means something is accumulating.

For a genuine leak check, the ignored integration test runs for as long as
you tell it:

```powershell
$env:BRAIL_LEAK_TEST_MINUTES = "60"
cargo test -p brail-tests --test windows_integration -- --ignored memory_stays_bounded --nocapture
```

It compares mean RAM across the first and last quarters of the run and
fails if growth exceeds 50 MB.

## When Brail will miss its targets

Stated plainly, because the spec asks for honesty over marketing:

- **No hardware encoder.** libx264 at 1080p60 will use multiple cores.
  The app detects this, says so, and recommends 720p30.
- **Software encoding at 4K.** Not realistically sustainable. The preset
  isn't offered unless a hardware encoder verifies support for it.
- **Many overlays.** Each one is a compositing pass. One webcam overlay is
  cheap; a dozen elements is a real per-frame cost.
- **Recording and streaming at different settings.** That genuinely needs
  two encoders. The UI says so rather than hiding the cost.
- **Long replay buffers at high resolution.** Five minutes of 1080p60 is
  hundreds of MB of encoded data, by definition.
