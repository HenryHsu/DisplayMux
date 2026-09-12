import "@fontsource-variable/manrope";
import {
  Activity, ArrowLeftRight, CircleHelp, Computer, createIcons, Download, KeyRound, Laptop,
  ExternalLink, Github, Monitor, MoonStar, Network, Plus, RefreshCw, Save, Search, Settings,
  ShieldCheck, Trash2, UserRound, Zap,
} from "lucide";
import { getVersion } from "@tauri-apps/api/app";
import { Channel, invoke } from "@tauri-apps/api/core";
import packageMetadata from "../package.json";
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

interface InputOption { value: number; code: string; name: string; }
interface OperationResult { title: string; detail: string; peerWoken: boolean; }
interface UpdateInfo { available: boolean; currentVersion: string; version: string | null; notes: string | null; }
type UpdateDownloadEvent =
  | { event: "started"; contentLength: number | null }
  | { event: "progress"; downloaded: number; contentLength: number | null }
  | { event: "finished" };

const standardInputs: InputOption[] = [
  [0x01, "VGA 1"], [0x02, "VGA 2"], [0x03, "DVI 1"], [0x04, "DVI 2"],
  [0x05, "Composite Video 1"], [0x06, "Composite Video 2"],
  [0x07, "S-Video 1"], [0x08, "S-Video 2"], [0x09, "Tuner 1"],
  [0x0a, "Tuner 2"], [0x0b, "Tuner 3"], [0x0c, "Component Video 1"],
  [0x0d, "Component Video 2"], [0x0e, "Component Video 3"],
  [0x0f, "DisplayPort 1"], [0x10, "DisplayPort 2"], [0x11, "HDMI 1"], [0x12, "HDMI 2"],
].map(([value, name]) => ({ value: value as number, code: codeFor(value as number), name: name as string }));

const previewSettings: AppSettings = {
  localHost: "windows", sharedMonitor: null, localInput: null, peers: [],
  broadcastIp: "255.255.255.255", wakePort: 9, sharedKey: "", waitSeconds: 45, autostart: true, checkUpdates: true,
};
const previewDashboard: DashboardState = {
  platform: "windows", localHost: "windows", agentConfigured: false, ddcAvailable: false,
  monitorStatus: "請在設定頁選擇共用螢幕", selectionNotice: null, monitors: [],
};

let settings = previewSettings;
let dashboard = previewDashboard;
let discoveredPeers: DiscoveredPeer[] = [];
let inputOptions = standardInputs;
let isPreview = false;
let pendingUpdate: UpdateInfo | null = null;

const app = document.querySelector<HTMLDivElement>("#app");
if (!app) throw new Error("找不到 DisplayMux 應用程式根節點");

