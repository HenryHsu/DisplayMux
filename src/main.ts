import "@fontsource-variable/manrope";
import {
  Activity, ArrowLeftRight, CircleHelp, Computer, createIcons, Download, KeyRound, Laptop,
  ChevronDown, ExternalLink, Github, Languages, Monitor, MoonStar, Network, Plus, RefreshCw, Save, Search, Settings,
  ShieldCheck, Trash2, UserRound, Zap,
} from "lucide";
import { getVersion } from "@tauri-apps/api/app";
import { Channel, invoke } from "@tauri-apps/api/core";
import packageMetadata from "../package.json";
import { locale, localePreference, setLocalePreference, t } from "./i18n";
import "./styles.css";

type Platform = "windows" | "mac";
type ResolutionSource = "edid" | "coreGraphicsDisplayMode" | "windowsDisplayMode";

interface MonitorResolution {
  width: number;
  height: number;
}

interface Fingerprint {
  manufacturer_id: string;
  product_code: string;
  serial_number: string | null;
}

interface MonitorDescriptor {
  id: string;
  name: string;
  active: boolean;
  builtIn: boolean;
  fingerprint: Fingerprint;
  maxResolution?: MonitorResolution | null;
  resolutionSource?: ResolutionSource | null;
}

interface SelectedMonitor {
  name: string;
  fingerprint: Fingerprint;
  maxResolution?: MonitorResolution | null;
  resolutionSource?: ResolutionSource | null;
}

interface HostRoute {
  id: string;
  name: string;
  platform: Platform;
  address: string;
  port: number;
  macAddress: string;
  input: number | null;
}

interface AppSettings {
  localHost: Platform;
  sharedMonitor: SelectedMonitor | null;
  localInput: number | null;
  supportedInputs: number[] | null;
  peers: HostRoute[];
  broadcastIp: string;
  wakePort: number;
  sharedKey: string;
  waitSeconds: number;
  autostart: boolean;
  checkUpdates: boolean;
}

interface DashboardState {
  platform: string;
  localHost: Platform;
  agentConfigured: boolean;
  ddcAvailable: boolean;
  monitorStatus: string;
  selectionNotice: string | null;
  monitors: MonitorDescriptor[];
}

interface DiscoveredPeer {
  id: string;
  name: string;
  platform: Platform;
  address: string;
  port: number;
  macAddress: string | null;
}

interface InputOption { value: number; name: string; }
interface OperationResult { title: string; detail: string; peerWoken: boolean; warning: boolean; }
interface UpdateInfo { available: boolean; currentVersion: string; version: string | null; notes: string | null; }
type SwitchProgressEvent =
  | { event: "waking"; peerName: string }
  | { event: "checking"; peerName: string }
  | { event: "waiting"; peerName: string; seconds: number }
  | { event: "switching" }
  | { event: "remoteFallback"; peerName: string };
type UpdateDownloadEvent =
  | { event: "started"; contentLength: number | null }
  | { event: "progress"; downloaded: number; contentLength: number | null }
  | { event: "finished" };

const standardInputs: InputOption[] = [
  [0x01, "VGA 1"], [0x02, "VGA 2"], [0x03, "DVI 1"], [0x04, "DVI 2"],
  [0x05, t("input.composite1")], [0x06, t("input.composite2")],
  [0x07, "S-Video 1"], [0x08, "S-Video 2"], [0x09, t("input.tuner1")],
  [0x0a, t("input.tuner2")], [0x0b, t("input.tuner3")], [0x0c, t("input.component1")],
  [0x0d, t("input.component2")], [0x0e, t("input.component3")],
  [0x0f, "DP 1"], [0x10, "DP 2"], [0x11, "HDMI 1"], [0x12, "HDMI 2"], [0x1b, "Type-C"],
].map(([value, name]) => ({ value: value as number, name: name as string }));

const previewSettings: AppSettings = {
  localHost: "windows", sharedMonitor: null, localInput: null, supportedInputs: null, peers: [],
  broadcastIp: "255.255.255.255", wakePort: 9, sharedKey: "", waitSeconds: 45, autostart: true, checkUpdates: true,
};
const previewDashboard: DashboardState = {
  platform: "windows", localHost: "windows", agentConfigured: false, ddcAvailable: false,
  monitorStatus: t("preview.monitorStatus"), selectionNotice: null, monitors: [],
};

let settings = previewSettings;
let dashboard = previewDashboard;
let discoveredPeers: DiscoveredPeer[] = [];
let inputOptions = standardInputs;
let isPreview = false;
let pendingUpdate: UpdateInfo | null = null;

const app = document.querySelector<HTMLDivElement>("#app");
if (!app) throw new Error(t("app.rootMissing"));
document.documentElement.lang = locale;

