# Building on Windows

Everything below assumes Windows 10 (2004/build 19041+ for full Windows
Graphics Capture support) or Windows 11, x64.

## Prerequisites

1. **Rust**, stable channel, MSVC toolchain (not GNU):
   ```powershell
   winget install Rustlang.Rustup
   rustup default stable-x86_64-pc-windows-msvc
   ```

2. **Visual Studio Build Tools** (C++ workload) — required both for the
   MSVC linker and because `ffmpeg-next`'s build script needs a working
   C compiler to generate bindings via `bindgen`/`clang`.
   ```powershell
   winget install Microsoft.VisualStudio.2022.BuildTools
   # In the installer, select "Desktop development with C++"
   ```

3. **LLVM/Clang** — `bindgen` (used transitively by `ffmpeg-next`) needs
   `libclang`:
   ```powershell
   winget install LLVM.LLVM
   # Then set, so bindgen can find it:
   $env:LIBCLANG_PATH = "C:\Program Files\LLVM\bin"
   ```

4. **FFmpeg development libraries** — `ffmpeg-next` links against real
   FFmpeg; you need the `dev` package (headers + `.lib` files), not just
   the runtime `ffmpeg.exe`. The simplest path on Windows is vcpkg:
   ```powershell
   git clone https://github.com/microsoft/vcpkg
   .\vcpkg\bootstrap-vcpkg.bat
   .\vcpkg\vcpkg install ffmpeg[nvcodec,amf,qsv]:x64-windows
   $env:FFMPEG_DIR = "$(Resolve-Path .\vcpkg\installed\x64-windows)"
   ```
   The `nvcodec`/`amf`/`qsv` vcpkg features are what actually pull in
   NVENC/AMF/QSV support in the FFmpeg build — without them you'll get a
   software-only FFmpeg and every hardware-encoder probe will correctly
   report unsupported.

5. **Node.js 18+** for the frontend build.

6. **Tauri CLI**:
   ```powershell
   cargo install tauri-cli --version "^2.0"
   ```

## Building

```powershell
cd ui
npm install
cd ..

cargo tauri dev      # dev build, hot-reloads the frontend
cargo tauri build    # release build; produces .msi and NSIS .exe under
                      # src-tauri/target/release/bundle/
```

## First-build troubleshooting

- **`bindgen` fails to find `libclang`**: double-check `$env:LIBCLANG_PATH`
  points at the directory containing `libclang.dll`, not the LLVM install
  root.
- **Linker errors about missing `avcodec.lib` etc.**: `$env:FFMPEG_DIR`
  needs to point at the vcpkg `x64-windows` triplet's install directory
  (containing `lib/` and `include/`), and that env var needs to be set in
  the same shell you run `cargo build`/`cargo tauri build` from — it
  doesn't persist across shell sessions unless you add it to your profile
  or system environment variables.
- **`windows` crate feature errors**: if you bump the `windows` crate
  version in `Cargo.toml`, re-check every feature name listed in the
  workspace `Cargo.toml` — feature names occasionally get renamed between
  minor versions.
- **Runtime: "WebView2 not found"**: install the
  [Evergreen WebView2 Runtime](https://developer.microsoft.com/microsoft-edge/webview2/)
  — present by default on Windows 11 and most updated Windows 10 machines,
  but not guaranteed on a clean VM you're testing with.
- **Hardware encoder always reports `verified: false`**: check that your
  FFmpeg build actually has the relevant encoder compiled in —
  `ffmpeg -encoders | findstr nvenc` (or `amf`/`qsv`) from the vcpkg-built
  `ffmpeg.exe` should list it. If it's missing, the vcpkg feature flags in
  step 4 above didn't take, usually because the GPU vendor's SDK headers
  (e.g. the NVIDIA Video Codec SDK for `nvcodec`) weren't available at
  vcpkg build time — vcpkg's build log will say which dependency it
  couldn't satisfy.

## Code signing (for a real release)

An unsigned `.msi`/`.exe` will trigger Windows SmartScreen warnings.
`tauri.conf.json`'s `bundle.windows` section supports a
`certificateThumbprint` + `digestAlgorithm` for signing with a certificate
already installed in the Windows certificate store — see
[Tauri's Windows signing docs](https://tauri.app/distribute/sign/windows/)
for the full setup once you have a code-signing certificate.
