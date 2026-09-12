use std::{
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    str::FromStr,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, RwLock,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use displaymux_core::{
    AgentAction, AgentClient, AgentResponse, AgentServer, DestinationHost, DiscoveredPeer,
    DisplayInput, DisplayMuxError, DisplayMuxProfile, DisplayMuxService, MacAddress,
    MdnsPeerDiscovery, MonitorControl, MonitorDescriptor, MonitorFingerprint, PeerDiscovery,
    PeerEndpoint, ResolutionSource, SwitchMode, SwitchOutcome, WakeTarget, DEFAULT_AGENT_PORT,
};
use serde::{Deserialize, Serialize};
use tauri::{ipc::Channel, AppHandle, Manager, State};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt as AutostartManagerExt};
use tauri_plugin_updater::UpdaterExt;
use tokio::{sync::Mutex, time::sleep};

static NONCE_COUNTER: AtomicU64 = AtomicU64::new(1);
static UI_LOCALE: AtomicU64 = AtomicU64::new(0);
const MIN_SHARED_KEY_LENGTH: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UiLocale {
    English,
    TraditionalChinese,
}

impl UiLocale {
    fn current() -> Self {
        if UI_LOCALE.load(Ordering::Relaxed) == 1 {
            Self::TraditionalChinese
        } else {
            Self::English
        }
    }
}

fn locale_from_tag(locale: &str) -> UiLocale {
    let normalized = locale.to_ascii_lowercase();
    if normalized.starts_with("zh-tw")
        || normalized.starts_with("zh-hant")
        || normalized.starts_with("zh-hk")
        || normalized.starts_with("zh-mo")
    {
        UiLocale::TraditionalChinese
    } else {
        UiLocale::English
    }
}

fn ui_text(zh_tw: &'static str, en: &'static str) -> &'static str {
    match UiLocale::current() {
        UiLocale::TraditionalChinese => zh_tw,
        UiLocale::English => en,
    }
}

fn localized_input_name(input: DisplayInput) -> String {
    let standard_name = match (UiLocale::current(), input.value()) {
        (_, 0x0f) => Some("DP 1"),
        (_, 0x10) => Some("DP 2"),
        (_, 0x1b) => Some("Type-C"),
        (UiLocale::TraditionalChinese, 0x05) => Some("複合視訊 1"),
        (UiLocale::TraditionalChinese, 0x06) => Some("複合視訊 2"),
        (UiLocale::TraditionalChinese, 0x09) => Some("電視調諧器 1"),
        (UiLocale::TraditionalChinese, 0x0a) => Some("電視調諧器 2"),
        (UiLocale::TraditionalChinese, 0x0b) => Some("電視調諧器 3"),
        (UiLocale::TraditionalChinese, 0x0c) => Some("色差視訊 1"),
        (UiLocale::TraditionalChinese, 0x0d) => Some("色差視訊 2"),
        (UiLocale::TraditionalChinese, 0x0e) => Some("色差視訊 3"),
        _ => input.standard_name(),
    };
    match standard_name {
        Some(name) => name.to_owned(),
        None => match UiLocale::current() {
            UiLocale::TraditionalChinese => "其他輸入".to_owned(),
            UiLocale::English => "Other input".to_owned(),
        },
    }
}