app.innerHTML = `
  <div class="app-shell">
    <aside class="sidebar" aria-label="${t("nav.aria")}">
      <div class="brand-block">
        <div class="brand-header">
          <div class="brand-icon"><i data-lucide="monitor"></i></div>
          <span class="brand-title">DisplayMux</span>
        </div>
        <label class="language-picker">
          <i data-lucide="languages"></i>
          <span class="sr-only">${t("language.label")}</span>
          <select id="language-select" aria-label="${t("language.label")}">
            <option value="system">${t("language.system")}</option>
            <option value="en">${t("language.english")}</option>
            <option value="zh-TW">${t("language.traditionalChinese")}</option>
          </select>
          <i class="language-chevron" data-lucide="chevron-down"></i>
        </label>
      </div>
      <nav class="sidebar-nav">
        <button class="nav-button is-active" data-page="dashboard"><i data-lucide="arrow-left-right"></i><span>${t("nav.dashboard")}</span></button>
        <button class="nav-button" data-page="settings"><i data-lucide="settings"></i><span>${t("nav.settings")}</span></button>
      </nav>
      <button class="nav-button nav-bottom" data-page="help"><i data-lucide="circle-help"></i><span>${t("nav.help")}</span></button>
    </aside>
    <main class="workspace">
      <header class="topbar">
        <h1 id="page-title">${t("page.dashboard")}</h1>
        <div class="topbar-actions">
          <div class="agent-pill" id="agent-pill"><span class="status-dot"></span><span>${t("dashboard.agentMissing")}</span></div>
          <button class="icon-button" id="update-button" title="${t("action.checkUpdates")}"><i data-lucide="download"></i></button>
          <button class="icon-button" id="refresh-button" title="${t("action.refresh")}"><i data-lucide="refresh-cw"></i></button>
        </div>
      </header>

      <section class="page is-active" id="dashboard-page">
        <div class="showcase-monitor-card">
          <div class="showcase-header">
            <span class="showcase-title">${t("dashboard.sharedDisplay")}</span>
            <div class="showcase-badges">
              <span class="status-badge subtle" id="screen-ratio">16:9</span>
              <span class="status-badge" id="screen-input">${t("dashboard.ddcReady")}</span>
            </div>
          </div>
          <div class="flat-monitor-wrap" id="flat-monitor-wrap"></div>
          <div class="showcase-info">
            <strong class="showcase-monitor-name" id="shared-monitor-name">${t("dashboard.notSelected")}</strong>
            <p class="showcase-monitor-desc" id="monitor-status">${t("dashboard.identityHint")}</p>
          </div>
        </div>

        <div class="host-route-grid" id="host-route-grid"></div>

        <section class="status-summary-bar">
          <div class="summary-item"><span>${t("dashboard.sharedLabel")}</span><strong id="monitor-health" class="text-accent">${t("dashboard.detecting")}</strong></div>
          <span class="summary-pipe"></span>
          <div class="summary-item"><span>${t("dashboard.hostsLabel")}</span><strong id="peer-health">${t("dashboard.hostCount", { count: 0 })}</strong></div>
          <span class="summary-pipe"></span>
          <div class="summary-item"><span>${t("dashboard.wakeLabel")}</span><strong id="wake-health" class="text-accent">${t("dashboard.noHosts")}</strong></div>
        </section>
      </section>

      <section class="page" id="settings-page">
        <div class="settings-layout">
          <section class="settings-main">
            <form id="settings-form">
              <div class="form-section first">
                <div class="pairing-heading">
                  <strong>${t("settings.stepMonitor")}</strong>
                </div>
                <div class="monitor-picker" id="monitor-picker"></div>
              </div>

              <div class="form-section two-columns">
                <label class="field"><span>${t("settings.localComputer")}</span><input id="local-host-name" disabled /></label>
                <label class="field"><span>${t("settings.localInput")}</span><select id="local-input" disabled></select><small id="local-input-name">${t("input.unset")}</small></label>
              </div>

              <div class="form-section pairing-section">
                <div class="pairing-heading">
                  <strong>${t("settings.stepHosts")}</strong>
                  <button class="scan-button" id="scan-button" type="button"><i data-lucide="search"></i>${t("action.searchAgain")}</button>
                </div>
                <div class="peer-list" id="peer-list"></div>
                <div class="paired-routes" id="paired-routes"></div>
              </div>

              <div class="form-section two-columns">
                <label class="field"><span>${t("settings.password")}</span><div class="input-wrap"><i data-lucide="key-round"></i><input id="shared-key" type="password" minlength="8" placeholder="${t("settings.passwordPlaceholder")}" /></div><small>${t("settings.passwordHint")}</small></label>
                <label class="field compact"><span>${t("settings.wait")}</span><input id="wait-seconds" type="number" min="5" max="120" /><small>${t("settings.waitHint")}</small></label>
              </div>

              <div class="toggles-section">
                <label class="switch-row">
                  <span class="switch-label">
                    <strong>${t("settings.autostart")}</strong>
                    <small>${t("settings.autostartHint")}</small>
                  </span>
                  <input id="autostart" type="checkbox" class="toggle-checkbox" />
                  <span class="switch-slider"></span>
                </label>
                <label class="switch-row">
                  <span class="switch-label">
                    <strong>${t("settings.autoUpdates")}</strong>
                    <small>${t("settings.autoUpdatesHint")}</small>
                  </span>
                  <input id="check-updates" type="checkbox" class="toggle-checkbox" />
                  <span class="switch-slider"></span>
                </label>
              </div>

              <div class="form-actions">
                <button class="save-button full-width" type="submit"><i data-lucide="save"></i>${t("action.save")}</button>
              </div>
            </form>
          </section>

          <aside class="compatibility-panel">
            <h3>${t("settings.inputGuide")}</h3>
            <div class="path-item"><span class="path-badge">01</span><div><strong>${t("settings.mccsTitle")}</strong><p>${t("settings.mccsBody")}</p></div></div>
            <div class="path-item"><span class="path-badge">02</span><div><strong>${t("settings.ddcTitle")}</strong><p>${t("settings.ddcBody")}</p></div></div>
            <div class="path-item"><span class="path-badge">03</span><div><strong>${t("settings.routingTitle")}</strong><p>${t("settings.routingBody")}</p></div></div>
            <div class="compat-note"><i data-lucide="shield-check"></i><p>${t("settings.safetyBody")}</p></div>
          </aside>
        </div>
      </section>

      <section class="page" id="help-page">
        <div class="help-content">
          <p class="section-kicker">OPERATING NOTES</p><h2>${t("help.heading")}</h2>
          <div class="note-list">
            <article><span>01</span><div><h3>${t("help.replaceTitle")}</h3><p>${t("help.replaceBody")}</p></div></article>
            <article><span>02</span><div><h3>${t("help.inputTitle")}</h3><p>${t("help.inputBody")}</p></div></article>
            <article><span>03</span><div><h3>${t("help.autoTitle")}</h3><p>${t("help.autoBody")}</p></div></article>
            <article><span>04</span><div><h3>${t("help.adapterTitle")}</h3><p>${t("help.adapterBody")}</p></div></article>
          </div>

          <section class="about-section" aria-labelledby="about-title">
            <p class="section-kicker">ABOUT</p><h2 id="about-title">${t("about.title")}</h2>
            <dl class="about-grid">
              <div class="about-item">
                <dt><i data-lucide="user-round"></i>${t("about.developer")}</dt>
                <dd>Henry Hsu</dd>
              </div>
              <div class="about-item">
                <dt><i data-lucide="github"></i>GitHub</dt>
                <dd><a href="https://github.com/HenryHsu/DisplayMux" target="_blank" rel="noopener noreferrer">HenryHsu/DisplayMux<i data-lucide="external-link"></i></a></dd>
              </div>
              <div class="about-item">
                <dt><i data-lucide="activity"></i>${t("about.version")}</dt>
                <dd id="app-version" aria-live="polite">${t("about.loading")}</dd>
              </div>
            </dl>
          </section>
        </div>
      </section>
    </main>
  </div>
  <div class="operation-overlay" id="operation-overlay" aria-live="polite" aria-hidden="true"><div class="operation-dialog"><div class="spinner"></div><p class="section-kicker">SMART SWITCH</p><h2 id="operation-title">${t("operation.running")}</h2><p id="operation-detail">${t("operation.preparingBody")}</p></div></div>
  <div class="update-overlay" id="update-overlay" aria-hidden="true">
    <div class="update-dialog">
      <p class="section-kicker">SIGNED UPDATE</p>
      <h2 id="update-title">${t("update.available")}</h2>
      <p id="update-version"></p>
      <div class="update-notes" id="update-notes"></div>
      <div class="update-progress" id="update-progress" hidden><div id="update-progress-bar"></div></div>
      <p class="update-progress-label" id="update-progress-label"></p>
      <div class="update-actions"><button class="scan-button" id="update-cancel" type="button">${t("action.later")}</button><button class="save-button" id="update-install" type="button"><i data-lucide="download"></i>${t("action.downloadInstall")}</button></div>
    </div>
  </div>
  <div class="toast" id="toast" role="status" aria-live="polite"><i data-lucide="zap"></i><div><strong id="toast-title"></strong><span id="toast-detail"></span></div></div>
`;