app.innerHTML = `
  <div class="app-shell">
    <aside class="sidebar" aria-label="主要導覽">
      <div class="brand-header">
        <div class="brand-icon"><i data-lucide="monitor"></i></div>
        <span class="brand-title">DisplayMux</span>
      </div>
      <nav class="sidebar-nav">
        <button class="nav-button is-active" data-page="dashboard"><i data-lucide="arrow-left-right"></i><span>切換中心</span></button>
        <button class="nav-button" data-page="settings"><i data-lucide="settings"></i><span>螢幕與主機</span></button>
      </nav>
      <button class="nav-button nav-bottom" data-page="help"><i data-lucide="circle-help"></i><span>使用說明</span></button>
    </aside>
    <main class="workspace">
      <header class="topbar">
        <h1 id="page-title">共用螢幕切換中心</h1>
        <div class="topbar-actions">
          <div class="agent-pill" id="agent-pill"><span class="status-dot"></span><span>網路 Agent 讀取中</span></div>
          <button class="icon-button" id="update-button" title="檢查更新"><i data-lucide="download"></i></button>
          <button class="icon-button" id="refresh-button" title="重新整理"><i data-lucide="refresh-cw"></i></button>
        </div>
      </header>

      <section class="page is-active" id="dashboard-page">
        <div class="showcase-monitor-card">
          <div class="showcase-header">
            <span class="showcase-title">共用螢幕</span>
            <div class="showcase-badges">
              <span class="status-badge subtle" id="screen-ratio">16:9</span>
              <span class="status-badge" id="screen-input">DDC/CI 已就緒</span>
            </div>
          </div>
          <div class="flat-monitor-wrap" id="flat-monitor-wrap"></div>
          <div class="showcase-info">
            <strong class="showcase-monitor-name" id="shared-monitor-name">尚未選擇</strong>
            <p class="showcase-monitor-desc" id="monitor-status">以 EDID 製造商、產品碼與序號鎖定，不依顯示器排列順序。</p>
          </div>
        </div>

        <div class="host-route-grid" id="host-route-grid"></div>

        <section class="status-summary-bar">
          <div class="summary-item"><span>共用螢幕：</span><strong id="monitor-health" class="text-accent">正在偵測</strong></div>
          <span class="summary-pipe"></span>
          <div class="summary-item"><span>已加入主機：</span><strong id="peer-health">0 台</strong></div>
          <span class="summary-pipe"></span>
          <div class="summary-item"><span>喚醒支援：</span><strong id="wake-health" class="text-accent">尚未加入主機</strong></div>
        </section>
      </section>

      <section class="page" id="settings-page">
        <div class="settings-layout">
          <section class="settings-main">
            <form id="settings-form">
              <div class="form-section first">
                <div class="pairing-heading">
                  <strong>1. 選擇唯一的共用螢幕</strong>
                </div>
                <div class="monitor-picker" id="monitor-picker"></div>
              </div>

              <div class="form-section two-columns">
                <label class="field"><span>這台電腦</span><input id="local-host-name" disabled /></label>
                <label class="field"><span>這台電腦連接的輸入值</span><input id="local-input" list="input-values" placeholder="例如 0x0F" /><small id="local-input-name">尚未設定</small></label>
              </div>

              <div class="form-section pairing-section">
                <div class="pairing-heading">
                  <strong>2. 加入同網路的其他主機</strong>
                  <button class="scan-button" id="scan-button" type="button"><i data-lucide="search"></i>重新搜尋</button>
                </div>
                <div class="peer-list" id="peer-list"></div>
                <div class="paired-routes" id="paired-routes"></div>
              </div>

              <div class="form-section two-columns">
                <label class="field"><span>配對密碼</span><div class="input-wrap"><i data-lucide="key-round"></i><input id="shared-key" type="password" minlength="8" placeholder="至少 8 個字元" /></div><small>所有主機請填入完全相同的內容。</small></label>
                <label class="field compact"><span>喚醒等待秒數 (45 秒)</span><input id="wait-seconds" type="number" min="5" max="120" /><small>逾時後不切換，避免黑畫面。</small></label>
              </div>

              <div class="toggles-section">
                <label class="switch-row">
                  <span class="switch-label">
                    <strong>登入後自動啟動</strong>
                    <small>讓其他主機能搜尋、喚醒並要求這台電腦代為切換。</small>
                  </span>
                  <input id="autostart" type="checkbox" class="toggle-checkbox" />
                  <span class="switch-slider"></span>
                </label>
                <label class="switch-row">
                  <span class="switch-label">
                    <strong>啟動後自動檢查更新</strong>
                    <small>只向 GitHub Releases 取得版本資訊；下載與安裝前仍會要求確認。</small>
                  </span>
                  <input id="check-updates" type="checkbox" class="toggle-checkbox" />
                  <span class="switch-slider"></span>
                </label>
              </div>

              <div class="form-actions">
                <button class="save-button full-width" type="submit"><i data-lucide="save"></i>儲存設定</button>
              </div>
            </form>
          </section>

          <aside class="compatibility-panel">
            <h3>輸入值判讀</h3>
            <div class="path-item"><span class="path-badge">01</span><div><strong>MCCS 標準通訊規範</strong><p>標準值自動命名（如 0x0F 為 DisplayPort 1，0x11 為 HDMI 1）。</p></div></div>
            <div class="path-item"><span class="path-badge">02</span><div><strong>DDC/CI 協議</strong><p>USB-C 等輸入可能使用廠商自訂值，DisplayMux 會保留原碼顯示自訂輸入。</p></div></div>
            <div class="path-item"><span class="path-badge">03</span><div><strong>多主機路由切換確認</strong><p>同一台共用螢幕可為每台已加入主機保存不同輸入值，確保切換安全。</p></div></div>
            <div class="compat-note"><i data-lucide="shield-check"></i><p>即使更換螢幕，也只會控制選取的 EDID 指紋，保護其他獨立工作螢幕安全。</p></div>
          </aside>
        </div>
      </section>

      <section class="page" id="help-page">
        <div class="help-content">
          <p class="section-kicker">OPERATING NOTES</p><h2>安全與相容性說明</h2>
          <div class="note-list">
            <article><span>01</span><div><h3>更換螢幕</h3><p>更換後請重新選擇共用螢幕。舊指紋找不到時，DisplayMux 會停止而不會改動其他螢幕。</p></div></article>
            <article><span>02</span><div><h3>輸入值</h3><p>DisplayMux 使用 DDC/CI VCP 0x60。常見值可自動命名，但廠商自訂值應依螢幕選單或說明書確認。</p></div></article>
            <article><span>03</span><div><h3>安全切換與直接切換</h3><p>安全切換會先確認或喚醒目標主機；直接切換只使用本機 DDC/CI，不需要網路，但對端離線時可能黑畫面。</p></div></article>
            <article><span>04</span><div><h3>MacBook 轉接器</h3><p>若 USB-C 或 HDMI 轉接器未轉送 DDC，可由另一台已配對、可控制螢幕的主機代為切換。</p></div></article>
          </div>

          <section class="about-section" aria-labelledby="about-title">
            <p class="section-kicker">ABOUT</p><h2 id="about-title">關於 DisplayMux</h2>
            <dl class="about-grid">
              <div class="about-item">
                <dt><i data-lucide="user-round"></i>開發者</dt>
                <dd>Henry Hsu</dd>
              </div>
              <div class="about-item">
                <dt><i data-lucide="github"></i>GitHub</dt>
                <dd><a href="https://github.com/HenryHsu/DisplayMux" target="_blank" rel="noopener noreferrer">HenryHsu/DisplayMux<i data-lucide="external-link"></i></a></dd>
              </div>
              <div class="about-item">
                <dt><i data-lucide="activity"></i>工具版本</dt>
                <dd id="app-version" aria-live="polite">讀取中</dd>
              </div>
            </dl>
          </section>
        </div>
      </section>
    </main>
  </div>
  <datalist id="input-values"></datalist>
  <div class="operation-overlay" id="operation-overlay" aria-live="polite" aria-hidden="true"><div class="operation-dialog"><div class="spinner"></div><p class="section-kicker">SAFE SWITCH</p><h2 id="operation-title">正在執行</h2><p>必要時會先確認或喚醒目標主機，再切換唯一指定的共用螢幕。</p></div></div>
  <div class="update-overlay" id="update-overlay" aria-hidden="true">
    <div class="update-dialog">
      <p class="section-kicker">SIGNED UPDATE</p>
      <h2 id="update-title">有可用更新</h2>
      <p id="update-version"></p>
      <div class="update-notes" id="update-notes"></div>
      <div class="update-progress" id="update-progress" hidden><div id="update-progress-bar"></div></div>
      <p class="update-progress-label" id="update-progress-label"></p>
      <div class="update-actions"><button class="scan-button" id="update-cancel" type="button">稍後</button><button class="save-button" id="update-install" type="button"><i data-lucide="download"></i>下載並安裝</button></div>
    </div>
  </div>
  <div class="toast" id="toast" role="status" aria-live="polite"><i data-lucide="zap"></i><div><strong id="toast-title"></strong><span id="toast-detail"></span></div></div>
`;

