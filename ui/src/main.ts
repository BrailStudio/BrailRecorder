import "./style.css";
import { api, type AppConfig, type CapabilityProfile, type TestResult } from "./lib/api";
import { onAppEvent } from "./lib/events";

/* ------------------------------------------------------------------
   The spec (§29) asks for an interface built around three primary
   actions — RECORD, STREAM, REPLAY — that is substantially simpler
   than OBS. So the home screen is exactly that: one large record
   control, two secondary actions, the source toggles, and a single
   plain-language performance verdict. Everything else lives behind
   the gear, and Advanced controls only appear in Advanced mode.
   ------------------------------------------------------------------ */

type Screen = "home" | "settings" | "streaming-setup";
type SettingsTab =
  | "general"
  | "recording"
  | "streaming"
  | "replay"
  | "audio"
  | "performance"
  | "hotkeys";

interface UiState {
  screen: Screen;
  settingsTab: SettingsTab;
  config: AppConfig | null;
  hardware: CapabilityProfile | null;
  selectedMonitorIndex: number;
  isRecording: boolean;
  isStreaming: boolean;
  isPaused: boolean;
  startedAt: number | null;
  stats: {
    cpu: number;
    gpu: number | null;
    ram: number;
    captureFps: number;
    uploadKbps: number;
    droppedPercent: number;
    connectionState: string;
  };
  micLevel: number;
  desktopLevel: number;
  activeEncoderLabel: string;
  warnings: { code: string; message: string; severity: string }[];
  testResult: TestResult | null;
  testing: boolean;
  toast: string | null;
}

const state: UiState = {
  screen: "home",
  settingsTab: "general",
  config: null,
  hardware: null,
  selectedMonitorIndex: 0,
  isRecording: false,
  isStreaming: false,
  isPaused: false,
  startedAt: null,
  stats: {
    cpu: 0,
    gpu: null,
    ram: 0,
    captureFps: 0,
    uploadKbps: 0,
    droppedPercent: 0,
    connectionState: "Idle",
  },
  micLevel: 0,
  desktopLevel: 0,
  activeEncoderLabel: "Detecting\u2026",
  warnings: [],
  testResult: null,
  testing: false,
  toast: null,
};

const app = document.getElementById("app")!;
let toastTimer: number | undefined;

/* ---------------------------- helpers ---------------------------- */

const esc = (s: string) => {
  const d = document.createElement("div");
  d.textContent = s;
  return d.innerHTML;
};

function duration(since: number | null): string {
  if (!since) return "00:00:00";
  const total = Math.floor((Date.now() - since) / 1000);
  return [Math.floor(total / 3600), Math.floor((total % 3600) / 60), total % 60]
    .map((v) => String(v).padStart(2, "0"))
    .join(":");
}

function showToast(message: string) {
  state.toast = message;
  render();
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => {
    state.toast = null;
    render();
  }, 4000);
}

/** Plain-language verdict derived only from measurements we actually have. */
function performanceVerdict(): { label: string; tone: string } {
  if (!state.isRecording && !state.isStreaming) return { label: "Idle", tone: "unknown" };
  const { cpu, droppedPercent } = state.stats;
  if (droppedPercent > 5 || cpu > 80) return { label: "Poor", tone: "poor" };
  if (droppedPercent > 1 || cpu > 50) return { label: "Fair", tone: "fair" };
  return { label: "Good", tone: "good" };
}

function currentModeLabel(): string {
  const r = state.config?.recording;
  if (!r) return "";
  return `${r.resolution.width} \u00d7 ${r.resolution.height} \u2022 ${r.frame_rate.replace("Fps", "")} FPS`;
}

function activeProfileName(): string | null {
  const id = state.config?.active_stream_profile;
  return state.config?.stream_profiles.find((p) => p.id === id)?.name ?? null;
}

function monitorId(): number {
  return state.hardware?.monitors[state.selectedMonitorIndex]?.handle_id ?? 0;
}

/* ------------------------------ init ----------------------------- */