const iconSet = { Activity, ArrowLeftRight, ChevronDown, CircleHelp, Computer, Download, ExternalLink, Github, KeyRound, Languages, Laptop, Monitor, MoonStar, Network, Plus, RefreshCw, Save, Search, Settings, ShieldCheck, Trash2, UserRound, Zap };
const refreshIcons = () => createIcons({ icons: iconSet });
refreshIcons();

const pageTitles: Record<string, string> = { dashboard: t("page.dashboard"), settings: t("page.settings"), help: t("page.help") };
document.querySelectorAll<HTMLButtonElement>("[data-page]").forEach((button) => button.addEventListener("click", () => showPage(button.dataset.page ?? "dashboard")));
document.querySelector<HTMLButtonElement>("#refresh-button")?.addEventListener("click", () => void refresh());
document.querySelector<HTMLButtonElement>("#update-button")?.addEventListener("click", () => pendingUpdate ? showUpdateDialog(pendingUpdate) : void checkForUpdates(true));
document.querySelector<HTMLButtonElement>("#update-cancel")?.addEventListener("click", hideUpdateDialog);
document.querySelector<HTMLButtonElement>("#update-install")?.addEventListener("click", () => void installUpdate());
document.querySelector<HTMLButtonElement>("#scan-button")?.addEventListener("click", () => void scanPeers());
const languageSelect = document.querySelector<HTMLSelectElement>("#language-select");
if (languageSelect) {
  languageSelect.value = localePreference;
  languageSelect.addEventListener("change", () => {
    if (setLocalePreference(languageSelect.value)) window.location.reload();
  });
}
document.querySelector<HTMLFormElement>("#settings-form")?.addEventListener("submit", (event) => void saveSettings(event));
document.querySelector<HTMLSelectElement>("#local-input")?.addEventListener("input", renderInputHints);
document.querySelector("#monitor-picker")?.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-monitor-id]");
  if (button?.dataset.monitorId) void selectMonitor(button.dataset.monitorId);
});
document.querySelector("#peer-list")?.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-add-peer]");
  if (button?.dataset.addPeer) void addPeer(button.dataset.addPeer);
});
document.querySelector("#paired-routes")?.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-remove-peer], [data-probe-id], [data-wake-id]");
  if (button?.dataset.removePeer) void removePeer(button.dataset.removePeer);
  if (button?.dataset.probeId) void peerCommand("probe_peer", button.dataset.probeId);
  if (button?.dataset.wakeId) void peerCommand("wake_peer", button.dataset.wakeId);
});
document.querySelector("#paired-routes")?.addEventListener("input", renderInputHints);
document.querySelector("#host-route-grid")?.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-switch-id]");
  if (button?.dataset.switchId) void switchHost(button.dataset.switchId);
});

function showPage(page: string): void {
  document.querySelectorAll(".page").forEach((item) => item.classList.remove("is-active"));
  document.querySelector(`#${page}-page`)?.classList.add("is-active");
  document.querySelectorAll(".nav-button").forEach((item) => item.classList.toggle("is-active", (item as HTMLElement).dataset.page === page));
  setText("#page-title", pageTitles[page] ?? pageTitles.dashboard);
}