const iconSet = { Activity, ArrowLeftRight, CircleHelp, Computer, Download, ExternalLink, Github, KeyRound, Laptop, Monitor, MoonStar, Network, Plus, RefreshCw, Save, Search, Settings, ShieldCheck, Trash2, UserRound, Zap };
const refreshIcons = () => createIcons({ icons: iconSet });
refreshIcons();

const pageTitles: Record<string, string> = { dashboard: "共用螢幕切換中心", settings: "螢幕與主機設定", help: "使用說明" };
document.querySelectorAll<HTMLButtonElement>("[data-page]").forEach((button) => button.addEventListener("click", () => showPage(button.dataset.page ?? "dashboard")));
document.querySelector<HTMLButtonElement>("#refresh-button")?.addEventListener("click", () => void refresh());
document.querySelector<HTMLButtonElement>("#update-button")?.addEventListener("click", () => pendingUpdate ? showUpdateDialog(pendingUpdate) : void checkForUpdates(true));
document.querySelector<HTMLButtonElement>("#update-cancel")?.addEventListener("click", hideUpdateDialog);
document.querySelector<HTMLButtonElement>("#update-install")?.addEventListener("click", () => void installUpdate());
document.querySelector<HTMLButtonElement>("#scan-button")?.addEventListener("click", () => void scanPeers());
document.querySelector<HTMLFormElement>("#settings-form")?.addEventListener("submit", (event) => void saveSettings(event));
document.querySelector<HTMLInputElement>("#local-input")?.addEventListener("input", renderInputHints);
document.querySelector("#monitor-picker")?.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-monitor-id]");
  if (button?.dataset.monitorId) void selectMonitor(button.dataset.monitorId);
});
document.querySelector("#peer-list")?.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-add-peer]");
  if (button?.dataset.addPeer) void addPeer(button.dataset.addPeer);
});
document.querySelector("#paired-routes")?.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-remove-peer]");
  if (button?.dataset.removePeer) void removePeer(button.dataset.removePeer);
});
document.querySelector("#paired-routes")?.addEventListener("input", renderInputHints);
document.querySelector("#host-route-grid")?.addEventListener("click", (event) => {
  const button = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-switch-id], [data-direct-switch-id], [data-probe-id], [data-wake-id]");
  if (button?.dataset.switchId) void switchHost(button.dataset.switchId);
  if (button?.dataset.directSwitchId) void directSwitchHost(button.dataset.directSwitchId);
  if (button?.dataset.probeId) void peerCommand("probe_peer", button.dataset.probeId);
  if (button?.dataset.wakeId) void peerCommand("wake_peer", button.dataset.wakeId);
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
    showToast("共用螢幕已自動更新", dashboard.selectionNotice);
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
    setText("#shared-monitor-name", "尚未選擇");
    setText("#monitor-status", dashboard.monitorStatus || "請在「螢幕與主機」設定頁選擇共用螢幕");
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

  setText("#screen-input", dashboard.ddcAvailable ? "DDC/CI 已就緒" : "尚未就緒");
  setText("#monitor-health", dashboard.ddcAvailable ? "已鎖定" : "尚未就緒");
  setText("#peer-health", `${settings.peers.length} 台`);
  setText("#wake-health", settings.peers.some((peer) => peer.macAddress) ? "正常" : (settings.peers.length ? "無 MAC 資料" : "尚未加入主機"));
  const pill = document.querySelector("#agent-pill");
  pill?.classList.toggle("is-ready", dashboard.agentConfigured);
  if (pill) pill.querySelector("span:last-child")!.textContent = isPreview ? "介面預覽" : dashboard.agentConfigured ? "網路 Agent 已設定" : "網路 Agent 未設定（不影響 DDC）";
  setInput("#local-host-name", dashboard.localHost === "windows" ? "這台 Windows PC" : "這台 Mac");
  setInput("#local-input", settings.localInput == null ? "" : codeFor(settings.localInput));
  setInput("#shared-key", settings.sharedKey);
  setInput("#wait-seconds", String(settings.waitSeconds));
  const autostart = document.querySelector<HTMLInputElement>("#autostart");
  if (autostart) autostart.checked = settings.autostart;
  const checkUpdates = document.querySelector<HTMLInputElement>("#check-updates");
  if (checkUpdates) checkUpdates.checked = settings.checkUpdates;
  const datalist = document.querySelector("#input-values");
  if (datalist) datalist.innerHTML = inputOptions.map((item) => `<option value="${item.code}">${escapeHtml(item.name)}</option>`).join("");
  renderMonitors(); renderPeerList(); renderPairedRoutes(); renderHostRoutes(); renderInputHints(); refreshIcons();
}