async function init() {
  try {
    state.config = await api.getConfig();
    state.hardware = await api.getHardwareProfile();
    state.activeEncoderLabel = await api.getActiveEncoderLabel();
  } catch (e) {
    showToast(String(e));
  }
  render();

  await onAppEvent((event) => {
    switch (event.type) {
      case "HardwareDetected":
        state.hardware = event.data as CapabilityProfile;
        break;
      case "RecordingStarted":
        state.isRecording = true;
        state.isPaused = false;
        state.startedAt = Date.now();
        break;
      case "RecordingStopped":
        state.isRecording = false;
        state.startedAt = null;
        showToast(`Saved to ${event.data.output_path}`);
        break;
      case "RecordingPaused":
        state.isPaused = true;
        break;
      case "RecordingResumed":
        state.isPaused = false;
        break;
      case "StreamConnected":
        state.isStreaming = true;
        state.stats.connectionState = "Connected";
        break;
      case "StreamDisconnected":
        state.isStreaming = false;
        state.stats.connectionState = "Idle";
        showToast(`Stream ended: ${event.data.reason}`);
        break;
      case "StreamReconnecting":
        state.stats.connectionState = `Reconnecting ${event.data.attempt}/${event.data.max_attempts}`;
        break;
      case "StreamStatsUpdated":
        state.stats.uploadKbps = event.data.upload_bitrate_kbps;
        state.stats.droppedPercent = event.data.dropped_frames_percent;
        state.stats.connectionState = event.data.connection_state;
        break;
      case "ResourceStatsUpdated":
        state.stats.cpu = event.data.process_cpu_percent;
        state.stats.gpu = event.data.gpu_usage_percent;
        state.stats.ram = event.data.process_ram_mb;
        break;
      case "CaptureStatsUpdated":
        state.stats.captureFps = event.data.capture_fps;
        break;
      case "AudioLevels":
        state.micLevel = event.data.microphone_rms;
        state.desktopLevel = event.data.desktop_rms;
        break;
      case "ReplaySaved":
        showToast(`Replay saved to ${event.data.output_path}`);
        break;
      case "AdaptiveRecommendation":
        state.warnings = [
          { code: event.data.action, message: event.data.reason, severity: event.data.severity },
          ...state.warnings.filter((w) => w.code !== event.data.action),
        ].slice(0, 3);
        break;
      case "RecordingRecovered":
        showToast(`Recovered an unfinished recording: ${event.data.recovered_path}`);
        break;
      case "Warning":
        state.warnings = [
          { code: event.data.code, message: event.data.message, severity: "Warning" },
          ...state.warnings,
        ].slice(0, 3);
        break;
      case "Error":
        showToast(typeof event.data === "string" ? event.data : JSON.stringify(event.data));
        break;
    }
    render();
  });

  // One timer for all time-based UI. Everything else arrives via backend
  // events already batched at 1 Hz, per the spec's rule against
  // high-frequency metric updates.
  setInterval(() => {
    if (state.isRecording || state.isStreaming) render();
  }, 1000);
}

/* ---------------------------- actions ---------------------------- */

async function toggleRecord() {
  try {
    if (state.isRecording) await api.stopRecording();
    else await api.startRecording(monitorId());
  } catch (e) {
    showToast(String(e));
  }
}

async function toggleStream() {
  const profileId = state.config?.active_stream_profile;
  if (!profileId) {
    state.screen = "streaming-setup";
    render();
    return;
  }
  try {
    if (state.isStreaming) await api.stopStreaming();
    else await api.startStreaming(profileId);
  } catch (e) {
    showToast(String(e));
  }
}

async function saveReplay() {
  try {
    await api.saveReplay();
  } catch (e) {
    showToast(String(e));
  }
}

async function takeScreenshot() {
  try {
    const path = await api.takeScreenshot(monitorId());
    showToast(`Screenshot saved to ${path}`);
  } catch (e) {
    showToast(String(e));
  }
}

async function testConnection() {
  const profileId = state.config?.active_stream_profile;
  if (!profileId) {
    showToast("Save a stream profile first.");
    return;
  }
  state.testing = true;
  state.testResult = null;
  render();
  try {
    state.testResult = await api.testStreamConnection(profileId);
  } catch (e) {
    state.testResult = { success: false, message: String(e), round_trip_ms: null };
  }
  state.testing = false;
  render();
}

async function patchConfig(mutate: (c: AppConfig) => void) {
  if (!state.config) return;
  mutate(state.config);
  try {
    await api.saveConfig(state.config);
  } catch (e) {
    showToast(String(e));
  }
  render();
}

/* ---------------------------- rendering -------------------------- */