async function refresh(): Promise<void> {
  document.querySelector("#refresh-button svg")?.classList.add("is-spinning");
  try {
    dashboard = await invoke<DashboardState>("get_dashboard_state");
    [settings, inputOptions] = await Promise.all([
      invoke<AppSettings>("get_settings"), invoke<InputOption[]>("get_input_options"),
    ]);
    try { discoveredPeers = await invoke<DiscoveredPeer[]>("discover_peers"); } catch { discoveredPeers = []; }
    isPreview = false;
  } catch {
    dashboard = previewDashboard; settings = previewSettings; inputOptions = standardInputs; discoveredPeers = []; isPreview = true;
  } finally { document.querySelector("#refresh-button svg")?.classList.remove("is-spinning"); }
  renderState();
  if (!isPreview && dashboard.selectionNotice) {
    showToast(t("toast.selectionUpdated"), dashboard.selectionNotice);
  }
}

function getFlatMonitorSvg(isUltrawide: boolean): string {
  if (isUltrawide) {
    return `<svg class="flat-monitor-svg" viewBox="0 0 380 190" fill="none" xmlns="http://www.w3.org/2000/svg">
      <defs>
        <linearGradient id="screen21" x1="190" y1="18" x2="190" y2="144" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stop-color="#142c22"/>
          <stop offset="100%" stop-color="#0b1713"/>
        </linearGradient>
        <linearGradient id="glare21" x1="360" y1="20" x2="160" y2="140" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stop-color="#ffffff" stop-opacity="0.16"/>
          <stop offset="45%" stop-color="#ffffff" stop-opacity="0.03"/>
          <stop offset="100%" stop-color="#ffffff" stop-opacity="0"/>
        </linearGradient>
        <linearGradient id="standNeck" x1="182" y1="144" x2="198" y2="144" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stop-color="#2a3a32"/>
          <stop offset="50%" stop-color="#42574c"/>
          <stop offset="100%" stop-color="#1e2a24"/>
        </linearGradient>
        <linearGradient id="standBase" x1="190" y1="172" x2="190" y2="180" gradientUnits="userSpaceOnUse">
          <stop offset="0%" stop-color="#3c5045"/>
          <stop offset="100%" stop-color="#1a2520"/>
        </linearGradient>
      </defs>
      <rect x="183" y="142" width="14" height="32" rx="2" fill="url(#standNeck)"/>
      <rect x="177" y="132" width="26" height="18" rx="3" fill="#1b2520"/>
      <rect x="125" y="172" width="130" height="7" rx="3.5" fill="url(#standBase)"/>
      <rect x="126" y="172" width="128" height="1.5" rx="0.75" fill="#587363" opacity="0.6"/>
      <rect x="16" y="16" width="348" height="130" rx="6" fill="#15211b" stroke="#2c3f34" stroke-width="2"/>
      <rect x="20" y="20" width="340" height="122" rx="3" fill="url(#screen21)"/>
      <polygon points="20,20 220,20 120,142 20,142" fill="url(#glare21)"/>
      <circle cx="190" cy="142" r="1.5" fill="#4ade80" opacity="0.8"/>
    </svg>`;
  }
  return `<svg class="flat-monitor-svg" viewBox="0 0 380 190" fill="none" xmlns="http://www.w3.org/2000/svg">
    <defs>
      <linearGradient id="screen16" x1="190" y1="14" x2="190" y2="152" gradientUnits="userSpaceOnUse">
        <stop offset="0%" stop-color="#142c22"/>
        <stop offset="100%" stop-color="#0b1713"/>
      </linearGradient>
      <linearGradient id="glare16" x1="320" y1="16" x2="160" y2="150" gradientUnits="userSpaceOnUse">
        <stop offset="0%" stop-color="#ffffff" stop-opacity="0.16"/>
        <stop offset="45%" stop-color="#ffffff" stop-opacity="0.03"/>
        <stop offset="100%" stop-color="#ffffff" stop-opacity="0"/>
      </linearGradient>
      <linearGradient id="standNeck" x1="182" y1="144" x2="198" y2="144" gradientUnits="userSpaceOnUse">
        <stop offset="0%" stop-color="#2a3a32"/>
        <stop offset="50%" stop-color="#42574c"/>
        <stop offset="100%" stop-color="#1e2a24"/>
      </linearGradient>
      <linearGradient id="standBase" x1="190" y1="172" x2="190" y2="180" gradientUnits="userSpaceOnUse">
        <stop offset="0%" stop-color="#3c5045"/>
        <stop offset="100%" stop-color="#1a2520"/>
      </linearGradient>
    </defs>
    <rect x="183" y="148" width="14" height="26" rx="2" fill="url(#standNeck)"/>
    <rect x="177" y="138" width="26" height="18" rx="3" fill="#1b2520"/>
    <rect x="135" y="172" width="110" height="7" rx="3.5" fill="url(#standBase)"/>
    <rect x="136" y="172" width="108" height="1.5" rx="0.75" fill="#587363" opacity="0.6"/>
    <rect x="65" y="12" width="250" height="144" rx="6" fill="#15211b" stroke="#2c3f34" stroke-width="2"/>
    <rect x="69" y="16" width="242" height="136" rx="3" fill="url(#screen16)"/>
    <polygon points="69,16 220,16 140,152 69,152" fill="url(#glare16)"/>
    <circle cx="190" cy="152.5" r="1.5" fill="#4ade80" opacity="0.8"/>
  </svg>`;
}

