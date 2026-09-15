import "@fontsource-variable/manrope";
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { locale, t } from "./i18n";
import "./host-switcher.css";

type Platform = "windows" | "mac";
type SwitchProgressEvent =
  | { event: "waking"; peerName: string }
  | { event: "checking"; peerName: string }
  | { event: "waiting"; peerName: string; seconds: number }
  | { event: "switching" }
  | { event: "remoteFallback"; peerName: string };

interface HostOption {
  id: string;
  name: string;
  platform: Platform;
  inputName: string | null;
  isLocal: boolean;
  available: boolean;
}

interface HostSwitcherState {
  sharedMonitorName: string | null;
  hosts: HostOption[];
}

interface OperationResult {
  title: string;
  detail: string;
}

const root = document.querySelector<HTMLElement>("#host-switcher-app")!;
if (!root) throw new Error("DisplayMux host switcher root was not found");

let state: HostSwitcherState = { sharedMonitorName: null, hosts: [] };
let selectedIndex = 0;
let switching = false;
let hideTimer: number | null = null;

function escapeHtml(value: string): string {
  return value.replace(/[&<>'"]/g, (character) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", "'": "&#39;", "\"": "&quot;",
  })[character] ?? character);
}

function platformLabel(platform: Platform): string {
  return platform === "mac" ? "macOS" : "Windows";
}

function render(message?: { title: string; detail: string; error?: boolean }): void {
  root.innerHTML = `
    <main class="switcher-shell" aria-labelledby="switcher-title">
      <header class="switcher-header">
        <div>
          <p class="eyebrow">DISPLAYMUX</p>
          <h1 id="switcher-title">${t("switcher.title")}</h1>
          <p>${escapeHtml(state.sharedMonitorName ?? t("switcher.noDisplay"))}</p>
        </div>
      </header>
      <section class="host-list" role="listbox" aria-label="${t("switcher.hostListAria")}">
        ${state.hosts.map((host, index) => `
          <button type="button" class="host-option ${index === selectedIndex ? "is-selected" : ""}"
            data-host-index="${index}" role="option" aria-selected="${index === selectedIndex}"
            ${host.available && !switching ? "" : "disabled"}>
            <span class="platform-mark ${host.platform}">${host.platform === "mac" ? "M" : "W"}</span>
            <span class="host-copy">
              <strong>${escapeHtml(host.name)}</strong>
              <small>${platformLabel(host.platform)} · ${escapeHtml(host.inputName ?? t("switcher.inputUnset"))}</small>
            </span>
            <span class="host-status">${host.isLocal ? t("switcher.local") : t("switcher.select")}</span>
          </button>
        `).join("") || `<p class="empty-state">${t("switcher.noHosts")}</p>`}
      </section>
      ${message ? `<div class="switch-message ${message.error ? "is-error" : ""}" role="status"><strong>${escapeHtml(message.title)}</strong><span>${escapeHtml(message.detail)}</span></div>` : ""}
      <footer>
        <span>${t("switcher.navigationHint")}</span>
        <span>${t("switcher.closeHint")}</span>
      </footer>
    </main>`;
}

function nextAvailableIndex(direction: 1 | -1): number {
  if (!state.hosts.some((host) => host.available)) return selectedIndex;
  let candidate = selectedIndex;
  do {
    candidate = (candidate + direction + state.hosts.length) % state.hosts.length;
  } while (!state.hosts[candidate].available);
  return candidate;
}

function selectIndex(index: number): void {
  if (!state.hosts[index]?.available || switching) return;
  selectedIndex = index;
  render();
  document.querySelector<HTMLElement>(`[data-host-index="${index}"]`)?.focus();
}

async function hideSwitcher(): Promise<void> {
  try { await invoke("hide_host_switcher"); } catch { window.close(); }
}

async function switchToSelected(): Promise<void> {
  const host = state.hosts[selectedIndex];
  if (!host?.available || switching) return;
  switching = true;
  render({ title: t("switcher.preparing"), detail: t("switcher.preparingDetail") });
  const onEvent = new Channel<SwitchProgressEvent>();
  onEvent.onmessage = (event) => {
    const detail = event.event === "waking"
      ? t("switcher.waking", { name: event.peerName })
      : event.event === "waiting"
        ? t("switcher.waiting", { name: event.peerName, seconds: event.seconds })
        : event.event === "remoteFallback"
          ? t("switcher.remoteFallback", { name: event.peerName })
          : t("switcher.switching");
    render({ title: t("switcher.preparing"), detail });
  };
  try {
    const result = await invoke<OperationResult>("switch_host", { targetId: host.id, onEvent });
    render({ title: result.title, detail: result.detail });
    hideTimer = window.setTimeout(() => {
      hideTimer = null;
      void hideSwitcher();
    }, 450);
  } catch (error) {
    switching = false;
    render({ title: t("switcher.failed"), detail: String(error), error: true });
  }
}

root.addEventListener("click", (event) => {
  const option = (event.target as HTMLElement).closest<HTMLButtonElement>("[data-host-index]");
  if (!option) return;
  const index = Number(option.dataset.hostIndex);
  if (Number.isInteger(index)) {
    selectedIndex = index;
    void switchToSelected();
  }
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    event.preventDefault();
    void hideSwitcher();
    return;
  }
  if (switching) return;
  if (event.key === "Tab" || event.key === "ArrowDown" || event.key === "ArrowUp") {
    event.preventDefault();
    const backwards = event.key === "ArrowUp" || (event.key === "Tab" && event.shiftKey);
    selectIndex(nextAvailableIndex(backwards ? -1 : 1));
  } else if (event.key === "Enter") {
    event.preventDefault();
    void switchToSelected();
  }
});

async function initialize(): Promise<void> {
  if (hideTimer !== null) {
    window.clearTimeout(hideTimer);
    hideTimer = null;
  }
  switching = false;
  render();
  try {
    await invoke("set_locale", { locale });
    state = await invoke<HostSwitcherState>("get_host_switcher_state");
  } catch {
    state = {
      sharedMonitorName: t("switcher.previewDisplay"),
      hosts: [
        { id: "local", name: t("switcher.previewWindows"), platform: "windows", inputName: "HDMI 1", isLocal: true, available: true },
        { id: "peer", name: t("switcher.previewMac"), platform: "mac", inputName: "DisplayPort", isLocal: false, available: true },
      ],
    };
  }
  selectedIndex = Math.max(0, state.hosts.findIndex((host) => host.available));
  render();
}

async function bootstrap(): Promise<void> {
  try {
    await listen("host-switcher-shown", () => void initialize());
  } catch {
    // Browser previews do not expose Tauri's event API.
  }
  await initialize();
}

void bootstrap();