function renderMonitors(): void {
  const container = document.querySelector("#monitor-picker");
  if (!container) return;
  if (!dashboard.monitors.length) {
    container.innerHTML = `<p class="peer-empty">沒有找到可選擇的 DDC/CI 螢幕</p>`; return;
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
          <span>${escapeHtml(fp.manufacturer_id)} / ${escapeHtml(fp.product_code)} / ${escapeHtml(fp.serial_number ?? "無序號")} ${resText} (DDC/CI 可控制)</span>
        </div>
      </div>
      <button type="button" class="monitor-select-btn ${isSelected ? "is-selected" : ""}" data-monitor-id="${escapeHtml(monitor.id)}" ${isSelected ? "disabled" : ""}>
        ${isSelected ? "已選取" : "設為共用"}
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
      <span>${platformName(peer.platform)} · 自動取得網路資訊</span>
    </div>
    <button type="button" class="peer-add-btn" data-add-peer="${escapeHtml(peer.id)}"><i data-lucide="plus"></i>加入</button>
  </article>`).join("") : `<p class="peer-empty">沒有尚未加入的 DisplayMux 主機</p>`;
}

function renderPairedRoutes(): void {
  const container = document.querySelector("#paired-routes");
  if (!container) return;
  container.innerHTML = settings.peers.length ? `<p class="field-title">已加入的主機與輸入</p>` + settings.peers.map((peer) => `<article class="paired-route-card">
    <div class="peer-identity">
      <strong>${escapeHtml(peer.name)}</strong>
      <span>${platformName(peer.platform)} · ${escapeHtml(peer.address)}</span>
    </div>
    <div class="paired-route-right">
      <label class="paired-input-wrap">
        <span>輸入值:</span>
        <input class="paired-input-field" data-route-input="${escapeHtml(peer.id)}" list="input-values" value="${peer.input == null ? "" : codeFor(peer.input)}" placeholder="例如 0x11"/>
      </label>
      <button class="delete-button" type="button" data-remove-peer="${escapeHtml(peer.id)}" title="移除"><i data-lucide="trash-2"></i></button>
    </div>
  </article>`).join("") : `<p class="peer-empty">尚未加入其他主機</p>`;
}

function renderHostRoutes(): void {
  const container = document.querySelector("#host-route-grid");
  if (!container) return;
  const routes = [
    { id: "local", name: dashboard.localHost === "windows" ? "這台 Windows 電腦" : "這台 Mac", platform: dashboard.localHost, input: settings.localInput, local: true },
    ...settings.peers.map((peer) => ({ ...peer, local: false })),
  ];
  container.innerHTML = routes.map((route) => {
    const badgeText = route.local
      ? (route.platform === "mac" ? "本機 macOS" : "本機 Windows")
      : (route.platform === "mac" ? "已連線 macOS" : "已連線 Windows");
    const inputDesc = route.input == null
      ? "尚未設定輸入"
      : (route.local ? `目前輸入：${escapeHtml(inputName(route.input))}` : `指定輸入：${escapeHtml(inputName(route.input))}`);
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
            <span>目前顯示中</span>
            <span class="active-toggle-indicator"></span>
          </div>
        ` : `
          <button class="switch-button primary" data-switch-id="${escapeHtml(route.id)}" ${route.input == null || !dashboard.ddcAvailable ? "disabled" : ""}>
            <i data-lucide="arrow-left-right"></i>
            <span>安全切換至此主機</span>
          </button>
          <div class="route-tools">
            <button class="text-button" type="button" data-direct-switch-id="${escapeHtml(route.id)}" ${route.input == null || !dashboard.ddcAvailable ? "disabled" : ""}>直接切換</button>
            <span class="tool-sep">·</span>
            <button class="text-button" type="button" data-probe-id="${escapeHtml(route.id)}">測試連線</button>
            <span class="tool-sep">·</span>
            <button class="text-button" type="button" data-wake-id="${escapeHtml(route.id)}">送出喚醒</button>
          </div>
        `}
      </article>
    `;
  }).join("");
}

function renderInputHints(): void {
  const local = document.querySelector<HTMLInputElement>("#local-input");
  setText("#local-input-name", labelForCode(local?.value ?? ""));
  document.querySelectorAll<HTMLInputElement>("[data-route-input]").forEach((input) => setText(`[data-input-hint="${cssEscape(input.dataset.routeInput ?? "")}"]`, labelForCode(input.value)));
}

async function scanPeers(): Promise<void> {
  const button = document.querySelector<HTMLButtonElement>("#scan-button");
  if (button) { button.disabled = true; button.textContent = "搜尋中"; }
  try {
    discoveredPeers = isPreview ? [] : await invoke<DiscoveredPeer[]>("discover_peers");
    renderPeerList(); refreshIcons();
    if (!discoveredPeers.length) showToast("尚未找到其他主機", "請在其他電腦開啟 DisplayMux，並確認位於相同私人網路。", true);
  } catch (error) { showToast("無法搜尋區域網路", String(error), true); }
  finally { if (button) { button.disabled = false; button.textContent = "重新搜尋"; } }
}

async function selectMonitor(monitorId: string): Promise<void> {
  try {
    settings = await invoke<AppSettings>("select_monitor", { monitorId });
    await refresh(); showToast("共用螢幕已選取", `${settings.sharedMonitor?.name ?? "指定螢幕"} 是唯一控制目標。`);
  } catch (error) { showToast("無法選擇螢幕", String(error), true); }
}

async function addPeer(peerId: string): Promise<void> {
  try { settings = await invoke<AppSettings>("select_peer", { peerId }); renderState(); showToast("主機已加入", "請為這台主機指定螢幕輸入值並儲存。"); }
  catch (error) { showToast("無法加入主機", String(error), true); }
}

async function removePeer(peerId: string): Promise<void> {
  try { settings = await invoke<AppSettings>("remove_peer", { peerId }); renderState(); }
  catch (error) { showToast("無法移除主機", String(error), true); }
}

async function saveSettings(event: SubmitEvent): Promise<void> {
  event.preventDefault();
  try {
    const localInput = parseInput(document.querySelector<HTMLInputElement>("#local-input")?.value ?? "");
    settings = {
      ...settings, localInput,
      peers: settings.peers.map((peer) => ({ ...peer, input: parseInput(document.querySelector<HTMLInputElement>(`[data-route-input="${cssEscape(peer.id)}"]`)?.value ?? "") })),
      sharedKey: document.querySelector<HTMLInputElement>("#shared-key")?.value ?? "",
      waitSeconds: Number(document.querySelector<HTMLInputElement>("#wait-seconds")?.value ?? 45),
      autostart: document.querySelector<HTMLInputElement>("#autostart")?.checked ?? true,
      checkUpdates: document.querySelector<HTMLInputElement>("#check-updates")?.checked ?? true,
    };
    const result = await invoke<OperationResult>("save_settings", { settings });
    showToast(result.title, result.detail); await refresh();
  } catch (error) { showToast("設定未儲存", String(error), true); }
}

async function switchHost(targetId: string): Promise<void> {
  showOperation("正在確認主機與共用螢幕");
  try { const result = await invoke<OperationResult>("switch_host", { targetId, force: false }); showToast(result.title, result.detail); }
  catch (error) { showToast("安全切換未執行", String(error), true); }
  finally { hideOperation(); }
}

async function directSwitchHost(targetId: string): Promise<void> {
  if (!window.confirm("直接切換不會確認或喚醒目標主機。若對端離線，螢幕可能暫時黑畫面。仍要切換嗎？")) return;
  showOperation("正在直接切換本機 DDC/CI");
  try { const result = await invoke<OperationResult>("switch_host", { targetId, force: true }); showToast(result.title, result.detail, true); }
  catch (error) { showToast("直接切換失敗", String(error), true); }
  finally { hideOperation(); }
}

async function peerCommand(command: "probe_peer" | "wake_peer", peerId: string): Promise<void> {
  try { const result = await invoke<OperationResult>(command, { peerId }); showToast(result.title, result.detail); }
  catch (error) { showToast(command === "probe_peer" ? "連線測試失敗" : "喚醒失敗", String(error), true); }
}

async function checkForUpdates(manual: boolean): Promise<void> {
  if (isPreview) {
    if (manual) showToast("無法檢查更新", "請從已安裝的 DisplayMux 執行更新檢查。", true);
    return;
  }
  const button = document.querySelector<HTMLButtonElement>("#update-button");
  button?.classList.add("is-checking");
  try {
    const update = await invoke<UpdateInfo>("check_for_update");
    pendingUpdate = update.available ? update : null;
    button?.classList.toggle("has-update", update.available);
    button?.setAttribute("title", update.available ? `可更新至 ${update.version}` : "檢查更新");
    if (update.available) {
      if (manual) showUpdateDialog(update);
      else showToast("有可用更新", `版本 ${update.version} 已發布；按上方下載按鈕查看。`);
    } else if (manual) {
      showToast("已是最新版本", `目前版本 ${update.currentVersion}。`);
    }
  } catch (error) {
    if (manual) showToast("無法檢查更新", String(error), true);
  } finally {
    button?.classList.remove("is-checking");
  }
}

function showUpdateDialog(update: UpdateInfo): void {
  setText("#update-title", `DisplayMux ${update.version ?? ""}`);
  setText("#update-version", `目前版本 ${update.currentVersion}`);
  setText("#update-notes", update.notes?.trim() || "此版本未提供更新說明。安裝檔會先通過 DisplayMux 簽章驗證。 ");
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
  if (installButton) { installButton.disabled = true; installButton.textContent = "準備下載"; }
  if (cancelButton) cancelButton.disabled = true;
  if (progress) progress.hidden = false;
  const onEvent = new Channel<UpdateDownloadEvent>();
  onEvent.onmessage = (event) => {
    if (event.event === "started") {
      setText("#update-progress-label", "正在下載已簽章的更新套件");
    } else if (event.event === "progress") {
      const percent = event.contentLength ? Math.min(100, Math.round(event.downloaded / event.contentLength * 100)) : 0;
      const bar = document.querySelector<HTMLElement>("#update-progress-bar");
      if (bar) bar.style.width = event.contentLength ? `${percent}%` : "35%";
      setText("#update-progress-label", event.contentLength ? `已下載 ${percent}%` : "正在下載更新套件");
    } else {
      setText("#update-progress-label", "簽章驗證完成，正在安裝並重新啟動");
    }
  };
  try {
    await invoke("install_update", { onEvent });
  } catch (error) {
    showToast("更新未安裝", String(error), true);
    if (installButton) { installButton.disabled = false; installButton.textContent = "重試下載與安裝"; }
    if (cancelButton) cancelButton.disabled = false;
  }
}

function parseInput(value: string): number | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  const parsed = /^0x/i.test(trimmed) ? Number.parseInt(trimmed.slice(2), 16) : Number.parseInt(trimmed, 10);
  if (!Number.isInteger(parsed) || parsed < 1 || parsed > 255) throw new Error(`無法辨識輸入值「${trimmed}」；請輸入 0x01 至 0xFF`);
  return parsed;
}

function inputName(value: number): string {
  const known = inputOptions.find((item) => item.value === value);
  return known ? `${known.name} · ${known.code}` : `自訂輸入 · ${codeFor(value)}`;
}
function labelForCode(value: string): string {
  try { const parsed = parseInput(value); return parsed == null ? "尚未設定" : inputName(parsed); }
  catch { return "輸入格式無效"; }
}
function codeFor(value: number): string { return `0x${value.toString(16).toUpperCase().padStart(2, "0")}`; }
function platformName(value: Platform): string { return value === "mac" ? "macOS" : "Windows"; }
function isUltrawideResolution(value: MonitorResolution): boolean { return value.height > 0 && value.width >= value.height * 2; }
function resolutionSourceName(value: ResolutionSource | null): string {
  if (value === "edid") return "EDID";
  if (value === "coreGraphicsDisplayMode") return "CoreGraphics 實體像素";
  if (value === "windowsDisplayMode") return "Windows 顯示模式";
  return "來源未知";
}
function sameFingerprint(left: Fingerprint, right: Fingerprint): boolean { return left.manufacturer_id.toUpperCase() === right.manufacturer_id.toUpperCase() && left.product_code.toUpperCase() === right.product_code.toUpperCase() && left.serial_number === right.serial_number; }
function escapeHtml(value: string): string { return value.replace(/[&<>'"]/g, (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", '"': "&quot;" })[char] ?? char); }
function cssEscape(value: string): string { return typeof CSS !== "undefined" && CSS.escape ? CSS.escape(value) : value.replace(/["\\]/g, "\\$&"); }
function setText(selector: string, value: string): void { const element = document.querySelector(selector); if (element) element.textContent = value; }
function setInput(selector: string, value: string): void { const element = document.querySelector<HTMLInputElement>(selector); if (element) element.value = value; }
function showOperation(title: string): void { setText("#operation-title", title); document.querySelector("#operation-overlay")?.classList.add("is-visible"); }
function hideOperation(): void { document.querySelector("#operation-overlay")?.classList.remove("is-visible"); }
let toastTimer = 0;
function showToast(title: string, detail: string, warning = false): void {
  const toast = document.querySelector("#toast"); if (!toast) return;
  window.clearTimeout(toastTimer); setText("#toast-title", title); setText("#toast-detail", detail);
  toast.classList.toggle("is-warning", warning); toast.classList.add("is-visible");
  toastTimer = window.setTimeout(() => toast.classList.remove("is-visible"), 5200);
}

async function bootstrap(): Promise<void> {
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
