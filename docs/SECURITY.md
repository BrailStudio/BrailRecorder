# Security and privacy

## What Brail protects

The app handles three things worth protecting: **stream keys** (equivalent
to a password for your channel), **your screen** (potentially anything),
and **your microphone and webcam**.

## Stream keys

A stream key lets anyone broadcast to your channel. It is treated as a
credential throughout.

**Storage.** Keys go into Windows Credential Manager via `CredWriteW`,
which encrypts at rest under your Windows login (DPAPI). This is the same
store Windows uses for saved Wi-Fi passwords and RDP credentials. No custom
crypto — a bespoke scheme would need to justify its own safety, and there's
no reason to when the OS already provides this.

Keys are namespaced `BrailRecorder_StreamKey_<profile-uuid>`, keyed by
profile UUID so renaming a profile never orphans its key.

**Never in config.** `StreamProfile::stream_key` is `#[serde(skip)]`. Even
if a profile holds a live key in memory, serializing it produces JSON
without the key. There's a test asserting exactly this
(`config_file_never_contains_a_stream_key`).

**Never in a URL.** `parse_rtmp_url` deliberately does not accept a key
embedded in the URL, and the key is always passed separately. URLs end up
in logs and error messages; keys must not.

**Never in logs.** Two layers:

1. `SecretRegistry` redacts registered secrets at the *writer*, not the
   call site. Relying on every future `tracing::info!` to remember not to
   include a key is the kind of discipline that fails once and leaks a
   credential permanently into a log a user then attaches to a bug report.
2. `redact_known_patterns` catches key-shaped tokens (YouTube's
   dash-separated groups, Twitch's `live_` prefix) that reached a log
   before anything registered them.

Values under 8 characters are never registered — redacting a 3-character
string would mangle unrelated log text, and a secret that short isn't
protecting anything.

**Only sent to the configured endpoint.** The key goes to the ingest server
in the profile and nowhere else. There is no telemetry, no analytics, and
no other network destination in the codebase.

## Screen, microphone, webcam

Recordings are written to a local folder and stay there. Nothing uploads
without explicit action — there is no cloud storage, no account, and no
background sync. Streaming connects directly to the service you configured.

Recording works fully offline. Only streaming needs a network.

## What the app does not do

- No account or subscription requirement
- No telemetry or usage analytics
- No crash reporting to a remote service
- No auto-uploads of any kind
- No browser extensions, no unrelated software
- No GPU driver modification
- No firewall rules without explicit consent

## Installer posture

- Requests elevation only for a per-machine install; a per-user install
  needs none.
- Installs only its own files and the WebView2 runtime if missing.
- Does not modify graphics drivers or unrelated system settings.
- Does not enable start-with-Windows by default.
- Release binaries should be code-signed. Unsigned installers trigger
  SmartScreen, which matters a great deal for an app asking for screen and
  microphone access. See `docs/BUILD.md`.

## Uninstall

Default uninstall **preserves your recordings**, which are never deleted
without explicit confirmation. It also currently preserves
`%APPDATA%\BrailRecorder\` so settings survive a reinstall.

Stream keys in Credential Manager are **not** yet removed on uninstall.
That needs a custom action enumerating `BrailRecorder_StreamKey_*` and
calling `CredDeleteW` for each. Until it lands, removing them manually via
Control Panel → Credential Manager → Windows Credentials is the workaround.
This is tracked in `installer/README.md` and `docs/STATUS.md`.

## Error messages

Errors never include secrets and never show a bare `0x80004005`. Every
`BrailError` variant maps to something the user can act on — which encoder
failed, what to try, which button fixes it.

## Reporting a vulnerability

No security contact is configured yet. Add one before public distribution.