#[tauri::command]
fn set_locale(locale: String, app: AppHandle) -> Result<(), String> {
    let selected = locale_from_tag(&locale);
    UI_LOCALE.store(
        u64::from(selected == UiLocale::TraditionalChinese),
        Ordering::Relaxed,
    );
    #[cfg(target_os = "windows")]
    if let Some(tray) = app.tray_by_id("displaymux") {
        use tauri::menu::MenuBuilder;
        let menu = MenuBuilder::new(&app)
            .text("tray-open", ui_text("開啟 DisplayMux", "Open DisplayMux"))
            .separator()
            .text("tray-quit", ui_text("結束 DisplayMux", "Quit DisplayMux"))
            .build()
            .map_err(user_error)?;
        tray.set_menu(Some(menu)).map_err(user_error)?;
    }
    #[cfg(not(target_os = "windows"))]
    let _ = app;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SelectedMonitor {
    name: String,
    fingerprint: MonitorFingerprint,
    #[serde(default)]
    max_resolution: Option<displaymux_core::MonitorResolution>,
    #[serde(default)]
    resolution_source: Option<ResolutionSource>,
}

impl From<&MonitorDescriptor> for SelectedMonitor {
    fn from(monitor: &MonitorDescriptor) -> Self {
        Self {
            name: monitor.name.clone(),
            fingerprint: monitor.fingerprint.clone(),
            max_resolution: monitor.max_resolution,
            resolution_source: monitor.resolution_source,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HostRoute {
    id: String,
    name: String,
    platform: DestinationHost,
    address: String,
    port: u16,
    mac_address: String,
    input: Option<DisplayInput>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct AppSettings {
    local_host: DestinationHost,
    shared_monitor: Option<SelectedMonitor>,
    local_input: Option<DisplayInput>,
    #[serde(default)]
    supported_inputs: Option<Vec<DisplayInput>>,
    peers: Vec<HostRoute>,
    broadcast_ip: String,
    wake_port: u16,
    shared_key: String,
    wait_seconds: u64,
    autostart: bool,
    check_updates: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            local_host: local_host(),
            shared_monitor: None,
            local_input: None,
            supported_inputs: None,
            peers: Vec::new(),
            broadcast_ip: "255.255.255.255".to_owned(),
            wake_port: 9,
            shared_key: String::new(),
            wait_seconds: 45,
            autostart: true,
            check_updates: true,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct LegacySettings {
    local_host: DestinationHost,
    peer_id: String,
    peer_name: String,
    peer_ip: String,
    peer_port: u16,
    peer_mac: String,
    broadcast_ip: String,
    wake_port: u16,
    shared_key: String,
    wait_seconds: u64,
    autostart: bool,
    check_updates: bool,
}

impl Default for LegacySettings {
    fn default() -> Self {
        Self {
            local_host: local_host(),
            peer_id: String::new(),
            peer_name: String::new(),
            peer_ip: String::new(),
            peer_port: DEFAULT_AGENT_PORT,
            peer_mac: String::new(),
            broadcast_ip: "255.255.255.255".to_owned(),
            wake_port: 9,
            shared_key: String::new(),
            wait_seconds: 45,
            autostart: true,
            check_updates: true,
        }
    }
}

struct AppRuntime {
    settings: Arc<RwLock<AppSettings>>,
    settings_path: PathBuf,
    agent_task: Mutex<Option<tauri::async_runtime::JoinHandle<()>>>,
    discovery: Option<MdnsPeerDiscovery>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardState {
    platform: &'static str,
    local_host: DestinationHost,
    agent_configured: bool,
    ddc_available: bool,
    monitor_status: String,
    selection_notice: Option<String>,
    monitors: Vec<MonitorDescriptor>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MonitorSelectionChange {
    SelectedOnlyMonitor {
        name: String,
    },
    ReplacedMissingMonitor {
        previous: String,
        replacement: String,
    },
    RefreshedMetadata,
}

struct MonitorInventory {
    detected: Vec<MonitorDescriptor>,
    controllable: Vec<MonitorDescriptor>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct InputOption {
    value: u32,
    name: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationResult {
    title: String,
    detail: String,
    peer_woken: bool,
    warning: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "event",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum SwitchProgress {
    Waking { peer_name: String },
    Checking { peer_name: String },
    Waiting { peer_name: String, seconds: u64 },
    Switching,
    RemoteFallback { peer_name: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum NetworkPreparation {
    NotRequired,
    Ready { wake_sent: bool },
    Unavailable { wake_sent: bool, reason: String },
}

impl NetworkPreparation {
    fn peer_woken(&self) -> bool {
        matches!(
            self,
            Self::Ready { wake_sent: true }
                | Self::Unavailable {
                    wake_sent: true,
                    ..
                }
        )
    }

    fn warning(&self) -> bool {
        matches!(self, Self::Unavailable { .. })
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct UpdateInfo {
    available: bool,
    current_version: String,
    version: Option<String>,
    notes: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(
    tag = "event",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum UpdateDownloadEvent {
    Started {
        content_length: Option<u64>,
    },
    Progress {
        downloaded: u64,
        content_length: Option<u64>,
    },
    Finished,
}

#[tauri::command]
async fn discover_peers(state: State<'_, AppRuntime>) -> Result<Vec<DiscoveredPeer>, String> {
    sleep(Duration::from_millis(700)).await;
    let discovery = state.discovery.as_ref().ok_or_else(|| {
        ui_text(
            "無法啟動區域網路搜尋；請確認防火牆允許 DisplayMux 使用私人網路",
            "Unable to start local network discovery. Allow DisplayMux through the firewall on private networks.",
        )
        .to_owned()
    })?;
    let peers = discovery.peers().map_err(core_user_error)?;
    refresh_paired_endpoints(&state, &peers)?;
    Ok(peers)
}

#[tauri::command]
fn select_peer(peer_id: String, state: State<'_, AppRuntime>) -> Result<AppSettings, String> {
    let discovery = state.discovery.as_ref().ok_or_else(|| {
        ui_text(
            "區域網路搜尋目前不可用",
            "Local network discovery is unavailable",
        )
        .to_owned()
    })?;
    let peer = discovery
        .peers()
        .map_err(core_user_error)?
        .into_iter()
        .find(|peer| peer.id == peer_id)
        .ok_or_else(|| {
            ui_text(
                "這台主機已離線，請重新搜尋後再試一次",
                "This host is offline. Search again and retry.",
            )
            .to_owned()
        })?;
    let mut settings = read_settings(&state)?;
    upsert_discovered_peer(&mut settings, &peer);
    store_settings(&state, settings)
}

#[tauri::command]
fn remove_peer(peer_id: String, state: State<'_, AppRuntime>) -> Result<AppSettings, String> {
    let mut settings = read_settings(&state)?;
    settings.peers.retain(|peer| peer.id != peer_id);
    store_settings(&state, settings)
}

#[tauri::command]
fn select_monitor(monitor_id: String, state: State<'_, AppRuntime>) -> Result<AppSettings, String> {
    let controller = platform_controller().map_err(core_user_error)?;
    let monitor = monitor_inventory(&controller)
        .map_err(core_user_error)?
        .controllable
        .into_iter()
        .find(|monitor| monitor.id.as_str() == monitor_id)
        .ok_or_else(|| {
            ui_text(
                "找不到這台螢幕，請重新整理後再選擇",
                "This display was not found. Refresh and select it again.",
            )
            .to_owned()
        })?;
    let mut settings = read_settings(&state)?;
    let monitor_changed = settings
        .shared_monitor
        .as_ref()
        .is_none_or(|selected| !selected.fingerprint.matches_exactly(&monitor.fingerprint));
    settings.shared_monitor = Some(SelectedMonitor::from(&monitor));
    if monitor_changed {
        settings.peers.iter_mut().for_each(|peer| peer.input = None);
    }
    refresh_selected_input_data(&controller, &monitor, &mut settings).map_err(core_user_error)?;
    store_settings(&state, settings)
}

#[tauri::command]
fn get_settings(state: State<'_, AppRuntime>) -> Result<AppSettings, String> {
    read_settings(&state)
}

#[tauri::command]
fn get_input_options(state: State<'_, AppRuntime>) -> Result<Vec<InputOption>, String> {
    let settings = read_settings(&state)?;
    let inputs = settings
        .supported_inputs
        .filter(|inputs| !inputs.is_empty())
        .unwrap_or_else(common_input_sources);
    Ok(inputs
        .into_iter()
        .map(|input| InputOption {
            value: input.value(),
            name: localized_input_name(input),
        })
        .collect())
}

#[tauri::command]
async fn save_settings(
    settings: AppSettings,
    state: State<'_, AppRuntime>,
    app: AppHandle,
) -> Result<OperationResult, String> {
    let protected = read_settings(&state)?;
    let mut settings = settings_for_current_build(settings);
    // Monitor identity and discovered input data are backend-owned. The webview may only
    // assign a filtered input to remote hosts; it cannot forge DDC discovery results.
    settings.local_host = protected.local_host;
    settings.shared_monitor = protected.shared_monitor;
    settings.local_input = protected.local_input;
    settings.supported_inputs = protected.supported_inputs;
    validate_settings(&settings).map_err(core_user_error)?;
    let enable_autostart = settings.autostart;
    store_settings(&state, settings)?;
    let autostart = app.autolaunch();
    let autostart_enabled = autostart.is_enabled().map_err(user_error)?;
    if enable_autostart != autostart_enabled {
        if enable_autostart {
            autostart.enable().map_err(user_error)?;
        } else {
            autostart.disable().map_err(user_error)?;
        }
    }
    restart_agent(&state).await?;
    Ok(OperationResult {
        title: ui_text("設定已儲存", "Settings saved").to_owned(),
        detail: ui_text(
            "共用螢幕、各主機輸入與配對設定已更新。",
            "The shared display, host inputs, and pairing settings were updated.",
        )
        .to_owned(),
        peer_woken: false,
        warning: false,
    })
}

#[tauri::command]
async fn check_for_update(app: AppHandle) -> Result<UpdateInfo, String> {
    let current_version = app.package_info().version.to_string();
    let update = app
        .updater()
        .map_err(update_error)?
        .check()
        .await
        .map_err(update_error)?;
    Ok(match update {
        Some(update) => UpdateInfo {
            available: true,
            current_version,
            version: Some(update.version),
            notes: update.body,
        },
        None => UpdateInfo {
            available: false,
            current_version,
            version: None,
            notes: None,
        },
    })
}

#[tauri::command]
async fn install_update(
    app: AppHandle,
    on_event: Channel<UpdateDownloadEvent>,
) -> Result<(), String> {
    let Some(update) = app
        .updater()
        .map_err(update_error)?
        .check()
        .await
        .map_err(update_error)?
    else {
        return Err(ui_text(
            "目前沒有可安裝的更新",
            "No update is currently available to install",
        )
        .to_owned());
    };

    let progress_events = on_event.clone();
    let finished_events = on_event;
    let mut downloaded = 0_u64;
    let mut started = false;
    update
        .download_and_install(
            move |chunk_length, content_length| {
                if !started {
                    let _ = progress_events.send(UpdateDownloadEvent::Started { content_length });
                    started = true;
                }
                downloaded = downloaded.saturating_add(chunk_length as u64);
                let _ = progress_events.send(UpdateDownloadEvent::Progress {
                    downloaded,
                    content_length,
                });
            },
            move || {
                let _ = finished_events.send(UpdateDownloadEvent::Finished);
            },
        )
        .await
        .map_err(update_install_error)?;

    tracing::info!(version = %update.version, "signed application update installed");
    app.restart()
}

#[tauri::command]
fn get_dashboard_state(state: State<'_, AppRuntime>) -> Result<DashboardState, String> {
    let mut settings = read_settings(&state)?;
    let mut selection_notice = None;
    let (monitors, monitor_status) = match enumerate_monitor_inventory() {
        Ok(inventory) => {
            if let Some(change) = reconcile_monitor_selection(
                &mut settings,
                &inventory.detected,
                &inventory.controllable,
            ) {
                if matches!(
                    &change,
                    MonitorSelectionChange::SelectedOnlyMonitor { .. }
                        | MonitorSelectionChange::ReplacedMissingMonitor { .. }
                ) {
                    settings.peers.iter_mut().for_each(|peer| peer.input = None);
                }
                if let Some(selected) = settings.shared_monitor.as_ref().and_then(|selected| {
                    inventory
                        .controllable
                        .iter()
                        .find(|monitor| selected.fingerprint.matches_exactly(&monitor.fingerprint))
                }) {
                    match platform_controller().and_then(|controller| {
                        refresh_selected_input_data(&controller, selected, &mut settings)
                    }) {
                        Ok(()) => {}
                        Err(error) => tracing::warn!(
                            monitor_id = selected.id.as_str(),
                            error = %error,
                            "unable to record input data for automatically selected display"
                        ),
                    }
                }
                store_settings(&state, settings.clone())?;
                selection_notice = match change {
                    MonitorSelectionChange::SelectedOnlyMonitor { name } => {
                        Some(match UiLocale::current() {
                            UiLocale::TraditionalChinese => format!("已自動選取唯一可控制的 DDC/CI 螢幕：{name}"),
                            UiLocale::English => format!("Automatically selected the only controllable DDC/CI display: {name}"),
                        })
                    }
                    MonitorSelectionChange::ReplacedMissingMonitor {
                        previous,
                        replacement,
                    } => Some(match UiLocale::current() {
                        UiLocale::TraditionalChinese => format!("先前選取的 {previous} 已消失；已安全更新為唯一可控制的 {replacement}"),
                        UiLocale::English => format!("Previously selected {previous} disappeared; safely selected the only controllable display, {replacement}"),
                    }),
                    MonitorSelectionChange::RefreshedMetadata => None,
                };
            }
            let selected = settings.shared_monitor.as_ref();
            let target_found = selected.is_some_and(|selected| {
                inventory
                    .controllable
                    .iter()
                    .any(|monitor| selected.fingerprint.matches_exactly(&monitor.fingerprint))
            });
            let target_detected = selected.is_some_and(|selected| {
                inventory
                    .detected
                    .iter()
                    .any(|monitor| selected.fingerprint.matches_exactly(&monitor.fingerprint))
            });
            let status = match (
                selected,
                target_found,
                target_detected,
                inventory.controllable.is_empty(),
            ) {
                (None, _, _, _) => ui_text(
                    "尚未選擇共用螢幕；目前不會控制任何螢幕",
                    "No shared display is selected; no display will be controlled",
                )
                .to_owned(),
                (Some(selected), true, _, _) => match UiLocale::current() {
                    UiLocale::TraditionalChinese => format!("已鎖定共用螢幕：{}", selected.name),
                    UiLocale::English => format!("Shared display locked: {}", selected.name),
                },
                (Some(selected), false, true, _) => match UiLocale::current() {
                    UiLocale::TraditionalChinese => {
                        format!("已偵測到 {}，但目前無法讀取 DDC/CI 輸入", selected.name)
                    }
                    UiLocale::English => format!(
                        "{} was detected, but its DDC/CI input cannot be read",
                        selected.name
                    ),
                },
                (Some(_), false, false, true) => ui_text(
                    "目前沒有可用的 DDC/CI 顯示器",
                    "No DDC/CI display is currently available",
                )
                .to_owned(),
                (Some(selected), false, false, false) => match UiLocale::current() {
                    UiLocale::TraditionalChinese => {
                        format!("找不到先前選擇的共用螢幕：{}", selected.name)
                    }
                    UiLocale::English => format!(
                        "Previously selected shared display was not found: {}",
                        selected.name
                    ),
                },
            };
            (inventory.controllable, status)
        }
        Err(error) => (Vec::new(), core_user_error(error)),
    };
    let ddc_available = settings.shared_monitor.as_ref().is_some_and(|selected| {
        monitors
            .iter()
            .any(|monitor| selected.fingerprint.matches_exactly(&monitor.fingerprint))
    });
    Ok(DashboardState {
        platform: std::env::consts::OS,
        local_host: settings.local_host,
        agent_configured: has_valid_shared_key(&settings.shared_key),
        ddc_available,
        monitor_status,
        selection_notice,
        monitors,
    })
}

#[tauri::command]
async fn probe_peer(
    peer_id: String,
    state: State<'_, AppRuntime>,
) -> Result<OperationResult, String> {
    let settings = read_settings(&state)?;
    let peer = find_peer(&settings, &peer_id)?;
    request_peer(&settings, peer, AgentAction::Ping).await?;
    Ok(OperationResult {
        title: match UiLocale::current() {
            UiLocale::TraditionalChinese => format!("{} 已連線", peer.name),
            UiLocale::English => format!("{} connected", peer.name),
        },
        detail: ui_text(
            "DisplayMux Agent 已就緒。",
            "The DisplayMux Agent is ready.",
        )
        .to_owned(),
        peer_woken: false,
        warning: false,
    })
}

#[tauri::command]
async fn wake_peer(
    peer_id: String,
    state: State<'_, AppRuntime>,
) -> Result<OperationResult, String> {
    let settings = read_settings(&state)?;
    let peer = find_peer(&settings, &peer_id)?;
    wake_route(&settings, peer).await?;
    Ok(OperationResult {
        title: match UiLocale::current() {
            UiLocale::TraditionalChinese => format!("已送出喚醒訊號給 {}", peer.name),
            UiLocale::English => format!("Wake signal sent to {}", peer.name),
        },
        detail: ui_text(
            "主機是否能喚醒仍取決於電源與網路設定。",
            "Whether the host wakes still depends on its power and network settings.",
        )
        .to_owned(),
        peer_woken: true,
        warning: false,
    })
}

#[tauri::command]
async fn switch_host(
    target_id: String,
    on_event: Channel<SwitchProgress>,
    state: State<'_, AppRuntime>,
) -> Result<OperationResult, String> {
    let settings = read_settings(&state)?;
    let target = if target_id == "local" {
        None
    } else {
        Some(find_peer(&settings, &target_id)?)
    };
    let input = if let Some(peer) = target {
        peer.input.ok_or_else(|| {
            ui_text(
                "尚未設定這台主機使用的螢幕輸入",
                "The display input for this host is not configured",
            )
            .to_owned()
        })?
    } else {
        settings.local_input.ok_or_else(|| {
            ui_text(
                "尚未設定這台主機使用的螢幕輸入",
                "The display input for this host is not configured",
            )
            .to_owned()
        })?
    };
    let preparation = match target {
        Some(peer) => prepare_automatic_switch(&settings, peer, &on_event).await,
        None => NetworkPreparation::NotRequired,
    };
    let _ = on_event.send(SwitchProgress::Switching);
    match run_local_switch(&settings, input) {
        Ok(outcome) => Ok(outcome_result(outcome, &preparation)),
        Err(local_error) => {
            let local_error = core_user_error(local_error);
            let executor = if target_id == "local" {
                (settings.peers.len() == 1).then(|| &settings.peers[0])
            } else {
                settings.peers.iter().find(|peer| peer.id == target_id)
            }
            .ok_or_else(|| match UiLocale::current() {
                UiLocale::TraditionalChinese => {
                    format!("本機無法切換，而且沒有其他已配對主機可代為執行：{local_error}")
                }
                UiLocale::English => format!(
                    "Local switching failed and no other paired host can perform it: {local_error}"
                ),
            })?;
            let _ = on_event.send(SwitchProgress::RemoteFallback {
                peer_name: executor.name.clone(),
            });
            request_peer(&settings, executor, AgentAction::SwitchInput { input })
                .await
                .map_err(|remote_error| match UiLocale::current() {
                    UiLocale::TraditionalChinese => format!(
                        "本機與 {} 都無法切換。本機：{}；遠端：{}",
                        executor.name, local_error, remote_error
                    ),
                    UiLocale::English => format!(
                        "Neither this computer nor {} could switch. Local: {}; remote: {}",
                        executor.name, local_error, remote_error
                    ),
                })?;
            Ok(OperationResult {
                title: match UiLocale::current() {
                    UiLocale::TraditionalChinese => format!("已由 {} 執行切換", executor.name),
                    UiLocale::English => format!("Switch performed by {}", executor.name),
                },
                detail: match UiLocale::current() {
                    UiLocale::TraditionalChinese => {
                        format!("遠端主機已切換至 {}。", localized_input_name(input))
                    }
                    UiLocale::English => {
                        format!(
                            "The remote host switched to {}.",
                            localized_input_name(input)
                        )
                    }
                },
                peer_woken: preparation.peer_woken(),
                warning: false,
            })
        }
    }
}

fn outcome_result(outcome: SwitchOutcome, preparation: &NetworkPreparation) -> OperationResult {
    let mut result = match outcome {
        SwitchOutcome::DryRun { .. } => OperationResult {
            title: ui_text("檢查完成", "Check complete").to_owned(),
            detail: ui_text("未變更螢幕輸入。", "The display input was not changed.").to_owned(),
            peer_woken: preparation.peer_woken(),
            warning: preparation.warning(),
        },
        SwitchOutcome::AlreadySelected { target, input } => OperationResult {
            title: ui_text("已在指定輸入", "Already on the assigned input").to_owned(),
            detail: match UiLocale::current() {
                UiLocale::TraditionalChinese => {
                    format!("{} 已使用 {}。", target.name, localized_input_name(input))
                }
                UiLocale::English => format!(
                    "{} is already using {}.",
                    target.name,
                    localized_input_name(input)
                ),
            },
            peer_woken: preparation.peer_woken(),
            warning: preparation.warning(),
        },
        SwitchOutcome::Switched {
            target,
            previous,
            selected,
        } => OperationResult {
            title: ui_text("共用螢幕已切換", "Shared display switched").to_owned(),
            detail: match UiLocale::current() {
                UiLocale::TraditionalChinese => format!(
                    "{} 已由 {} 切換至 {}。",
                    target.name,
                    localized_input_name(previous),
                    localized_input_name(selected)
                ),
                UiLocale::English => format!(
                    "{} switched from {} to {}.",
                    target.name,
                    localized_input_name(previous),
                    localized_input_name(selected)
                ),
            },
            peer_woken: preparation.peer_woken(),
            warning: preparation.warning(),
        },
    };
    if let NetworkPreparation::Unavailable {
        wake_sent, reason, ..
    } = preparation
    {
        let wake_detail = if *wake_sent {
            ui_text("已先送出喚醒訊號，但", "A wake signal was sent, but ")
        } else {
            ui_text(
                "無法送出喚醒訊號，且",
                "A wake signal could not be sent, and ",
            )
        };
        result.detail.push_str(&match UiLocale::current() {
            UiLocale::TraditionalChinese => format!(" {wake_detail}無法透過區域網路確認目標主機（{reason}）；已自動改用本機 DDC/CI。若目標主機尚未就緒，螢幕可能暫時黑畫面。"),
            UiLocale::English => format!(" {wake_detail}the target host could not be confirmed over the local network ({reason}); local DDC/CI was selected automatically. The display may be temporarily blank if the target host is not ready."),
        });
    }
    result
}

async fn prepare_automatic_switch(
    settings: &AppSettings,
    peer: &HostRoute,
    on_event: &Channel<SwitchProgress>,
) -> NetworkPreparation {
    let _ = on_event.send(SwitchProgress::Waking {
        peer_name: peer.name.clone(),
    });
    let wake_result = wake_route(settings, peer).await;
    let wake_sent = wake_result.is_ok();

    let _ = on_event.send(SwitchProgress::Checking {
        peer_name: peer.name.clone(),
    });
    if !has_valid_shared_key(&settings.shared_key) {
        return NetworkPreparation::Unavailable {
            wake_sent,
            reason: match UiLocale::current() {
                UiLocale::TraditionalChinese => format!("網路 Agent 尚未設定至少 {MIN_SHARED_KEY_LENGTH} 個字元的配對密碼"),
                UiLocale::English => format!("The network Agent does not have a pairing password of at least {MIN_SHARED_KEY_LENGTH} characters"),
            },
        };
    }
    if request_peer(settings, peer, AgentAction::Ping)
        .await
        .is_ok()
    {
        return NetworkPreparation::Ready { wake_sent };
    }

    if let Err(wake_error) = wake_result {
        return NetworkPreparation::Unavailable {
            wake_sent: false,
            reason: match UiLocale::current() {
                UiLocale::TraditionalChinese => format!("{}，且 Agent 目前沒有回應", wake_error),
                UiLocale::English => format!("{wake_error}, and the Agent is not responding"),
            },
        };
    }

    let _ = on_event.send(SwitchProgress::Waiting {
        peer_name: peer.name.clone(),
        seconds: settings.wait_seconds.clamp(5, 120),
    });
    match wait_until_peer_ready(settings, peer).await {
        Ok(()) => NetworkPreparation::Ready { wake_sent: true },
        Err(reason) => NetworkPreparation::Unavailable {
            wake_sent: true,
            reason,
        },
    }
}

async fn wait_until_peer_ready(settings: &AppSettings, peer: &HostRoute) -> Result<(), String> {
    let attempts = settings.wait_seconds.clamp(5, 120);
    for _ in 0..attempts {
        sleep(Duration::from_secs(1)).await;
        if request_peer(settings, peer, AgentAction::Ping)
            .await
            .is_ok()
        {
            return Ok(());
        }
    }
    Err(match UiLocale::current() {
        UiLocale::TraditionalChinese => {
            format!("{} 在送出喚醒訊號後 {} 秒內仍沒有回應", peer.name, attempts)
        }
        UiLocale::English => format!(
            "{} did not respond within {} seconds after the wake signal",
            peer.name, attempts
        ),
    })
}

async fn request_peer(
    settings: &AppSettings,
    peer: &HostRoute,
    action: AgentAction,
) -> Result<AgentResponse, String> {
    let endpoint = route_endpoint(peer).map_err(core_user_error)?;
    if !has_valid_shared_key(&settings.shared_key) {
        return Err(match UiLocale::current() {
            UiLocale::TraditionalChinese => {
                format!("請先設定至少 {MIN_SHARED_KEY_LENGTH} 個字元的配對密碼")
            }
            UiLocale::English => format!(
                "Configure a pairing password of at least {MIN_SHARED_KEY_LENGTH} characters first"
            ),
        });
    }
    let response = AgentClient::new(endpoint, Arc::<[u8]>::from(settings.shared_key.as_bytes()))
        .request(action, next_nonce())
        .await
        .map_err(core_user_error)?;
    if response.ready {
        Ok(response)
    } else {
        Err(response.message)
    }
}

async fn wake_route(settings: &AppSettings, peer: &HostRoute) -> Result<(), String> {
    if peer.mac_address.trim().is_empty() {
        return Err(match UiLocale::current() {
            UiLocale::TraditionalChinese => format!("{} 沒有提供可用的 MAC 位址，因此無法使用 Wake-on-LAN；主機醒著時仍可切換", peer.name),
            UiLocale::English => format!("{} has no usable MAC address, so Wake-on-LAN is unavailable; switching still works while the host is awake", peer.name),
        });
    }
    let mac_address = MacAddress::from_str(&peer.mac_address).map_err(core_user_error)?;
    let broadcast_address = Ipv4Addr::from_str(&settings.broadcast_ip)
        .map_err(|_| ui_text("廣播位址格式無效", "Invalid broadcast address").to_owned())?;
    WakeTarget {
        mac_address,
        broadcast_address,
        port: settings.wake_port,
    }
    .wake()
    .await
    .map_err(core_user_error)
}

fn route_endpoint(peer: &HostRoute) -> Result<PeerEndpoint, DisplayMuxError> {
    let address = IpAddr::from_str(&peer.address).map_err(|_| {
        DisplayMuxError::PeerUnavailable(match UiLocale::current() {
            UiLocale::TraditionalChinese => format!("{} 的 IP 位址無效", peer.name),
            UiLocale::English => format!("{} has an invalid IP address", peer.name),
        })
    })?;
    Ok(PeerEndpoint {
        address,
        port: peer.port,
    })
}

fn find_peer<'a>(settings: &'a AppSettings, peer_id: &str) -> Result<&'a HostRoute, String> {
    settings
        .peers
        .iter()
        .find(|peer| peer.id == peer_id)
        .ok_or_else(|| {
            ui_text(
                "找不到這台已配對主機，請重新搜尋並加入",
                "This paired host was not found. Search for it and add it again.",
            )
            .to_owned()
        })
}

fn validate_settings(settings: &AppSettings) -> Result<(), DisplayMuxError> {
    if settings.local_host != local_host() {
        return Err(DisplayMuxError::Backend(
            ui_text(
                "這台電腦的主機類型必須由作業系統自動判定",
                "This computer's host type must be determined by the operating system",
            )
            .to_owned(),
        ));
    }
    if settings
        .local_input
        .is_some_and(|input| DisplayInput::new(input.value()).is_err())
        || settings.peers.iter().any(|peer| {
            peer.input
                .is_some_and(|input| DisplayInput::new(input.value()).is_err())
        })
    {
        return Err(DisplayMuxError::Backend(
            ui_text(
                "請選擇有效的螢幕輸入 Port",
                "Select a valid display input port",
            )
            .to_owned(),
        ));
    }
    let assigned_inputs = settings
        .local_input
        .into_iter()
        .chain(settings.peers.iter().filter_map(|peer| peer.input))
        .collect::<Vec<_>>();
    let unique_inputs = assigned_inputs
        .iter()
        .map(|input| input.value())
        .collect::<std::collections::HashSet<_>>();
    if unique_inputs.len() != assigned_inputs.len() {
        return Err(DisplayMuxError::Backend(
            ui_text(
                "每個主機必須使用不同的螢幕輸入 Port",
                "Each host must use a different display input port",
            )
            .to_owned(),
        ));
    }
    if let Some(supported) = &settings.supported_inputs {
        if settings
            .peers
            .iter()
            .filter_map(|peer| peer.input)
            .any(|assigned| !supported.contains(&assigned))
        {
            return Err(DisplayMuxError::Backend(
                ui_text(
                    "輸入值不在這台螢幕的 MCCS capabilities 清單中",
                    "The input is not listed in this display's MCCS capabilities",
                )
                .to_owned(),
            ));
        }
    }
    for peer in &settings.peers {
        route_endpoint(peer)?;
        if !peer.mac_address.trim().is_empty() {
            MacAddress::from_str(&peer.mac_address)?;
        }
    }
    Ipv4Addr::from_str(&settings.broadcast_ip).map_err(|_| {
        DisplayMuxError::WakeFailed(
            ui_text("廣播位址格式無效", "Invalid broadcast address").to_owned(),
        )
    })?;
    if !settings.shared_key.is_empty() && !has_valid_shared_key(&settings.shared_key) {
        return Err(DisplayMuxError::Backend(match UiLocale::current() {
            UiLocale::TraditionalChinese => {
                format!("配對密碼至少需要 {MIN_SHARED_KEY_LENGTH} 個字元")
            }
            UiLocale::English => format!(
                "The pairing password must contain at least {MIN_SHARED_KEY_LENGTH} characters"
            ),
        }));
    }
    Ok(())
}

fn upsert_discovered_peer(settings: &mut AppSettings, peer: &DiscoveredPeer) {
    if let Some(existing) = settings.peers.iter_mut().find(|item| item.id == peer.id) {
        existing.name.clone_from(&peer.name);
        existing.platform = peer.platform;
        existing.address = peer.address.to_string();
        existing.port = peer.port;
        existing.mac_address = peer.mac_address.clone().unwrap_or_default();
        return;
    }
    settings.peers.push(HostRoute {
        id: peer.id.clone(),
        name: peer.name.clone(),
        platform: peer.platform,
        address: peer.address.to_string(),
        port: peer.port,
        mac_address: peer.mac_address.clone().unwrap_or_default(),
        input: None,
    });
}

fn refresh_paired_endpoints(state: &AppRuntime, peers: &[DiscoveredPeer]) -> Result<(), String> {
    let current = read_settings_inner(state)?;
    let mut updated = current.clone();
    for peer in peers {
        if updated.peers.iter().any(|item| item.id == peer.id) {
            upsert_discovered_peer(&mut updated, peer);
        }
    }
    if updated != current {
        store_settings(state, updated)?;
    }
    Ok(())
}

async fn restart_agent(state: &AppRuntime) -> Result<(), String> {
    let settings = read_settings_inner(state)?;
    let mut current_task = state.agent_task.lock().await;
    if let Some(task) = current_task.take() {
        task.abort();
    }
    if !has_valid_shared_key(&settings.shared_key) {
        return Ok(());
    }
    let server = AgentServer::new(
        SocketAddr::from(([0, 0, 0, 0], DEFAULT_AGENT_PORT)),
        Arc::<[u8]>::from(settings.shared_key.as_bytes()),
    );
    let live_settings = Arc::clone(&state.settings);
    *current_task = Some(tauri::async_runtime::spawn(async move {
        let result = server
            .run(move |action| {
                let live_settings = Arc::clone(&live_settings);
                async move {
                    match action {
                        AgentAction::Ping => AgentResponse {
                            ready: true,
                            message: ui_text(
                                "DisplayMux Agent 已就緒",
                                "DisplayMux Agent is ready",
                            )
                            .to_owned(),
                        },
                        AgentAction::SwitchInput { input } => {
                            let fingerprint = live_settings.read().ok().and_then(|settings| {
                                settings
                                    .shared_monitor
                                    .as_ref()
                                    .map(|monitor| monitor.fingerprint.clone())
                            });
                            let Some(fingerprint) = fingerprint else {
                                return AgentResponse {
                                    ready: false,
                                    message: ui_text(
                                        "這台主機尚未選擇共用螢幕",
                                        "No shared display is selected on this host",
                                    )
                                    .to_owned(),
                                };
                            };
                            match tauri::async_runtime::spawn_blocking(move || {
                                run_switch(fingerprint, input)
                            })
                            .await
                            {
                                Ok(Ok(_)) => AgentResponse {
                                    ready: true,
                                    message: match UiLocale::current() {
                                        UiLocale::TraditionalChinese => format!(
                                            "遠端主機已切換至 {}",
                                            localized_input_name(input)
                                        ),
                                        UiLocale::English => format!(
                                            "The remote host switched to {}",
                                            localized_input_name(input)
                                        ),
                                    },
                                },
                                Ok(Err(error)) => AgentResponse {
                                    ready: false,
                                    message: core_user_error(error),
                                },
                                Err(error) => AgentResponse {
                                    ready: false,
                                    message: match UiLocale::current() {
                                        UiLocale::TraditionalChinese => {
                                            format!("切換工作無法執行：{error}")
                                        }
                                        UiLocale::English => {
                                            format!("The switching task could not run: {error}")
                                        }
                                    },
                                },
                            }
                        }
                    }
                }
            })
            .await;
        if let Err(error) = result {
            tracing::error!(error = %error, "DisplayMux agent stopped");
        }
    }));
    Ok(())
}

fn store_settings(state: &AppRuntime, settings: AppSettings) -> Result<AppSettings, String> {
    persist_settings(&state.settings_path, &settings).map_err(core_user_error)?;
    let mut current = state.settings.write().map_err(|_| {
        ui_text(
            "無法更新設定，請重新啟動 DisplayMux",
            "Unable to update settings. Restart DisplayMux.",
        )
        .to_owned()
    })?;
    *current = settings.clone();
    Ok(settings)
}

fn read_settings(state: &AppRuntime) -> Result<AppSettings, String> {
    read_settings_inner(state)
}

fn read_settings_inner(state: &AppRuntime) -> Result<AppSettings, String> {
    state
        .settings
        .read()
        .map(|settings| settings.clone())
        .map_err(|_| {
            ui_text(
                "無法讀取設定，請重新啟動 DisplayMux",
                "Unable to read settings. Restart DisplayMux.",
            )
            .to_owned()
        })
}

fn load_settings(path: &Path) -> AppSettings {
    let Ok(contents) = fs::read_to_string(path) else {
        return AppSettings::default();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) else {
        return AppSettings::default();
    };
    if value.get("sharedMonitor").is_some() || value.get("peers").is_some() {
        serde_json::from_value(value).unwrap_or_default()
    } else {
        serde_json::from_value::<LegacySettings>(value)
            .map(migrate_legacy_settings)
            .unwrap_or_default()
    }
}

fn migrate_legacy_settings(legacy: LegacySettings) -> AppSettings {
    let local_input = DisplayInput::new(match legacy.local_host {
        DestinationHost::Windows => 0x0f,
        DestinationHost::Mac => 0x11,
    })
    .ok();
    let peer_input = DisplayInput::new(match legacy.local_host {
        DestinationHost::Windows => 0x11,
        DestinationHost::Mac => 0x0f,
    })
    .ok();
    let peers = if legacy.peer_id.is_empty() || legacy.peer_ip.is_empty() {
        Vec::new()
    } else {
        vec![HostRoute {
            id: legacy.peer_id,
            name: legacy.peer_name,
            platform: match legacy.local_host {
                DestinationHost::Windows => DestinationHost::Mac,
                DestinationHost::Mac => DestinationHost::Windows,
            },
            address: legacy.peer_ip,
            port: legacy.peer_port,
            mac_address: legacy.peer_mac,
            input: peer_input,
        }]
    };
    AppSettings {
        local_host: legacy.local_host,
        // 舊版沒有保存使用者選擇；升級後要求重新選取，避免沿用硬體假設。
        shared_monitor: None,
        local_input,
        supported_inputs: None,
        peers,
        broadcast_ip: legacy.broadcast_ip,
        wake_port: legacy.wake_port,
        shared_key: legacy.shared_key,
        wait_seconds: legacy.wait_seconds,
        autostart: legacy.autostart,
        check_updates: legacy.check_updates,
    }
}

fn persist_settings(path: &Path, settings: &AppSettings) -> Result<(), DisplayMuxError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| DisplayMuxError::Backend(error.to_string()))?;
    }
    let serialized = serde_json::to_vec_pretty(settings)
        .map_err(|error| DisplayMuxError::Backend(error.to_string()))?;
    fs::write(path, serialized).map_err(|error| DisplayMuxError::Backend(error.to_string()))
}

fn next_nonce() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let counter = NONCE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{}-{now}-{counter}", std::process::id())
}

fn run_local_switch(
    settings: &AppSettings,
    input: DisplayInput,
) -> Result<SwitchOutcome, DisplayMuxError> {
    let selected = settings
        .shared_monitor
        .as_ref()
        .ok_or(DisplayMuxError::TargetNotFound)?;
    run_switch(selected.fingerprint.clone(), input)
}

fn run_switch(
    fingerprint: MonitorFingerprint,
    input: DisplayInput,
) -> Result<SwitchOutcome, DisplayMuxError> {
    let service = DisplayMuxService::new(
        platform_controller()?,
        DisplayMuxProfile {
            shared_monitor: fingerprint,
        },
    );
    service.switch_to_input(input, SwitchMode::Apply)
}

fn enumerate_monitor_inventory() -> Result<MonitorInventory, DisplayMuxError> {
    let controller = platform_controller()?;
    monitor_inventory(&controller)
}

fn monitor_inventory<C: MonitorControl>(
    controller: &C,
) -> Result<MonitorInventory, DisplayMuxError> {
    let detected = controller.enumerate()?;
    let controllable = detected
        .iter()
        .filter(|monitor| match controller.read_input(&monitor.id) {
            Ok(_) => true,
            Err(error) => {
                tracing::debug!(
                    monitor_id = monitor.id.as_str(),
                    error = %error,
                    "display does not expose a controllable DDC/CI input"
                );
                false
            }
        })
        .cloned()
        .collect();
    Ok(MonitorInventory {
        detected,
        controllable,
    })
}

fn common_input_sources() -> Vec<DisplayInput> {
    (1..=0x12)
        .chain(std::iter::once(0x1b))
        .filter_map(|value| DisplayInput::new(value).ok())
        .filter(|input| input.standard_name().is_some() || input.value() == 0x1b)
        .collect()
}

fn refresh_selected_input_data<C: MonitorControl>(
    controller: &C,
    monitor: &MonitorDescriptor,
    settings: &mut AppSettings,
) -> Result<(), DisplayMuxError> {
    // Reading VCP 0x60 is non-disruptive. Never write or cycle ports for discovery.
    settings.local_input = None;
    settings.supported_inputs = None;
    settings.local_input = Some(controller.read_input(&monitor.id)?);
    settings.supported_inputs = match controller.supported_inputs(&monitor.id) {
        Ok(inputs) if !inputs.is_empty() => Some(inputs),
        Ok(_) => None,
        Err(error) => {
            tracing::warn!(
                monitor_id = monitor.id.as_str(),
                error = %error,
                "monitor capabilities unavailable; using common MCCS input list"
            );
            None
        }
    };
    Ok(())
}

fn reconcile_monitor_selection(
    settings: &mut AppSettings,
    detected: &[MonitorDescriptor],
    controllable: &[MonitorDescriptor],
) -> Option<MonitorSelectionChange> {
    if let Some(selected) = settings.shared_monitor.as_ref() {
        if let Some(current) = controllable
            .iter()
            .find(|monitor| selected.fingerprint.matches_exactly(&monitor.fingerprint))
        {
            let refreshed = SelectedMonitor::from(current);
            if *selected != refreshed {
                settings.shared_monitor = Some(refreshed);
                return Some(MonitorSelectionChange::RefreshedMetadata);
            }
            return None;
        }
        if detected
            .iter()
            .any(|monitor| selected.fingerprint.matches_exactly(&monitor.fingerprint))
        {
            return None;
        }
    }

    let mut auto_candidates = controllable.iter().filter(|monitor| !monitor.built_in);
    let only = auto_candidates.next()?;
    if auto_candidates.next().is_some() {
        return None;
    }
    let replacement = SelectedMonitor::from(only);
    let change = match settings.shared_monitor.replace(replacement) {
        Some(previous) => MonitorSelectionChange::ReplacedMissingMonitor {
            previous: previous.name,
            replacement: only.name.clone(),
        },
        None => MonitorSelectionChange::SelectedOnlyMonitor {
            name: only.name.clone(),
        },
    };
    Some(change)
}

#[cfg(target_os = "windows")]
fn platform_controller() -> Result<impl MonitorControl, DisplayMuxError> {
    displaymux_core::windows::WindowsMonitorController::new()
}

#[cfg(target_os = "macos")]
fn platform_controller() -> Result<impl MonitorControl, DisplayMuxError> {
    Ok(displaymux_core::macos::MacOsMonitorController::new())
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn platform_controller() -> Result<UnsupportedController, DisplayMuxError> {
    Err(DisplayMuxError::UnsupportedPlatform)
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
struct UnsupportedController;

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
impl MonitorControl for UnsupportedController {
    fn enumerate(&self) -> Result<Vec<MonitorDescriptor>, DisplayMuxError> {
        Err(DisplayMuxError::UnsupportedPlatform)
    }
    fn read_input(
        &self,
        _monitor: &displaymux_core::MonitorId,
    ) -> Result<DisplayInput, DisplayMuxError> {
        Err(DisplayMuxError::UnsupportedPlatform)
    }
    fn supported_inputs(
        &self,
        _monitor: &displaymux_core::MonitorId,
    ) -> Result<Vec<DisplayInput>, DisplayMuxError> {
        Err(DisplayMuxError::UnsupportedPlatform)
    }
    fn write_input(
        &self,
        _monitor: &displaymux_core::MonitorId,
        _input: DisplayInput,
    ) -> Result<(), DisplayMuxError> {
        Err(DisplayMuxError::UnsupportedPlatform)
    }
}

#[cfg(target_os = "windows")]
const fn local_host() -> DestinationHost {
    DestinationHost::Windows
}
#[cfg(target_os = "macos")]
const fn local_host() -> DestinationHost {
    DestinationHost::Mac
}
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
const fn local_host() -> DestinationHost {
    DestinationHost::Windows
}

fn user_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

fn core_user_error(error: DisplayMuxError) -> String {
    error.localized_message(UiLocale::current() == UiLocale::TraditionalChinese)
}

fn update_error(error: impl std::fmt::Display) -> String {
    tracing::warn!(error = %error, "application update check failed");
    ui_text(
        "無法檢查更新；請確認網路可連線至 GitHub Releases，稍後再試一次",
        "Unable to check for updates. Confirm that GitHub Releases is reachable and try again later.",
    ).to_owned()
}

fn update_install_error(error: impl std::fmt::Display) -> String {
    tracing::error!(error = %error, "signed application update installation failed");
    ui_text(
        "更新下載或簽章驗證失敗；目前版本未變更，請稍後再試一次",
        "The update download or signature verification failed. The current version was not changed; try again later.",
    ).to_owned()
}
fn has_valid_shared_key(shared_key: &str) -> bool {
    shared_key.chars().count() >= MIN_SHARED_KEY_LENGTH
}

fn settings_for_current_build(settings: AppSettings) -> AppSettings {
    // A development executable may point at a dev server and, on Windows, may
    // be a console process. Never persist it as a login item.
    #[cfg(debug_assertions)]
    let settings = AppSettings {
        autostart: false,
        ..settings
    };
    settings
}

fn autostart_args() -> Option<Vec<&'static str>> {
    #[cfg(target_os = "windows")]
    {
        Some(vec!["--autostart"])
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

fn launched_from_autostart(args: impl IntoIterator<Item = String>) -> bool {
    args.into_iter().any(|arg| arg == "--autostart")
}

#[cfg(target_os = "windows")]
fn hide_windows_main_window(window: &tauri::Window) {
    if let Err(error) = window.set_skip_taskbar(true) {
        tracing::warn!(error = %error, "unable to remove DisplayMux from the taskbar");
    }
    if let Err(error) = window.hide() {
        tracing::warn!(error = %error, "unable to hide DisplayMux in the system tray");
    }
}

#[cfg(target_os = "windows")]
fn hide_windows_main_webview(window: &tauri::WebviewWindow) {
    if let Err(error) = window.set_skip_taskbar(true) {
        tracing::warn!(error = %error, "unable to remove DisplayMux from the taskbar");
    }
    if let Err(error) = window.hide() {
        tracing::warn!(error = %error, "unable to hide DisplayMux in the system tray");
    }
}

#[cfg(target_os = "windows")]
fn show_windows_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if let Err(error) = window.set_skip_taskbar(false) {
        tracing::warn!(error = %error, "unable to restore DisplayMux to the taskbar");
    }
    if let Err(error) = window.show() {
        tracing::warn!(error = %error, "unable to show DisplayMux from the system tray");
    }
    if let Err(error) = window.unminimize() {
        tracing::warn!(error = %error, "unable to unminimize DisplayMux");
    }
    if let Err(error) = window.set_focus() {
        tracing::warn!(error = %error, "unable to focus DisplayMux");
    }
}

#[cfg(target_os = "windows")]
fn setup_windows_tray(app: &tauri::App) -> tauri::Result<()> {
    use tauri::{
        menu::MenuBuilder,
        tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    };

    let menu = MenuBuilder::new(app)
        .text("tray-open", ui_text("開啟 DisplayMux", "Open DisplayMux"))
        .separator()
        .text("tray-quit", ui_text("結束 DisplayMux", "Quit DisplayMux"))
        .build()?;
    let mut tray = TrayIconBuilder::with_id("displaymux")
        .menu(&menu)
        .tooltip("DisplayMux")
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "tray-open" => show_windows_main_window(app),
            "tray-quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } | TrayIconEvent::DoubleClick {
                    button: MouseButton::Left,
                    ..
                }
            ) {
                show_windows_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }
    tray.build(app)?;
    Ok(())
}

pub fn run() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_target(false)
        .compact()
        .try_init();
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            autostart_args(),
        ))
        .plugin(tauri_plugin_updater::Builder::new().build());
    #[cfg(target_os = "windows")]
    let builder = builder.on_window_event(|window, event| {
        if window.label() != "main" {
            return;
        }
        match event {
            tauri::WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                hide_windows_main_window(window);
            }
            tauri::WindowEvent::Resized(_) if window.is_minimized().unwrap_or(false) => {
                hide_windows_main_window(window);
            }
            _ => {}
        }
    });
    builder
        .setup(|app| {
            let config_dir = app
                .path()
                .app_config_dir()
                .map_err(|error| anyhow::anyhow!(error))?;
            let settings_path = config_dir.join("settings.json");
            let settings = settings_for_current_build(load_settings(&settings_path));
            #[cfg(debug_assertions)]
            if let Err(error) = app.autolaunch().disable() {
                tracing::warn!(error = %error, "unable to remove development autostart entry");
            }
            #[cfg(all(target_os = "windows", not(debug_assertions)))]
            if settings.autostart {
                if let Err(error) = app.autolaunch().enable() {
                    tracing::warn!(error = %error, "unable to refresh the login autostart entry");
                }
            }
            let discovery = MdnsPeerDiscovery::start(local_host(), DEFAULT_AGENT_PORT)
                .map(Some)
                .unwrap_or_else(|error| {
                    tracing::warn!(error = %error, "unable to start DisplayMux mDNS discovery");
                    None
                });
            app.manage(AppRuntime {
                settings: Arc::new(RwLock::new(settings)),
                settings_path,
                agent_task: Mutex::new(None),
                discovery,
            });
            #[cfg(target_os = "windows")]
            {
                setup_windows_tray(app)?;
                if launched_from_autostart(std::env::args()) {
                    if let Some(window) = app.get_webview_window("main") {
                        hide_windows_main_webview(&window);
                    }
                }
            }
            let handle: AppHandle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                if let Some(runtime) = handle.try_state::<AppRuntime>() {
                    if let Err(error) = restart_agent(&runtime).await {
                        tracing::warn!(error = %error, "unable to start DisplayMux agent");
                    }
                }
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            set_locale,
            discover_peers,
            select_peer,
            remove_peer,
            select_monitor,
            get_settings,
            get_input_options,
            save_settings,
            check_for_update,
            install_update,
            get_dashboard_state,
            probe_peer,
            wake_peer,
            switch_host
        ])
        .run(tauri::generate_context!())
        .map_err(anyhow::Error::from)
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    struct SelectionController {
        monitors: Vec<MonitorDescriptor>,
        controllable: HashSet<String>,
    }

    impl MonitorControl for SelectionController {
        fn enumerate(&self) -> Result<Vec<MonitorDescriptor>, DisplayMuxError> {
            Ok(self.monitors.clone())
        }

        fn read_input(
            &self,
            monitor: &displaymux_core::MonitorId,
        ) -> Result<DisplayInput, DisplayMuxError> {
            if self.controllable.contains(monitor.as_str()) {
                DisplayInput::new(0x0f)
            } else {
                Err(DisplayMuxError::Backend("DDC/CI unavailable".to_owned()))
            }
        }

        fn supported_inputs(
            &self,
            monitor: &displaymux_core::MonitorId,
        ) -> Result<Vec<DisplayInput>, DisplayMuxError> {
            if self.controllable.contains(monitor.as_str()) {
                Ok(vec![
                    DisplayInput::new(0x0f).unwrap(),
                    DisplayInput::new(0x11).unwrap(),
                    DisplayInput::new(0x1b).unwrap(),
                ])
            } else {
                Err(DisplayMuxError::Backend(
                    "capabilities unavailable".to_owned(),
                ))
            }
        }

        fn write_input(
            &self,
            _monitor: &displaymux_core::MonitorId,
            _input: DisplayInput,
        ) -> Result<(), DisplayMuxError> {
            unreachable!("selection tests never write an input")
        }
    }

    fn monitor(id: &str) -> MonitorDescriptor {
        MonitorDescriptor {
            id: displaymux_core::MonitorId::new(id),
            name: id.to_owned(),
            fingerprint: MonitorFingerprint::new("ACM", id, Some(format!("serial-{id}"))),
            active: true,
            built_in: false,
            max_resolution: Some(displaymux_core::MonitorResolution::new(2560, 1440)),
            resolution_source: Some(ResolutionSource::WindowsDisplayMode),
        }
    }

    #[test]
    fn shared_key_requires_at_least_eight_characters() {
        assert!(!has_valid_shared_key("1234567"));
        assert!(has_valid_shared_key("12345678"));
        assert!(has_valid_shared_key("配對密碼八個字元"));
    }

    #[test]
    fn new_install_does_not_assume_a_monitor_or_input() {
        let settings = AppSettings::default();
        assert!(settings.shared_monitor.is_none());
        assert!(settings.local_input.is_none());
        assert!(settings.peers.is_empty());
        assert!(settings.check_updates);
    }

    #[test]
    fn development_builds_do_not_register_autostart() {
        let settings = settings_for_current_build(AppSettings::default());
        assert_eq!(settings.autostart, !cfg!(debug_assertions));
    }

    #[test]
    fn only_the_explicit_login_argument_starts_windows_hidden() {
        assert!(launched_from_autostart([
            "DisplayMux.exe".to_owned(),
            "--autostart".to_owned(),
        ]));
        assert!(!launched_from_autostart(["DisplayMux.exe".to_owned()]));
    }

    #[test]
    fn automatic_switch_tracks_wake_and_network_fallback_state() {
        assert!(!NetworkPreparation::NotRequired.peer_woken());
        assert!(!NetworkPreparation::Ready { wake_sent: true }.warning());
        assert!(NetworkPreparation::Ready { wake_sent: true }.peer_woken());
        assert!(NetworkPreparation::Unavailable {
            wake_sent: false,
            reason: "offline".to_owned(),
        }
        .warning());
    }

    #[test]
    fn v011_selected_monitor_without_resolution_source_still_loads() {
        let value = serde_json::json!({
            "name": "Existing monitor",
            "fingerprint": {
                "manufacturer_id": "ACM",
                "product_code": "1234",
                "serial_number": "serial"
            },
            "maxResolution": { "width": 3440, "height": 1440 }
        });
        let selected: SelectedMonitor = serde_json::from_value(value).unwrap();
        assert_eq!(selected.resolution_source, None);
        assert_eq!(selected.max_resolution.unwrap().width, 3440);
    }

    #[test]
    fn migration_preserves_the_previous_two_host_configuration() {
        let cases = [
            (DestinationHost::Windows, DestinationHost::Mac, 0x0f, 0x11),
            (DestinationHost::Mac, DestinationHost::Windows, 0x11, 0x0f),
        ];

        for (local_host, peer_platform, local_input, peer_input) in cases {
            let legacy = LegacySettings {
                local_host,
                peer_id: "peer".to_owned(),
                peer_name: "Peer computer".to_owned(),
                peer_ip: "192.168.1.20".to_owned(),
                ..LegacySettings::default()
            };
            let migrated = migrate_legacy_settings(legacy);
            assert!(migrated.shared_monitor.is_none());
            assert_eq!(migrated.local_input.unwrap().value(), local_input);
            assert_eq!(migrated.peers[0].platform, peer_platform);
            assert_eq!(migrated.peers[0].input.unwrap().value(), peer_input);
        }
    }

    #[test]
    fn selects_and_persists_the_only_controllable_monitor() {
        let external = monitor("external");
        let mut internal = monitor("internal");
        internal.built_in = true;
        let uncontrollable = monitor("uncontrollable");
        let controller = SelectionController {
            monitors: vec![internal, uncontrollable, external.clone()],
            controllable: HashSet::from(["internal".to_owned(), external.id.as_str().to_owned()]),
        };
        let inventory = monitor_inventory(&controller).unwrap();
        assert_eq!(inventory.detected.len(), 3);
        assert_eq!(inventory.controllable.len(), 2);
        let mut settings = AppSettings::default();

        let change = reconcile_monitor_selection(
            &mut settings,
            &inventory.detected,
            &inventory.controllable,
        );

        assert_eq!(
            change,
            Some(MonitorSelectionChange::SelectedOnlyMonitor {
                name: "external".to_owned()
            })
        );
        assert_eq!(
            settings.shared_monitor,
            Some(SelectedMonitor::from(&external))
        );
    }

    #[test]
    fn selected_monitor_records_current_input_and_capability_values_without_writes() {
        let external = monitor("external");
        let controller = SelectionController {
            monitors: vec![external.clone()],
            controllable: HashSet::from([external.id.as_str().to_owned()]),
        };
        let mut settings = AppSettings::default();

        refresh_selected_input_data(&controller, &external, &mut settings).unwrap();

        assert_eq!(settings.local_input.unwrap().value(), 0x0f);
        assert_eq!(
            settings
                .supported_inputs
                .unwrap()
                .iter()
                .map(|input| input.value())
                .collect::<Vec<_>>(),
            vec![0x0f, 0x11, 0x1b]
        );
    }

    #[test]
    fn common_input_names_include_vga_dvi_dp_hdmi_and_type_c_without_codes() {
        let inputs = common_input_sources();
        for value in [0x01, 0x03, 0x0f, 0x11, 0x1b] {
            assert!(inputs.iter().any(|input| input.value() == value));
        }
        assert_eq!(
            localized_input_name(DisplayInput::new(0x0f).unwrap()),
            "DP 1"
        );
        assert_eq!(
            localized_input_name(DisplayInput::new(0x1b).unwrap()),
            "Type-C"
        );
        assert!(!localized_input_name(DisplayInput::new(0x11).unwrap()).contains("0x"));
    }

    #[test]
    fn settings_reject_duplicate_and_unadvertised_input_assignments() {
        let mut settings = AppSettings {
            local_input: DisplayInput::new(0x0f).ok(),
            supported_inputs: Some(vec![
                DisplayInput::new(0x0f).unwrap(),
                DisplayInput::new(0x11).unwrap(),
            ]),
            ..AppSettings::default()
        };
        settings.peers.push(HostRoute {
            id: "peer".to_owned(),
            name: "Peer".to_owned(),
            platform: DestinationHost::Mac,
            address: "192.168.1.20".to_owned(),
            port: DEFAULT_AGENT_PORT,
            mac_address: String::new(),
            input: DisplayInput::new(0x0f).ok(),
        });
        assert!(validate_settings(&settings).is_err());

        settings.peers[0].input = DisplayInput::new(0x1b).ok();
        assert!(validate_settings(&settings).is_err());
    }

    #[test]
    fn safely_replaces_a_missing_selection_when_only_one_candidate_remains() {
        let previous = monitor("disconnected");
        let replacement = monitor("replacement");
        let mut settings = AppSettings {
            shared_monitor: Some(SelectedMonitor::from(&previous)),
            ..AppSettings::default()
        };

        let change = reconcile_monitor_selection(
            &mut settings,
            std::slice::from_ref(&replacement),
            std::slice::from_ref(&replacement),
        );

        assert_eq!(
            change,
            Some(MonitorSelectionChange::ReplacedMissingMonitor {
                previous: "disconnected".to_owned(),
                replacement: "replacement".to_owned(),
            })
        );
        assert_eq!(
            settings.shared_monitor,
            Some(SelectedMonitor::from(&replacement))
        );
    }

    #[test]
    fn never_guesses_between_multiple_controllable_monitors() {
        let mut settings = AppSettings::default();
        let monitors = [monitor("first"), monitor("second")];

        assert_eq!(
            reconcile_monitor_selection(&mut settings, &monitors, &monitors),
            None
        );
        assert!(settings.shared_monitor.is_none());
    }

    #[test]
    fn missing_selection_is_not_replaced_when_multiple_candidates_remain() {
        let disconnected = monitor("disconnected");
        let mut settings = AppSettings {
            shared_monitor: Some(SelectedMonitor::from(&disconnected)),
            ..AppSettings::default()
        };
        let monitors = [monitor("first"), monitor("second")];

        assert_eq!(
            reconcile_monitor_selection(&mut settings, &monitors, &monitors),
            None
        );
        assert_eq!(
            settings.shared_monitor,
            Some(SelectedMonitor::from(&disconnected))
        );
    }

    #[test]
    fn automatic_offline_fallback_explains_the_black_screen_risk() {
        let target = monitor("external");
        let input = DisplayInput::new(0x11).unwrap();
        let preparation = NetworkPreparation::Unavailable {
            wake_sent: true,
            reason: "Agent 沒有回應".to_owned(),
        };
        let result = outcome_result(
            SwitchOutcome::AlreadySelected { target, input },
            &preparation,
        );

        assert!(result.warning);
        assert!(result.peer_woken);
        assert!(result
            .detail
            .contains("local DDC/CI was selected automatically"));
        assert!(result.detail.contains("temporarily blank"));
    }

    #[test]
    fn locale_detection_uses_traditional_chinese_and_falls_back_to_english() {
        assert_eq!(locale_from_tag("zh-TW"), UiLocale::TraditionalChinese);
        assert_eq!(locale_from_tag("zh-Hant-HK"), UiLocale::TraditionalChinese);
        assert_eq!(locale_from_tag("en-US"), UiLocale::English);
        assert_eq!(locale_from_tag("ja-JP"), UiLocale::English);
        assert_eq!(locale_from_tag("zh-CN"), UiLocale::English);
    }

    #[test]
    fn transient_ddc_failure_does_not_replace_a_still_detected_selection() {
        let selected = monitor("selected");
        let replacement = monitor("replacement");
        let mut settings = AppSettings {
            shared_monitor: Some(SelectedMonitor::from(&selected)),
            ..AppSettings::default()
        };
        let detected = [selected.clone(), replacement.clone()];

        assert_eq!(
            reconcile_monitor_selection(
                &mut settings,
                &detected,
                std::slice::from_ref(&replacement),
            ),
            None
        );
        assert_eq!(
            settings.shared_monitor,
            Some(SelectedMonitor::from(&selected))
        );
    }
}