function render() {
  app.innerHTML =
    state.screen === "home"
      ? renderHome()
      : state.screen === "streaming-setup"
      ? renderStreamingSetup()
      : renderSettings();

  if (state.toast) {
    app.insertAdjacentHTML("beforeend", `<div class="toast">${esc(state.toast)}</div>`);
  }
  attachHandlers();
}

function renderHome(): string {
  const verdict = performanceVerdict();
  const a = state.config?.audio;
  const cam = state.config?.webcam;
  const live = state.isStreaming;

  return `
    <header class="app-bar">
      <div class="brand">BRAIL RECORDER</div>
      <button class="icon-btn" id="open-settings" aria-label="Settings" title="Settings">\u2699</button>
    </header>

    <main class="home">
      <div class="mode-line">${esc(currentModeLabel())}</div>
      <div class="encoder-line">${esc(state.activeEncoderLabel)}</div>

      <button class="record-orb ${state.isRecording ? "recording" : ""}" id="btn-record"
              aria-label="${state.isRecording ? "Stop recording" : "Start recording"}">
        <span class="orb-dot"></span>
        <span class="orb-label">${state.isRecording ? "STOP" : "RECORD"}</span>
      </button>

      <div class="elapsed ${state.isRecording ? "live" : ""}">${duration(state.startedAt)}</div>

      <div class="secondary-actions">
        <button class="action-btn ${live ? "live" : ""}" id="btn-stream">
          <span class="action-title">${live ? "END STREAM" : "STREAM"}</span>
          <span class="action-sub">${
            live ? esc(state.stats.connectionState) : esc(activeProfileName() ?? "Not set up")
          }</span>
        </button>
        <button class="action-btn" id="btn-replay">
          <span class="action-title">REPLAY</span>
          <span class="action-sub">${
            state.config?.instant_replay.enabled
              ? `Last ${state.config.instant_replay.buffer_seconds}s`
              : "Off"
          }</span>
        </button>
      </div>

      <div class="source-toggles">
        ${chip("mic", "\uD83C\uDFA4", !!a?.microphone.enabled && !a?.microphone.muted, state.micLevel)}
        ${chip("desktop", "\uD83D\uDD0A", !!a?.desktop.enabled && !a?.desktop.muted, state.desktopLevel)}
        ${chip("cam", "\uD83D\uDCF7", !!cam?.enabled, 0)}
        <button class="chip" id="btn-screenshot" title="Take a screenshot">\uD83D\uDCF8<span class="chip-label">SHOT</span></button>
      </div>

      <div class="verdict verdict-${verdict.tone}">
        Performance: <strong>${verdict.label}</strong>
        ${
          state.isRecording || state.isStreaming
            ? `<span class="verdict-detail">CPU ${state.stats.cpu.toFixed(0)}% \u00b7 ${
                state.stats.gpu === null ? "GPU \u2014" : `GPU ${state.stats.gpu.toFixed(0)}%`
              } \u00b7 RAM ${state.stats.ram.toFixed(0)} MB${
                live ? ` \u00b7 \u2191 ${(state.stats.uploadKbps / 1000).toFixed(1)} Mbps` : ""
              }</span>`
            : ""
        }
      </div>

      ${
        state.warnings.length
          ? `<div class="warnings">${state.warnings
              .map(
                (w) =>
                  `<div class="warning warning-${w.severity.toLowerCase()}">\u26a0 ${esc(w.message)}</div>`
              )
              .join("")}</div>`
          : ""
      }
    </main>
  `;
}

function chip(id: string, icon: string, on: boolean, level: number): string {
  const pct = Math.min(100, Math.round(level * 140));
  return `
    <button class="chip ${on ? "on" : "off"}" data-toggle="${id}">
      ${icon}<span class="chip-label">${on ? "ON" : "OFF"}</span>
      ${on && level > 0 ? `<span class="chip-meter" style="width:${pct}%"></span>` : ""}
    </button>`;
}

