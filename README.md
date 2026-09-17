# Brail Recorder

**Record. Stream. Replay.**

A lightweight Windows screen recorder and live-streaming app, built to use
meaningfully less memory than OBS while covering the same core workflow.
Hardware-accelerated encoding on NVIDIA, AMD and Intel, real RTMP/RTMPS
streaming to YouTube and elsewhere, instant replay, and an interface built
around three buttons instead of a mixing desk.

> **Read [docs/STATUS.md](docs/STATUS.md) first.** It's a plain accounting
> of what's complete, what's stubbed, and what needs verifying on real
> Windows hardware. This project was written without access to a Windows
> machine, so that distinction matters.

## Features

**Recording** — 144p through 4K at 30/60/90/120 FPS, custom resolutions,
monitor / window / region capture, multi-monitor with mixed DPI and refresh
rates, optional cursor capture and click highlighting.

**Encoding** — NVENC, AMD AMF, Intel Quick Sync, and an optimized software
fallback. H.264, HEVC and AV1 where hardware supports them. Encoders are
verified by actually opening them, not inferred from the GPU model.

**Streaming** — YouTube, Twitch, Facebook and custom RTMP/RTMPS/SRT
endpoints. Stream keys stored encrypted. A TEST CONNECTION button that
performs a real handshake against the ingest server. Automatic reconnect
with backoff, live stream health, and adaptive bitrate that announces
every change rather than silently degrading quality.

**Instant Replay** — 15s / 30s / 1m / 2m / 5m rolling buffer, saved with a
hotkey, without interrupting the buffer or writing huge permanent files.

**Audio** — WASAPI desktop and microphone capture, per-source volume, mute
and gain, live meters, optional separate tracks.

**Screenshots** — PNG or JPEG, full screen, a specific monitor, or a region.

**Brail Adaptive Engine** — monitors CPU, GPU, RAM, encoder load, capture
FPS, disk and network, and recommends (or, if you let it, applies) settings
changes when the system genuinely can't keep up. Six modes from Ultra Lite
to Quality, plus Custom, which only ever warns.

**Reliability** — records to MKV so a crash leaves a playable file, then
remuxes to MP4 losslessly. Unfinished recordings are detected and recovered
on the next launch.

## Requirements

- Windows 10 build 19041 (2004) or later, or Windows 11, 64-bit
- WebView2 runtime (preinstalled on Windows 11 and most Windows 10)
- A GPU with NVENC, AMF or Quick Sync is strongly recommended but not
  required — software encoding always works

## Installation

Run `Brail-Recorder-Setup-x64.exe` and follow the wizard. It checks your
Windows version and GPU, installs only what you select, and does not enable
start-with-Windows by default.

To build from source, see [docs/BUILD.md](docs/BUILD.md).

## Recording guide

1. Pick a source in Settings, or accept the detected primary display.
2. Press **RECORD** (or `Ctrl+Shift+F9` from anywhere).
3. Press it again to stop. Files land in `Videos\Brail Recorder`.

The mode line under the title shows what's actually being recorded, and the
line under it shows the encoder genuinely in use — if it says
`Software (CPU)`, no hardware encoder was verified on your system.

**Pause** with `Ctrl+Shift+F8`. Pausing keeps the same file open, so
resuming continues one recording rather than starting a second.

## YouTube streaming guide

1. Open YouTube Studio → **Go Live** → **Stream**.
2. Copy the **Stream key** (not the URL — Brail fills that in).
3. In Brail: **STREAM** → Platform: YouTube → paste the key.
4. Pick a quality. Only presets your hardware verified are listed.
5. Press **TEST CONNECTION**. This performs a real handshake and confirms
   the key is accepted. No video is sent, so it won't create a stray
   broadcast on your channel.
6. Press **START STREAM**, then start the broadcast in YouTube Studio.

Your key is stored encrypted in Windows Credential Manager, never written
to a config file, never included in a URL, and redacted from logs.

## Stream key setup for other services

| Service | Where to find the key | Notes |
|---|---|---|
| YouTube | Studio → Go Live → Stream | Server prefilled |
| Twitch | Creator Dashboard → Settings → Stream | Bitrate capped at 6000 kbps |
| Facebook | Live Producer → Streaming software | Uses RTMPS |
| Custom | your server's docs | RTMP, RTMPS or SRT |

## Performance modes

| Mode | Prioritizes |
|---|---|
| Ultra Lite | Minimum resource usage. Preview off. |
| Low-End | Game performance over recording quality. |
| Balanced | Quality against system impact. Default. |
| Quality | Video quality. Uses more CPU and GPU. |
| Streaming | Stable delivery over peak quality. |
| Custom | Your settings. The engine only warns, never changes. |

**Gaming Mode** additionally minimizes preview rendering and UI work while
capturing a game.

## Low-end PC recommendations

- Start at **720p30** and move up only if Performance reads Good.
- Use a hardware encoder if you have one — it's the single biggest
  difference, far more than any other setting.
- Turn off the preview (Settings → Performance → Preview FPS: 0).
- Leave Instant Replay off unless you need it; it runs a second encoder
  continuously.
- Choose **Low-End** mode, which protects game framerate over recording
  quality and intervenes much earlier than the other modes.

## Troubleshooting

**"Software (CPU)" when you have a GPU.** The encoder probe couldn't open
your hardware encoder. Usually the FFmpeg build lacks nvcodec/amf/qsv
support (see BUILD.md), or another application is holding the encoder.
Close other recording software and re-detect in Settings.

**Stream key rejected.** Keys are regenerated whenever you reset them in
your channel dashboard. Copy it again. TEST CONNECTION distinguishes a
rejected key from an unreachable server.

**Dropped frames while streaming.** Check the Performance line. High CPU
means the encoder is the bottleneck — lower the preset or resolution.
Normal CPU with drops means upload bandwidth; lower the bitrate.

**Recording stutters in a game.** Switch to Low-End mode and turn off the
preview. If it persists, lower recording FPS before resolution — halving
framerate costs less visual quality than halving resolution.

**Recording seems to have failed.** MKV files stay playable even if the
app never finalized them. Brail also detects unfinished recordings on the
next launch and offers to recover them.

**App won't start.** Install the
[WebView2 runtime](https://developer.microsoft.com/microsoft-edge/webview2/).

## Hardware encoder support

| Vendor | Encoder | H.264 | HEVC | AV1 |
|---|---|---|---|---|
| NVIDIA | NVENC | GTX 600+ | GTX 950+ | RTX 40 series |
| AMD | AMF | GCN 2+ | Polaris+ | RX 7000 series |
| Intel | Quick Sync | HD 4000+ | HD 515+ | Arc / 11th gen+ |
| — | Software | always | always | slow |

Brail never relies on this table at runtime. It opens each encoder to
confirm it actually works, because driver and SDK mismatches are common
enough that inference alone would mean advertising encoders that fail at
record time.

## Documentation

| Document | Contents |
|---|---|
| [STATUS.md](docs/STATUS.md) | What's real, stubbed, and unverified |
| [ARCHITECTURE.md](docs/ARCHITECTURE.md) | Pipeline, threading, design decisions |
| [PERFORMANCE.md](docs/PERFORMANCE.md) | Memory budget and how to measure it |
| [STREAMING.md](docs/STREAMING.md) | Protocols, reconnect, adaptive bitrate |
| [SECURITY.md](docs/SECURITY.md) | Credential handling and privacy |
| [DEVELOPMENT.md](docs/DEVELOPMENT.md) | Conventions and how to contribute |
| [BUILD.md](docs/BUILD.md) | Prerequisites and troubleshooting |

## License

Not yet decided — add one before distributing.
