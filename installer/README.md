# Installer customization

Tauri's NSIS bundler accepts a custom template, which is how the wizard in
§42 of the spec gets built rather than shipping the generic default. Point
`tauri.conf.json` at one:

```json
"nsis": {
  "installMode": "both",
  "template": "../installer/installer.nsi",
  "headerImage": "../installer/header.bmp",
  "sidebarImage": "../installer/sidebar.bmp",
  "installerIcon": "../src-tauri/icons/icon.ico",
  "languages": ["English"]
}
```

Start from Tauri's default template (`nsis/installer.nsi` in the
`tauri-bundler` crate) and modify it rather than writing from scratch —
the default handles WebView2 bootstrapping, upgrade detection and
uninstall registration correctly, and those are easy to get subtly wrong.

## What the spec asks for, and how each part maps to NSIS

| Requirement | Mechanism |
|---|---|
| Branded welcome screen with tagline | `MUI_WELCOMEPAGE_TITLE` / `_TEXT`, `MUI_WELCOMEFINISHPAGE_BITMAP` |
| Custom install directory with Browse | `MUI_PAGE_DIRECTORY` (default) |
| Free-space check | `${DriveSpace}` against `$INSTDIR`'s drive, before copy |
| Component selection | `MUI_PAGE_COMPONENTS` with `SectionIn RO` for required parts |
| Hardware detection display | custom page calling `nsisdxdiag` or a small bundled probe |
| Real progress | default `MUI_PAGE_INSTFILES` — it reflects actual file copy |
| Expandable details | `MUI_INSTFILESPAGE_` with `ShowInstDetails show` |
| Optional file association | a non-required `Section` writing `HKCR` entries |
| Launch on finish | `MUI_FINISHPAGE_RUN` |
| Retry / Cancel / View Details on failure | `IfErrors` with a custom `MessageBox` |
| Repair and Modify | `MUI_PAGE_COMPONENTS` on re-run, detected via the uninstall registry key |

## Components

Sections matching the spec's list. Core and Audio are `RO` (read-only,
always installed):

```nsis
Section "Brail Recorder Core" SecCore
  SectionIn RO
SectionEnd

Section "Hardware Encoding Support" SecHwEnc
SectionEnd

Section "Streaming Support" SecStreaming
SectionEnd

Section "Webcam Support" SecWebcam
SectionEnd

Section "Audio Support" SecAudio
  SectionIn RO
SectionEnd

Section "Replay Support" SecReplay
SectionEnd
```

## Hardware detection page

Show what was detected, in the format the spec gives:

```
✓ Windows 11 64-bit
✓ NVIDIA GPU detected
✓ NVENC available
✓ Hardware acceleration available
```

Detect GPU vendor by reading `HKLM\SYSTEM\CurrentControlSet\Control\Class\
{4d36e968-e325-11ce-bfc1-08002be10318}\0000\ProviderName`. Report only what
you actually read — a missing value means the line shows an informational
note, never a fabricated ✓.

**Never modify graphics drivers.** Detection is read-only.

## Elevation

`installMode: "both"` lets the user choose per-user (no elevation) or
per-machine (elevation). Don't force `RequestExecutionLevel admin` — the
spec is explicit about not requesting privileges unnecessarily.

## Uninstall

Default uninstall **must preserve recordings**. Offer removal of settings
and profiles as explicit, unchecked options:

```nsis
Section /o "Remove settings and profiles" SecRemoveSettings
  RMDir /r "$APPDATA\BrailRecorder"
SectionEnd
```

Recordings live in `Videos\Brail Recorder` and should never be touched
without a separate, explicitly confirmed prompt.

**Still needed:** stream keys in Credential Manager are not yet removed on
uninstall. That requires a custom action enumerating
`BrailRecorder_StreamKey_*` and calling `CredDeleteW` on each — either a
small NSIS plugin or a `--cleanup-credentials` flag on the main binary that
the uninstaller invokes. The second option is simpler and reuses
`brail-security`'s existing vault code.

## Portable build

The spec also asks for `Brail-Recorder-Portable-x64.exe`. Tauri doesn't
produce one directly; the practical approach is zipping the built binary
with its WebView2 loader and adding a `--portable` flag that makes
`ConfigStore` resolve its directory next to the executable instead of
`%APPDATA%`. That's a small change to `brail-storage::config_store::config_dir`
and is not yet implemented.

## Signing

An unsigned installer triggers SmartScreen, which matters a great deal for
an app requesting screen and microphone access. See the signing section in
`docs/BUILD.md`.