function renderStreamingSetup(): string {
  const profiles = state.config?.stream_profiles ?? [];
  const active = state.config?.active_stream_profile;

  return `
    <header class="app-bar">
      <button class="icon-btn" id="back-home" aria-label="Back">\u2190</button>
      <div class="brand">STREAMING</div>
      <span class="spacer"></span>
    </header>
    <main class="panel">
      <div class="field">
        <label for="sel-platform">Platform</label>
        <select id="sel-platform">
          <option>YouTube</option><option>Twitch</option><option>Facebook</option><option>Custom</option>
        </select>
      </div>
      <div class="field">
        <label for="inp-key">Stream key</label>
        <div class="key-row">
          <input type="password" id="inp-key" placeholder="Paste your stream key" autocomplete="off" />
          <button class="icon-btn" id="toggle-key" aria-label="Show or hide stream key" title="Show/hide">\uD83D\uDC41</button>
        </div>
        <div class="field-hint">Stored encrypted in Windows Credential Manager. Never written to logs or config files.</div>
      </div>
      <div class="field">
        <label for="sel-quality">Quality</label>
        <select id="sel-quality">
          ${(state.hardware ? supportedQualityOptions() : ["Detecting\u2026"])
            .map((q) => `<option>${esc(q)}</option>`)
            .join("")}
        </select>
      </div>
      <div class="field">
        <label for="sel-encoder">Encoder</label>
        <select id="sel-encoder">
          <option>Auto (${esc(state.activeEncoderLabel)})</option>
          ${(state.hardware?.supported_encoders ?? [])
            .filter((e) => e.verified)
            .map((e) => `<option>${esc(e.backend)} ${esc(e.codec)}</option>`)
            .join("")}
        </select>
      </div>

      <div class="button-row">
        <button class="btn-secondary" id="btn-test" ${state.testing ? "disabled" : ""}>
          ${state.testing ? "Testing\u2026" : "TEST CONNECTION"}
        </button>
        <button class="btn-primary" id="btn-go-live" ${!active ? "disabled" : ""}>START STREAM</button>
      </div>

      ${
        state.testResult
          ? `<div class="test-result ${state.testResult.success ? "ok" : "fail"}">
               ${state.testResult.success ? "\u2713" : "\u2717"} ${esc(state.testResult.message)}
               ${
                 state.testResult.round_trip_ms !== null
                   ? `<span class="mono"> (${state.testResult.round_trip_ms.toFixed(0)} ms)</span>`
                   : ""
               }
             </div>`
          : ""
      }

      ${
        profiles.length
          ? `<div class="section-label" style="margin-top:20px">Saved profiles</div>
             ${profiles
               .map(
                 (p) =>
                   `<button class="list-row ${p.id === active ? "selected" : ""}" data-profile="${p.id}">
                      <span>${esc(p.name)}</span><span class="mono dim">${esc(p.service)}</span>
                    </button>`
               )
               .join("")}`
          : `<div class="empty-state">No saved profiles yet.</div>`
      }
    </main>
  `;
}

function supportedQualityOptions(): string[] {
  const all: [string, number][] = [
    ["Ultra Low", 640 * 360],
    ["Low", 854 * 480],
    ["Balanced", 1280 * 720],
    ["Full HD", 1920 * 1080],
    ["High", 2560 * 1440],
    ["Ultra", 3840 * 2160],
  ];
  const max = Math.max(
    0,
    ...(state.hardware?.supported_encoders ?? [])
      .filter((e) => e.verified)
      .map((e) => e.max_resolution.width * e.max_resolution.height)
  );
  const supported = all.filter(([, px]) => px <= max).map(([label]) => label);
  return supported.length ? supported : ["Balanced"];
}