function renderState(): void {
  let currentResolution: MonitorResolution | null = null;
  let currentResolutionSource: ResolutionSource | null = null;
  if (settings.sharedMonitor) {
    setText("#shared-monitor-name", settings.sharedMonitor.name);
    setText("#monitor-status", dashboard.monitorStatus);
    currentResolution = settings.sharedMonitor.maxResolution ?? null;
    currentResolutionSource = settings.sharedMonitor.resolutionSource ?? null;
    if (!currentResolution) {
      const match = dashboard.monitors.find((m) =>
        sameFingerprint(m.fingerprint, settings.sharedMonitor!.fingerprint)
      );
      if (match?.maxResolution) {
        currentResolution = match.maxResolution;
        currentResolutionSource = match.resolutionSource ?? null;
      }
    }
  } else {
    setText("#shared-monitor-name", t("dashboard.notSelected"));
    setText("#monitor-status", dashboard.monitorStatus || t("preview.monitorStatus"));
    if (dashboard.monitors.length > 0 && dashboard.monitors[0].maxResolution) {
      currentResolution = dashboard.monitors[0].maxResolution;
      currentResolutionSource = dashboard.monitors[0].resolutionSource ?? null;
    }
  }

  const isUltrawide = Boolean(currentResolution && isUltrawideResolution(currentResolution));

  const monitorWrap = document.querySelector("#flat-monitor-wrap");
  if (monitorWrap) {
    monitorWrap.innerHTML = getFlatMonitorSvg(isUltrawide);
  }

  const ratioBadge = document.querySelector("#screen-ratio");
  if (ratioBadge) {
    if (currentResolution) {
      ratioBadge.textContent = isUltrawide
        ? `21:9 · ${currentResolution.width}×${currentResolution.height} · ${resolutionSourceName(currentResolutionSource)}`
        : `16:9 · ${currentResolution.width}×${currentResolution.height} · ${resolutionSourceName(currentResolutionSource)}`;
    } else {
      ratioBadge.textContent = isUltrawide ? "21:9" : "16:9";
    }
  }

  setText("#screen-input", dashboard.ddcAvailable ? t("dashboard.ddcReady") : t("dashboard.notReady"));
  setText("#monitor-health", dashboard.ddcAvailable ? t("dashboard.locked") : t("dashboard.notReady"));
  setText("#peer-health", t("dashboard.hostCount", { count: settings.peers.length }));
  setText("#wake-health", settings.peers.some((peer) => peer.macAddress) ? t("dashboard.wakeNormal") : (settings.peers.length ? t("dashboard.noMac") : t("dashboard.noHosts")));
  const pill = document.querySelector("#agent-pill");
  pill?.classList.toggle("is-ready", dashboard.agentConfigured);
  if (pill) pill.querySelector("span:last-child")!.textContent = isPreview ? t("dashboard.preview") : dashboard.agentConfigured ? t("dashboard.agentReady") : t("dashboard.agentMissing");
  setInput("#local-host-name", dashboard.localHost === "windows" ? t("dashboard.localWindowsPc") : t("dashboard.localMac"));
  const localInput = document.querySelector<HTMLSelectElement>("#local-input");
  if (localInput) {
    localInput.innerHTML = renderInputOptions("local", settings.localInput);
    localInput.value = settings.localInput == null ? "" : String(settings.localInput);
  }
  setInput("#shared-key", settings.sharedKey);
  setInput("#wait-seconds", String(settings.waitSeconds));
  const autostart = document.querySelector<HTMLInputElement>("#autostart");
  if (autostart) autostart.checked = settings.autostart;
  const checkUpdates = document.querySelector<HTMLInputElement>("#check-updates");
  if (checkUpdates) checkUpdates.checked = settings.checkUpdates;
  renderMonitors(); renderPeerList(); renderPairedRoutes(); renderHostRoutes(); renderInputHints(); refreshIcons();
}

function renderMonitors(): void {
  const container = document.querySelector("#monitor-picker");
  if (!container) return;
  if (!dashboard.monitors.length) {
    container.innerHTML = `<p class="peer-empty">${t("settings.noMonitors")}</p>`; return;
  }
  container.innerHTML = dashboard.monitors.map((monitor) => {
    const selected = settings.sharedMonitor?.fingerprint;
    const isSelected = selected && sameFingerprint(selected, monitor.fingerprint);
    const fp = monitor.fingerprint;
    const res = monitor.maxResolution;
    const resText = res
      ? `(${res.width}×${res.height} ${isUltrawideResolution(res) ? "21:9" : "16:9"} · ${resolutionSourceName(monitor.resolutionSource ?? null)})`
      : "";
    return `<article class="monitor-card-item ${isSelected ? "is-selected" : ""}">
      <div class="monitor-item-left">
        <div class="monitor-item-icon"><i data-lucide="monitor"></i></div>
        <div class="monitor-identity">
          <strong>${escapeHtml(monitor.name)}</strong>
          <span>${escapeHtml(fp.manufacturer_id)} / ${escapeHtml(fp.product_code)} / ${escapeHtml(fp.serial_number ?? t("settings.noSerial"))} ${resText} (${t("settings.ddcControllable")})</span>
        </div>
      </div>
      <button type="button" class="monitor-select-btn ${isSelected ? "is-selected" : ""}" data-monitor-id="${escapeHtml(monitor.id)}" ${isSelected ? "disabled" : ""}>
        ${isSelected ? t("action.selected") : t("action.selectShared")}
      </button>
    </article>`;
  }).join("");
}

function renderPeerList(): void {
  const list = document.querySelector("#peer-list");
  if (!list) return;
  const available = discoveredPeers.filter((peer) => !settings.peers.some((item) => item.id === peer.id));
  list.innerHTML = available.length ? available.map((peer) => `<article class="peer-row">
    <div class="peer-identity">
      <strong>${escapeHtml(peer.name)}</strong>
      <span>${platformName(peer.platform)} · ${t("settings.networkAuto")}</span>
    </div>
    <button type="button" class="peer-add-btn" data-add-peer="${escapeHtml(peer.id)}"><i data-lucide="plus"></i>${t("action.add")}</button>
  </article>`).join("") : `<p class="peer-empty">${t("settings.noAvailableHosts")}</p>`;
}

