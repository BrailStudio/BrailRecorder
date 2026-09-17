# Build status

This was written in a Linux sandbox with no Windows machine, no network
access, and no GPU. Nothing here has been compiled or run. Every file is
real, intentional code against the actual Windows/FFmpeg/RTMP APIs — not
pseudocode — but "written carefully" and "compiles and works on real
hardware" are different claims, and only the first has been checked.

That is the honest gap between this and what the spec asked for. The spec
said build it, run it, and test it; I could do the first. This document is
the map of what the remaining two require.

## Complete and self-contained

These have no platform dependency, are covered by tests that run anywhere,
and should need no rework:

- Quality presets, YouTube profile generation, capability filtering
- Bitrate derivation (resolution, framerate, codec, service caps)
- Encoder selection, including hardware-over-software preference
- Smart defaults from detected hardware
- Instant replay ring buffer, including GOP-boundary eviction
- Reconnect backoff with overflow safety
- Overlay and webcam layout maths, including NV12 even-dimension rounding
- Region clamping
- Brail Adaptive Engine decision rules
- Audio gain, mute, clipping limit, and level metering
- Log redaction (registered secrets and pattern matching)
- Benchmark report computation
- Atomic config persistence with corrupt-file recovery

## Structurally complete, needs compiling against real APIs

The full pipeline exists end to end. What these need is a Windows compiler
against real SDK headers — expect a bounded number of signature fixes, not
a rewrite:

- **`brail-encoder::gpu_surface`** — the D3D11VA hardware-frames-context
  FFI bridge. The trickiest unsafe code here and the most likely to need
  field-layout fixes against your exact FFmpeg version. **Do this first:**
  it's the zero-copy GPU path, and without it the memory and CPU targets
  are not reachable at 1080p60.
- **`brail-streaming::rtmp_client`** — written against `rml_rtmp`'s session
  API from memory. Check `ClientSession` / `ClientSessionResult` variants
  against your pinned version.
- **`brail-hardware`** (audio_devices, cameras, monitors) — COM/WinRT
  interop that may need small signature fixes against `windows` 0.58.
- **`brail-capture::wgc`, `::screenshot`** — the WGC interop factory
  pattern and staging-texture readback.
- **`brail-security::vault`** — `CREDENTIALW` field layout.

## Explicitly incomplete

Each is marked in-code with a comment saying exactly what's missing:

| What | State |
|---|---|
| `brail-audio::resampler::resample` | Returns input unchanged. Mixing sources at different rates will sound wrong until swresample is wired. |
| GPU frame submission in `ffmpeg_encoder::submit_frame` | Every frame takes the CPU path regardless of payload type. Correct, but misses the performance target. |
| `ffmpeg_encoder::set_bitrate` | Returns `Unsupported`. Adaptive bitrate decisions currently have nowhere to land; needs per-vendor FFI reconfigure calls. |
| Audio path in `start_recording_impl` | Every piece exists in `brail-audio` but isn't threaded into the recording command yet. Video-only today. |
| Overlay compositing | Layout maths done and tested; the GPU compositing pass itself isn't written. |
| Webcam frame dimensions | Hardcoded 640×480 instead of reading the negotiated media type. |
| SRT publish | Client is a shell. RTMP is real. Deliberately last per the spec's own service priority. |
| `monitors::friendly_name_for_device` | Falls back to `\\.\DISPLAY1` instead of the EDID name; needs `QueryDisplayConfig`. |
| `monitor::system_cpu_percent` | Returns `None`; needs the delta-sampling the per-process sampler already has. |
| Streaming without recording | `start_streaming` requires an existing capture session and errors otherwise. |
| Remaining hotkey actions | Record and replay are wired; streaming, mic mute, webcam and screenshot follow the same pattern. |
| Credential cleanup on uninstall | Needs an NSIS custom action or a `--cleanup-credentials` flag. |
| Portable build | Needs a `--portable` flag changing where `ConfigStore` resolves its directory. |
| Installer wizard | `tauri.conf.json` produces working MSI and NSIS installers, but the branded multi-page wizard from §42 needs the custom NSIS template mapped out in `installer/README.md`. Icons are also still missing. |

## What to verify on a Windows machine

1. **`cargo check --workspace` first.** Most of the risk is exact API
   surface, which only a real compiler against real headers can catch.
2. **`cargo test -p brail-tests`** — the pure-logic suite should pass
   immediately. If it doesn't, that's a real logic bug, not an environment
   problem.
3. **Encoder verification on your GPU.** The probe genuinely opens each
   encoder, so `verified: false` for something you know works is a real
   signal — usually FFmpeg built without nvcodec/amf/qsv.
4. **RTMP against a real ingest.** Use `test_connection` with your own key
   before any real broadcast.
5. **The installer.** Needs real icons (`src-tauri/icons/` is empty; run
   `cargo tauri icon`) and, for release, a code-signing certificate.

## Suggested order

1. `cargo check --workspace` clean.
2. Software encoding recording to MKV end to end — exercises capture →
   encode → mux with no hardware variables. Fastest path to "it records."
3. Wire the audio path into `start_recording_impl`.
4. One hardware encoder verified and recording.
5. RTMP streaming against a real test ingest.
6. GPU zero-copy path — optimize a working pipeline rather than debugging
   an unverified one.
7. Overlay compositing, then the installer wizard.