function renderSettings(): string {
  const tabs: SettingsTab[] = [
    "general",
    "recording",
    "streaming",
    "replay",
    "audio",
    "performance",
    "hotkeys",
  ];
  return `
    <header class="app-bar">
      <button class="icon-btn" id="back-home" aria-label="Back">\u2190</button>
      <div class="brand">SETTINGS</div>
      <label class="mode-switch">
        <span>Advanced</span>
        <span class="switch"><input type="checkbox" id="toggle-advanced" ${
          state.config?.ui_mode === "Advanced" ? "checked" : ""
        } /><span class="switch-track"></span></span>
      </label>
    </header>
    <div class="settings-body">
      <nav class="settings-nav">
        ${tabs
          .map(
            (t) =>
              `<button class="nav-item ${
                state.settingsTab === t ? "active" : ""
              }" data-settings-tab="${t}">${t[0].toUpperCase() + t.slice(1)}</button>`
          )
          .join("")}
      </nav>
      <div class="settings-panel">${renderSettingsTab()}</div>
    </div>
  `;
}

function renderSettingsTab(): string {
  const c = state.config;
  if (!c) return `<div class="empty-state">Loading\u2026</div>`;
  const advanced = c.ui_mode === "Advanced";

  switch (state.settingsTab) {
    case "general":
      return `
        <h3>General</h3>
        ${row("Minimize to tray", "", switchEl("set-tray", c.general.minimize_to_tray))}
        ${row("Start with Windows", "Off by default.", switchEl("set-startup", c.general.start_with_windows))}
        ${row("Show notifications", "", switchEl("set-notify", c.general.show_notifications))}
        <div class="field" style="margin-top:16px">
          <label for="set-outdir">Recording folder</label>
          <input type="text" id="set-outdir" value="${esc(c.recording.output_dir)}" readonly />
        </div>
        <div class="field">
          <label for="set-filename">Filename format</label>
          <input type="text" id="set-filename" value="${esc(c.general.filename_format)}" />
          <div class="field-hint">Tokens: {date} {time} {type} {resolution} {fps}</div>
        </div>`;

    case "recording":
      return `
        <h3>Recording</h3>
        ${row("Capture cursor", "", switchEl("set-cursor", c.recording.capture_cursor))}
        ${row("Highlight clicks", "", switchEl("set-highlight", c.recording.highlight_cursor))}
        <div class="field" style="margin-top:16px">
          <label for="set-container">Container</label>
          <select id="set-container">
            ${["Mkv", "Mp4", "WebM"]
              .map((f) => `<option ${c.recording.container === f ? "selected" : ""}>${f}</option>`)
              .join("")}
          </select>
          <div class="field-hint">MKV survives a crash intact. MP4 is offered as a fast remux after recording.</div>
        </div>
        ${
          advanced
            ? `<h3 style="margin-top:24px">Advanced encoding</h3>
               ${numField("set-bitrate", "Bitrate (kbps)", c.recording.encoder.bitrate_kbps ?? 6000)}
               ${numField("set-keyframe", "Keyframe interval (s)", c.recording.encoder.keyframe_interval_secs)}
               ${numField("set-bframes", "B-frames", c.recording.encoder.b_frames)}
               <div class="field">
                 <label for="set-ratecontrol">Rate control</label>
                 <select id="set-ratecontrol">
                   ${["Cbr", "Vbr", "Cqp"]
                     .map(
                       (m) =>
                         `<option ${c.recording.encoder.rate_control === m ? "selected" : ""}>${m}</option>`
                     )
                     .join("")}
                 </select>
               </div>`
            : `<div class="hint-card">Switch on Advanced (top right) for bitrate, rate control, keyframe interval and B-frames.</div>`
        }`;

    case "audio":
      return `
        <h3>Audio</h3>
        ${sliderRow("vol-desktop", "Desktop volume", c.audio.desktop.volume, state.desktopLevel)}
        ${row("Mute desktop", "", switchEl("mute-desktop", c.audio.desktop.muted))}
        ${sliderRow("vol-mic", "Microphone volume", c.audio.microphone.volume, state.micLevel)}
        ${row("Mute microphone", "", switchEl("mute-mic", c.audio.microphone.muted))}
        <div class="field" style="margin-top:16px">
          <label for="set-samplerate">Sample rate</label>
          <select id="set-samplerate">
            <option ${c.audio.sample_rate === 48000 ? "selected" : ""}>48000</option>
            <option ${c.audio.sample_rate === 44100 ? "selected" : ""}>44100</option>
          </select>
        </div>
        ${row("Separate audio tracks", "Keeps desktop and mic on their own tracks.", switchEl("set-tracks", c.audio.separate_tracks))}
        ${row("Noise suppression", "Costs CPU. Off by default.", switchEl("set-noise", c.audio.noise_suppression))}`;

    case "replay":
      return `
        <h3>Instant Replay</h3>
        ${row("Enable instant replay", "Continuously buffers recent footage.", switchEl("set-replay", c.instant_replay.enabled))}
        <div class="field" style="margin-top:16px">
          <label for="set-buffer">Buffer duration</label>
          <select id="set-buffer">
            ${[15, 30, 60, 120, 300]
              .map(
                (s) =>
                  `<option value="${s}" ${c.instant_replay.buffer_seconds === s ? "selected" : ""}>${
                    s < 60 ? `${s} seconds` : `${s / 60} minute${s > 60 ? "s" : ""}`
                  }</option>`
              )
              .join("")}
          </select>
          <div class="field-hint">Longer buffers use proportionally more memory.</div>
        </div>`;

    case "performance":
      return `
        <h3>Brail Adaptive Engine</h3>
        <div class="field">
          <label for="set-mode">Mode</label>
          <select id="set-mode">
            ${["UltraLite", "LowEnd", "Balanced", "Quality", "Streaming", "Custom"]
              .map((m) => `<option ${c.performance.adaptive_mode === m ? "selected" : ""}>${m}</option>`)
              .join("")}
          </select>
        </div>
        ${row("Auto-optimize", "Lets the engine apply its recommendations. Custom mode only ever warns.", switchEl("set-autoopt", c.performance.auto_optimize))}
        ${row("Gaming Mode", "Minimizes preview and UI work while capturing a game.", switchEl("set-gaming", c.performance.gaming_mode))}
        ${row("Resource monitoring", "", switchEl("set-monitor", c.performance.resource_monitoring))}
        <div class="field" style="margin-top:16px">
          <label for="set-previewfps">Preview FPS (0 disables)</label>
          <input type="number" id="set-previewfps" value="${c.performance.preview_fps}" min="0" max="60" />
        </div>
        <button class="btn-secondary" id="btn-benchmark" style="margin-top:12px">RUN BENCHMARK</button>`;

    case "hotkeys":
      return `
        <h3>Hotkeys</h3>
        ${Object.entries(c.hotkeys)
          .map(([action, combo]) => {
            const label = action.replace(/_/g, " ").replace(/\b\w/g, (m) => m.toUpperCase());
            const keys = combo
              ? [
                  combo.ctrl && "Ctrl",
                  combo.shift && "Shift",
                  combo.alt && "Alt",
                  combo.win && "Win",
                  combo.key,
                ]
                  .filter(Boolean)
                  .join(" + ")
              : "Not set";
            return `<div class="hotkey-row"><span>${esc(label)}</span><kbd>${esc(keys)}</kbd></div>`;
          })
          .join("")}`;

    case "streaming":
      return `
        <h3>Streaming</h3>
        <button class="btn-secondary" id="open-streaming-setup">OPEN STREAMING SETUP</button>
        <div class="field-hint" style="margin-top:12px">Platform, stream key, quality and connection testing live there.</div>`;
  }
}

function row(label: string, hint: string, control: string): string {
  return `<div class="toggle-row">
    <div><div class="toggle-row-label">${esc(label)}</div>${
    hint ? `<div class="toggle-row-hint">${esc(hint)}</div>` : ""
  }</div>${control}</div>`;
}

function switchEl(id: string, checked: boolean | undefined): string {
  return `<label class="switch"><input type="checkbox" id="${id}" ${
    checked ? "checked" : ""
  } /><span class="switch-track"></span></label>`;
}

function numField(id: string, label: string, value: number): string {
  return `<div class="field"><label for="${id}">${esc(
    label
  )}</label><input type="number" id="${id}" value="${value}" /></div>`;
}

function sliderRow(id: string, label: string, value: number, level: number): string {
  return `<div class="field">
    <label for="${id}">${esc(label)}</label>
    <input type="range" id="${id}" min="0" max="150" value="${Math.round(value * 100)}" />
    <div class="level-meter"><div class="level-fill" style="width:${Math.min(100, level * 140)}%"></div></div>
  </div>`;
}

/* --------------------------- handlers ---------------------------- */

function attachHandlers() {
  const on = (sel: string, ev: string, fn: (e: Event) => void) =>
    app.querySelector(sel)?.addEventListener(ev, fn);

  on("#open-settings", "click", () => {
    state.screen = "settings";
    render();
  });
  on("#back-home", "click", () => {
    state.screen = "home";
    state.testResult = null;
    render();
  });
  on("#open-streaming-setup", "click", () => {
    state.screen = "streaming-setup";
    render();
  });

  on("#btn-record", "click", toggleRecord);
  on("#btn-stream", "click", toggleStream);
  on("#btn-replay", "click", saveReplay);
  on("#btn-screenshot", "click", takeScreenshot);
  on("#btn-test", "click", testConnection);
  on("#btn-go-live", "click", toggleStream);

  on("#toggle-key", "click", () => {
    const input = app.querySelector<HTMLInputElement>("#inp-key");
    if (input) input.type = input.type === "password" ? "text" : "password";
  });

  app.querySelectorAll<HTMLButtonElement>("[data-settings-tab]").forEach((b) =>
    b.addEventListener("click", () => {
      state.settingsTab = b.dataset.settingsTab as SettingsTab;
      render();
    })
  );

  app.querySelectorAll<HTMLButtonElement>("[data-toggle]").forEach((b) =>
    b.addEventListener("click", () => {
      const which = b.dataset.toggle;
      patchConfig((c) => {
        if (which === "mic") c.audio.microphone.muted = !c.audio.microphone.muted;
        if (which === "desktop") c.audio.desktop.muted = !c.audio.desktop.muted;
        if (which === "cam") c.webcam.enabled = !c.webcam.enabled;
      });
    })
  );

  app.querySelectorAll<HTMLButtonElement>("[data-profile]").forEach((b) =>
    b.addEventListener("click", () =>
      patchConfig((c) => (c.active_stream_profile = b.dataset.profile!))
    )
  );

  const bindSwitch = (id: string, apply: (c: AppConfig, v: boolean) => void) =>
    on(`#${id}`, "change", (e) =>
      patchConfig((c) => apply(c, (e.target as HTMLInputElement).checked))
    );

  bindSwitch("toggle-advanced", (c, v) => (c.ui_mode = v ? "Advanced" : "Beginner"));
  bindSwitch("set-tray", (c, v) => (c.general.minimize_to_tray = v));
  bindSwitch("set-startup", (c, v) => (c.general.start_with_windows = v));
  bindSwitch("set-notify", (c, v) => (c.general.show_notifications = v));
  bindSwitch("set-cursor", (c, v) => (c.recording.capture_cursor = v));
  bindSwitch("set-highlight", (c, v) => (c.recording.highlight_cursor = v));
  bindSwitch("mute-mic", (c, v) => (c.audio.microphone.muted = v));
  bindSwitch("mute-desktop", (c, v) => (c.audio.desktop.muted = v));
  bindSwitch("set-tracks", (c, v) => (c.audio.separate_tracks = v));
  bindSwitch("set-noise", (c, v) => (c.audio.noise_suppression = v));
  bindSwitch("set-replay", (c, v) => (c.instant_replay.enabled = v));
  bindSwitch("set-autoopt", (c, v) => (c.performance.auto_optimize = v));
  bindSwitch("set-gaming", (c, v) => (c.performance.gaming_mode = v));
  bindSwitch("set-monitor", (c, v) => (c.performance.resource_monitoring = v));

  on("#vol-mic", "change", (e) =>
    patchConfig(
      (c) => (c.audio.microphone.volume = Number((e.target as HTMLInputElement).value) / 100)
    )
  );
  on("#vol-desktop", "change", (e) =>
    patchConfig((c) => (c.audio.desktop.volume = Number((e.target as HTMLInputElement).value) / 100))
  );
  on("#set-buffer", "change", (e) =>
    patchConfig(
      (c) => (c.instant_replay.buffer_seconds = Number((e.target as HTMLSelectElement).value))
    )
  );
  on("#set-filename", "change", (e) =>
    patchConfig((c) => (c.general.filename_format = (e.target as HTMLInputElement).value))
  );
  on("#set-previewfps", "change", (e) =>
    patchConfig((c) => (c.performance.preview_fps = Number((e.target as HTMLInputElement).value)))
  );
  on("#set-mode", "change", (e) =>
    patchConfig((c) => (c.performance.adaptive_mode = (e.target as HTMLSelectElement).value as never))
  );
  on("#set-container", "change", (e) =>
    patchConfig((c) => (c.recording.container = (e.target as HTMLSelectElement).value as never))
  );
  on("#set-bitrate", "change", (e) =>
    patchConfig(
      (c) => (c.recording.encoder.bitrate_kbps = Number((e.target as HTMLInputElement).value))
    )
  );

  on("#btn-benchmark", "click", async () => {
    showToast("Running benchmark\u2026 this takes about 30 seconds.");
    try {
      const report = await api.runBenchmark();
      showToast(`Benchmark complete: ${report.summary}`);
    } catch (e) {
      showToast(String(e));
    }
  });
}

init();