function renderPairedRoutes(): void {
  const container = document.querySelector("#paired-routes");
  if (!container) return;
  const discoveryNote = `<p class="input-discovery-note">${settings.supportedInputs?.length ? t("settings.capabilitiesDetected", { count: inputOptions.length }) : t("settings.capabilitiesFallback")}</p>`;
  container.innerHTML = discoveryNote + (settings.peers.length ? `<p class="field-title">${t("settings.addedHosts")}</p>` + settings.peers.map((peer) => `<article class="paired-route-card">
    <div class="peer-identity">
      <strong>${escapeHtml(peer.name)}</strong>
      <span>${platformName(peer.platform)} · ${escapeHtml(peer.address)}</span>
      <div class="peer-diagnostic-actions" aria-label="${escapeHtml(t("settings.diagnosticAria", { name: peer.name }))}">
        <button class="text-button" type="button" data-probe-id="${escapeHtml(peer.id)}">${t("action.testConnection")}</button>
        <span class="tool-sep">·</span>
        <button class="text-button" type="button" data-wake-id="${escapeHtml(peer.id)}" ${peer.macAddress.trim() ? "" : "disabled"}>${t("action.sendWake")}</button>
      </div>
    </div>
    <div class="paired-route-right">
      <label class="paired-input-wrap">
        <span>${t("settings.inputValue")}</span>
        <select class="paired-input-field" data-route-input="${escapeHtml(peer.id)}">${renderInputOptions(peer.id, peer.input)}</select>
      </label>
      <button class="delete-button" type="button" data-remove-peer="${escapeHtml(peer.id)}" title="${t("action.remove")}"><i data-lucide="trash-2"></i></button>
    </div>
  </article>`).join("") : `<p class="peer-empty">${t("settings.noAddedHosts")}</p>`);
}

function renderInputOptions(routeId: string, current: number | null): string {
  const assignedElsewhere = new Set<number>();
  const localValue = inputValueFromElement("#local-input", settings.localInput);
  if (routeId !== "local" && localValue != null) assignedElsewhere.add(localValue);
  for (const peer of settings.peers) {
    const peerValue = inputValueFromElement(`[data-route-input="${cssEscape(peer.id)}"]`, peer.input);
    if (peer.id !== routeId && peerValue != null) assignedElsewhere.add(peerValue);
  }
  const options = inputOptions
    .filter((item) => !assignedElsewhere.has(item.value) || item.value === current)
    .map((item) => `<option value="${item.value}" ${item.value === current ? "selected" : ""}>${escapeHtml(localizedInputOptionName(item))}</option>`)
    .join("");
  return `<option value="" ${current == null ? "selected" : ""}>${t("settings.selectInput")}</option>${options}`;
}

function inputValueFromElement(selector: string, fallback: number | null): number | null {
  const value = document.querySelector<HTMLInputElement>(selector)?.value;
  if (value == null) return fallback;
  try { return parseInput(value); } catch { return null; }
}

function renderHostRoutes(): void {
  const container = document.querySelector("#host-route-grid");
  if (!container) return;
  const routes = [
    { id: "local", name: dashboard.localHost === "windows" ? t("dashboard.localWindows") : t("dashboard.localMac"), platform: dashboard.localHost, input: settings.localInput, local: true },
    ...settings.peers.map((peer) => ({ ...peer, local: false })),
  ];
  container.innerHTML = routes.map((route) => {
    const badgeText = route.local
      ? (route.platform === "mac" ? t("dashboard.localMacOs") : t("dashboard.localWindowsBadge"))
      : (route.platform === "mac" ? t("dashboard.connectedMacOs") : t("dashboard.connectedWindows"));
    const inputDesc = route.input == null
      ? t("dashboard.inputUnset")
      : (route.local ? t("dashboard.currentInput", { input: escapeHtml(inputName(route.input)) }) : t("dashboard.assignedInput", { input: escapeHtml(inputName(route.input)) }));
    const iconName = route.platform === "mac" ? "laptop" : "computer";

    return `
      <article class="host-route-card ${route.local ? "is-local" : ""}">
        <div class="host-card-header">
          <div class="host-icon ${route.platform}">
            <i data-lucide="${iconName}"></i>
          </div>
          <div class="host-copy">
            <span class="host-label ${route.local ? "is-local" : ""}">${badgeText}</span>
            <h2 class="host-title">${escapeHtml(route.name)}</h2>
            <p class="host-input-desc">${inputDesc}</p>
          </div>
        </div>
        ${route.local ? `
          <div class="local-active-state">
            <span>${t("dashboard.currentlyDisplayed")}</span>
            <span class="active-toggle-indicator"></span>
          </div>
        ` : `
          <button class="switch-button primary" data-switch-id="${escapeHtml(route.id)}" ${route.input == null || (!dashboard.ddcAvailable && !dashboard.agentConfigured) ? "disabled" : ""}>
            <i data-lucide="arrow-left-right"></i>
            <span>${t("action.switchHost")}</span>
          </button>
        `}
      </article>
    `;
  }).join("");
}

function renderInputHints(): void {
  const local = document.querySelector<HTMLSelectElement>("#local-input");
  setText("#local-input-name", labelForCode(local?.value ?? ""));
  document.querySelectorAll<HTMLSelectElement>("[data-route-input]").forEach((input) => {
    const routeId = input.dataset.routeInput ?? "";
    const current = inputValueFromElement(`[data-route-input="${cssEscape(routeId)}"]`, null);
    input.innerHTML = renderInputOptions(routeId, current);
    input.value = current == null ? "" : String(current);
  });
}

