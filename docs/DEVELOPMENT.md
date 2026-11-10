# Development

Build prerequisites are in [BUILD.md](BUILD.md). Current state of every
subsystem — what's real, what's stubbed, what's unverified — is in
[STATUS.md](STATUS.md), which is the first thing to read.

## Workspace layout

Twelve crates plus the Tauri shell. The dependency graph is a star:
everything depends on `brail-core`, and nothing else depends on anything
else. That's what lets the UI process link against just the config types
instead of pulling in Direct3D, FFmpeg and RTMP.

```
brail-core         shared types — frames, config, events, errors, presets
brail-hardware     CPU/GPU/monitor/audio/encoder detection
brail-capture      WGC, Desktop Duplication fallback, screenshots, overlays
brail-encoder      NVENC/AMF/QSV/software, muxing, bitrate control
brail-audio        WASAPI, resampling, mixing, controls, AAC
brail-streaming    RTMP/RTMPS/SRT, FLV muxing, reconnect, connection testing
brail-replay       instant-replay ring buffer
brail-performance  resource monitoring, Adaptive Engine, benchmark mode
brail-storage      config persistence, disk space, output paths
brail-security     credential vault, log redaction
brail-hotkeys      global hotkeys on a dedicated message-loop thread
brail-recovery     crash detection and recording recovery
src-tauri          commands, events, app state
ui                 TypeScript frontend, no framework
```

## Conventions

**Where a change goes.** A new capture source is `brail-capture`. A new
codec is a line in `ffmpeg_encoder.rs`'s lookup table plus any private
options. A new streaming service is a `StreamingService` variant plus its
ingest URL and bitrate rules. If a change touches three crates, the
abstraction is probably in the wrong place — say so rather than working
around it.

**Errors.** Every subsystem maps its errors into `BrailError`. The frontend
should never see an `HRESULT` or FFmpeg errno. Each variant's message is
written for a user, not a developer.

**Events.** One channel, `brail://event`, carrying the `AppEvent` enum.
Adding an event means a variant in `brail-core::events` and a case in
`ui/src/lib/events.ts`. One channel means one frontend listener and one
place to log everything for diagnostics.

**Threading.** Anything that can block on a driver gets an OS thread, not a
tokio task. Anything doing file or network I/O is a tokio task consuming
from a bounded channel. Capture callbacks only forward to a channel — they
must never block.

**Bounded everything.** Every channel has a capacity. A full channel drops
frames; it never grows. If you add a buffer, state its bound and what
happens when it's hit.

**No fake functionality.** If something can't be implemented, detect the
limitation, handle it, explain it in the UI, provide a fallback, and
document it. A stub gets a comment saying exactly what's missing and gets
an entry in STATUS.md. Never a silent no-op behind a working-looking
button.

## Testing

```bash
cargo test -p brail-tests                    # pure logic, runs anywhere
cargo test --workspace                       # includes unit tests in crates
```

```powershell
cargo test -p brail-tests --test windows_integration -- --ignored --nocapture
```

The split is deliberate. Capture, encoding, WASAPI and RTMP need real
hardware or a real server; mocking them would only prove the mock works.
Everything with genuine logic and no platform dependency — presets, bitrate
derivation, ring buffer GOP handling, backoff, overlay layout, the adaptive
engine's rules — is tested and runs anywhere.

Tests needing credentials read them from the environment and skip rather
than fail when unset, so a contributor without a stream key can run
everything else.

## Adding a setting

1. Field on the relevant struct in `brail-core::config` or `::settings`,
   with a default.
2. Mirror the type in `ui/src/lib/api.ts`.
3. Control in the matching `renderSettingsTab()` case in `ui/src/main.ts`.
4. Bind it in `attachHandlers()` via `bindSwitch` or an explicit listener.

Config saves atomically (temp file, then rename), so a crash mid-save can't
corrupt `config.json`. A config that fails to parse is backed up to
`config.json.corrupt` and defaults are used — a bad config must never stop
the app from starting.

## Working on this without Windows

Most of the logic — presets, bitrate maths, ring buffer, backoff, overlay
layout, adaptive rules, redaction, audio controls — is platform independent
and tested on any OS. The Windows-specific layers need a real machine, and
[STATUS.md](STATUS.md) lists exactly which files those are and in what
order to tackle them.