async function scanPeers(): Promise<void> {
  const button = document.querySelector<HTMLButtonElement>("#scan-button");
  if (button) { button.disabled = true; button.textContent = t("action.searching"); }
  try {
    discoveredPeers = isPreview ? [] : await invoke<DiscoveredPeer[]>("discover_peers");
    renderPeerList(); refreshIcons();
    if (!discoveredPeers.length) showToast(t("toast.noPeersTitle"), t("toast.noPeersBody"), true);
  } catch (error) { showToast(t("toast.scanFailed"), String(error), true); }
  finally { if (button) { button.disabled = false; button.textContent = t("action.searchAgain"); } }
}

async function selectMonitor(monitorId: string): Promise<void> {
  try {
    settings = await invoke<AppSettings>("select_monitor", { monitorId });
    await refresh(); showToast(t("toast.monitorSelected"), t("toast.monitorSelectedBody", { name: settings.sharedMonitor?.name ?? t("dashboard.sharedDisplay") }));
  } catch (error) { showToast(t("toast.monitorSelectFailed"), String(error), true); }
}

async function addPeer(peerId: string): Promise<void> {
  try { settings = await invoke<AppSettings>("select_peer", { peerId }); renderState(); showToast(t("toast.peerAdded"), t("toast.peerAddedBody")); }
  catch (error) { showToast(t("toast.peerAddFailed"), String(error), true); }
}

async function removePeer(peerId: string): Promise<void> {
  try { settings = await invoke<AppSettings>("remove_peer", { peerId }); renderState(); }
  catch (error) { showToast(t("toast.peerRemoveFailed"), String(error), true); }
}

async function saveSettings(event: SubmitEvent): Promise<void> {
  event.preventDefault();
  try {
    const localInput = parseInput(document.querySelector<HTMLSelectElement>("#local-input")?.value ?? "");
    settings = {
      ...settings, localInput,
      peers: settings.peers.map((peer) => ({ ...peer, input: parseInput(document.querySelector<HTMLSelectElement>(`[data-route-input="${cssEscape(peer.id)}"]`)?.value ?? "") })),
      sharedKey: document.querySelector<HTMLInputElement>("#shared-key")?.value ?? "",
      waitSeconds: Number(document.querySelector<HTMLInputElement>("#wait-seconds")?.value ?? 45),
      autostart: document.querySelector<HTMLInputElement>("#autostart")?.checked ?? true,
      checkUpdates: document.querySelector<HTMLInputElement>("#check-updates")?.checked ?? true,
    };
    const result = await invoke<OperationResult>("save_settings", { settings });
    showToast(result.title, result.detail); await refresh();
  } catch (error) { showToast(t("toast.settingsFailed"), String(error), true); }
}

async function switchHost(targetId: string): Promise<void> {
  showOperation(t("operation.preparingTitle"), t("operation.preparingBody"));
  const onEvent = new Channel<SwitchProgressEvent>();
  onEvent.onmessage = (event) => {
    if (event.event === "waking") {
      showOperation(t("operation.wakingTitle", { name: event.peerName }), t("operation.wakingBody"));
    } else if (event.event === "checking") {
      showOperation(t("operation.checkingTitle", { name: event.peerName }), t("operation.checkingBody"));
    } else if (event.event === "waiting") {
      showOperation(t("operation.waitingTitle", { name: event.peerName }), t("operation.waitingBody", { seconds: event.seconds }));
    } else if (event.event === "remoteFallback") {
      showOperation(t("operation.remoteTitle", { name: event.peerName }), t("operation.remoteBody"));
    } else {
      showOperation(t("operation.switchingTitle"), t("operation.switchingBody"));
    }
  };
  try {
    const result = await invoke<OperationResult>("switch_host", { targetId, onEvent });
    showToast(result.title, result.detail, result.warning);
  } catch (error) {
    showToast(t("toast.switchFailed"), String(error), true);
  } finally {
    hideOperation();
  }
}

async function peerCommand(command: "probe_peer" | "wake_peer", peerId: string): Promise<void> {
  try { const result = await invoke<OperationResult>(command, { peerId }); showToast(result.title, result.detail); }
  catch (error) { showToast(command === "probe_peer" ? t("toast.probeFailed") : t("toast.wakeFailed"), String(error), true); }
}

async function checkForUpdates(manual: boolean): Promise<void> {
  if (isPreview) {
    if (manual) showToast(t("toast.updateUnavailable"), t("toast.updateUnavailableBody"), true);
    return;
  }
  const button = document.querySelector<HTMLButtonElement>("#update-button");
  button?.classList.add("is-checking");
  try {
    const update = await invoke<UpdateInfo>("check_for_update");
    pendingUpdate = update.available ? update : null;
    button?.classList.toggle("has-update", update.available);
    button?.setAttribute("title", update.available ? t("update.availableTooltip", { version: update.version ?? "" }) : t("action.checkUpdates"));
    if (update.available) {
      if (manual) showUpdateDialog(update);
      else showToast(t("update.availableTitle"), t("update.availableBody", { version: update.version ?? "" }));
    } else if (manual) {
      showToast(t("update.latestTitle"), t("update.latestBody", { version: update.currentVersion }));
    }
  } catch (error) {
    if (manual) showToast(t("toast.updateFailed"), String(error), true);
  } finally {
    button?.classList.remove("is-checking");
  }
}

function showUpdateDialog(update: UpdateInfo): void {
  setText("#update-title", `DisplayMux ${update.version ?? ""}`);
  setText("#update-version", t("update.currentVersion", { version: update.currentVersion }));
  setText("#update-notes", update.notes?.trim() || t("update.noneNotes"));
  const overlay = document.querySelector("#update-overlay");
  overlay?.classList.add("is-visible");
  overlay?.setAttribute("aria-hidden", "false");
}

function hideUpdateDialog(): void {
  const overlay = document.querySelector("#update-overlay");
  overlay?.classList.remove("is-visible");
  overlay?.setAttribute("aria-hidden", "true");
}

async function installUpdate(): Promise<void> {
  const installButton = document.querySelector<HTMLButtonElement>("#update-install");
  const cancelButton = document.querySelector<HTMLButtonElement>("#update-cancel");
  const progress = document.querySelector<HTMLElement>("#update-progress");
  if (installButton) { installButton.disabled = true; installButton.textContent = t("update.preparing"); }
  if (cancelButton) cancelButton.disabled = true;
  if (progress) progress.hidden = false;
  const onEvent = new Channel<UpdateDownloadEvent>();
  onEvent.onmessage = (event) => {
    if (event.event === "started") {
      setText("#update-progress-label", t("update.downloadingSigned"));
    } else if (event.event === "progress") {
      const percent = event.contentLength ? Math.min(100, Math.round(event.downloaded / event.contentLength * 100)) : 0;
      const bar = document.querySelector<HTMLElement>("#update-progress-bar");
      if (bar) bar.style.width = event.contentLength ? `${percent}%` : "35%";
      setText("#update-progress-label", event.contentLength ? t("update.downloaded", { percent }) : t("update.downloading"));
    } else {
      setText("#update-progress-label", t("update.installing"));
    }
  };
  try {
    await invoke("install_update", { onEvent });
  } catch (error) {
    showToast(t("toast.installFailed"), String(error), true);
    if (installButton) { installButton.disabled = false; installButton.textContent = t("update.retry"); }
    if (cancelButton) cancelButton.disabled = false;
  }
}

function parseInput(value: string): number | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  const parsed = /^0x/i.test(trimmed) ? Number.parseInt(trimmed.slice(2), 16) : Number.parseInt(trimmed, 10);
  if (!Number.isInteger(parsed) || parsed < 1 || parsed > 255) throw new Error(t("input.parseError", { value: trimmed }));
  return parsed;
}

function inputName(value: number): string {
  const known = inputOptions.find((item) => item.value === value);
  return known ? localizedInputOptionName(known) : t("input.other");
}
function localizedInputOptionName(input: InputOption): string {
  const localized = new Map<number, string>([
    [0x05, t("input.composite1")], [0x06, t("input.composite2")],
    [0x09, t("input.tuner1")], [0x0a, t("input.tuner2")], [0x0b, t("input.tuner3")],
    [0x0c, t("input.component1")], [0x0d, t("input.component2")], [0x0e, t("input.component3")],
  ]);
  return localized.get(input.value) ?? input.name;
}
function labelForCode(value: string): string {
  try { const parsed = parseInput(value); return parsed == null ? t("input.unset") : inputName(parsed); }
  catch { return t("input.invalid"); }
}
function platformName(value: Platform): string { return value === "mac" ? "macOS" : "Windows"; }
function isUltrawideResolution(value: MonitorResolution): boolean { return value.height > 0 && value.width >= value.height * 2; }
function resolutionSourceName(value: ResolutionSource | null): string {
  if (value === "edid") return "EDID";
  if (value === "coreGraphicsDisplayMode") return t("resolution.coreGraphics");
  if (value === "windowsDisplayMode") return t("resolution.windows");
  return t("resolution.unknown");
}
function sameFingerprint(left: Fingerprint, right: Fingerprint): boolean { return left.manufacturer_id.toUpperCase() === right.manufacturer_id.toUpperCase() && left.product_code.toUpperCase() === right.product_code.toUpperCase() && left.serial_number === right.serial_number; }
function escapeHtml(value: string): string { return value.replace(/[&<>'"]/g, (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;" })[char] ?? char); }
function cssEscape(value: string): string { return typeof CSS !== "undefined" && CSS.escape ? CSS.escape(value) : value.replace(/["\\]/g, "\\$&"); }
function setText(selector: string, value: string): void { const element = document.querySelector(selector); if (element) element.textContent = value; }
function setInput(selector: string, value: string): void { const element = document.querySelector<HTMLInputElement>(selector); if (element) element.value = value; }
function showOperation(title: string, detail?: string): void {
  setText("#operation-title", title);
  if (detail) setText("#operation-detail", detail);
  document.querySelector("#operation-overlay")?.classList.add("is-visible");
}
function hideOperation(): void { document.querySelector("#operation-overlay")?.classList.remove("is-visible"); }
let toastTimer = 0;
function showToast(title: string, detail: string, warning = false): void {
  const toast = document.querySelector("#toast"); if (!toast) return;
  window.clearTimeout(toastTimer); setText("#toast-title", title); setText("#toast-detail", detail);
  toast.classList.toggle("is-warning", warning); toast.classList.add("is-visible");
  toastTimer = window.setTimeout(() => toast.classList.remove("is-visible"), 5200);
}

async function bootstrap(): Promise<void> {
  if (!isPreview) {
    try { await invoke("set_locale", { locale }); } catch { /* Preview mode has no Tauri backend. */ }
  }
  await Promise.all([refresh(), renderAppVersion()]);
  if (settings.checkUpdates && !isPreview) window.setTimeout(() => void checkForUpdates(false), 1800);
}

async function renderAppVersion(): Promise<void> {
  try {
    setText("#app-version", `v${await getVersion()}`);
  } catch {
    setText("#app-version", `v${packageMetadata.version}`);
  }
}

void bootstrap();
