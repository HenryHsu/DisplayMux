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
const MIN_SHARED_KEY_LENGTH: usize = 8;

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
    code: String,
    name: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationResult {
    title: String,
    detail: String,
    peer_woken: bool,
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
        "無法啟動區域網路搜尋；請確認防火牆允許 DisplayMux 使用私人網路".to_owned()
    })?;
    let peers = discovery.peers().map_err(user_error)?;
    refresh_paired_endpoints(&state, &peers)?;
    Ok(peers)
}

#[tauri::command]
fn select_peer(peer_id: String, state: State<'_, AppRuntime>) -> Result<AppSettings, String> {
    let discovery = state
        .discovery
        .as_ref()
        .ok_or_else(|| "區域網路搜尋目前不可用".to_owned())?;
    let peer = discovery
        .peers()
        .map_err(user_error)?
        .into_iter()
        .find(|peer| peer.id == peer_id)
        .ok_or_else(|| "這台主機已離線，請重新搜尋後再試一次".to_owned())?;
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
    let monitor = enumerate_monitor_inventory()
        .map_err(user_error)?
        .controllable
        .into_iter()
        .find(|monitor| monitor.id.as_str() == monitor_id)
        .ok_or_else(|| "找不到這台螢幕，請重新整理後再選擇".to_owned())?;
    let mut settings = read_settings(&state)?;
    settings.shared_monitor = Some(SelectedMonitor::from(&monitor));
    store_settings(&state, settings)
}

#[tauri::command]
fn get_settings(state: State<'_, AppRuntime>) -> Result<AppSettings, String> {
    read_settings(&state)
}

#[tauri::command]
fn get_input_options() -> Vec<InputOption> {
    (1..=0x12)
        .filter_map(|value| DisplayInput::new(value).ok())
        .filter_map(|input| {
            input.standard_name().map(|name| InputOption {
                value: input.value(),
                code: format!("0x{:02X}", input.value()),
                name: name.to_owned(),
            })
        })
        .collect()
}

#[tauri::command]
async fn save_settings(
    settings: AppSettings,
    state: State<'_, AppRuntime>,
    app: AppHandle,
) -> Result<OperationResult, String> {
    let settings = settings_for_current_build(settings);
    validate_settings(&settings).map_err(user_error)?;
    let enable_autostart = settings.autostart;
    store_settings(&state, settings)?;
    let autostart = app.autolaunch();
    if enable_autostart {
        autostart.enable().map_err(user_error)?;
    } else {
        autostart.disable().map_err(user_error)?;
    }
    restart_agent(&state).await?;
    Ok(OperationResult {
        title: "設定已儲存".to_owned(),
        detail: "共用螢幕、各主機輸入與配對設定已更新。".to_owned(),
        peer_woken: false,
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
        return Err("目前沒有可安裝的更新".to_owned());
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
                store_settings(&state, settings.clone())?;
                selection_notice = match change {
                    MonitorSelectionChange::SelectedOnlyMonitor { name } => {
                        Some(format!("已自動選取唯一可控制的 DDC/CI 螢幕：{name}"))
                    }
                    MonitorSelectionChange::ReplacedMissingMonitor {
                        previous,
                        replacement,
                    } => Some(format!(
                        "先前選取的 {previous} 已消失；已安全更新為唯一可控制的 {replacement}"
                    )),
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
                (None, _, _, _) => "尚未選擇共用螢幕；目前不會控制任何螢幕".to_owned(),
                (Some(selected), true, _, _) => format!("已鎖定共用螢幕：{}", selected.name),
                (Some(selected), false, true, _) => {
                    format!("已偵測到 {}，但目前無法讀取 DDC/CI 輸入", selected.name)
                }
                (Some(_), false, false, true) => "目前沒有可用的 DDC/CI 顯示器".to_owned(),
                (Some(selected), false, false, false) => {
                    format!("找不到先前選擇的共用螢幕：{}", selected.name)
                }
            };
            (inventory.controllable, status)
        }
        Err(error) => (Vec::new(), error.to_string()),
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
    let response = request_peer(&settings, peer, AgentAction::Ping).await?;
    Ok(OperationResult {
        title: format!("{} 已連線", peer.name),
        detail: response.message,
        peer_woken: false,
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
        title: format!("已送出喚醒訊號給 {}", peer.name),
        detail: "主機是否能喚醒仍取決於電源與網路設定。".to_owned(),
        peer_woken: true,
    })
}

#[tauri::command]
async fn switch_host(
    target_id: String,
    force: bool,
    state: State<'_, AppRuntime>,
) -> Result<OperationResult, String> {
    let settings = read_settings(&state)?;
    let input = if target_id == "local" {
        settings
            .local_input
            .ok_or_else(|| "尚未設定這台主機使用的螢幕輸入".to_owned())?
    } else {
        find_peer(&settings, &target_id)?
            .input
            .ok_or_else(|| "尚未設定這台主機使用的螢幕輸入".to_owned())?
    };
    let mut peer_woken = false;
    if requires_network_preflight(&target_id, force) {
        let target = find_peer(&settings, &target_id)?;
        peer_woken = prepare_safe_switch(&settings, target).await?;
    }
    match run_local_switch(&settings, input) {
        Ok(outcome) => Ok(outcome_result(
            outcome,
            peer_woken,
            force && target_id != "local",
        )),
        Err(local_error) => {
            let executor = if target_id == "local" {
                (settings.peers.len() == 1).then(|| &settings.peers[0])
            } else {
                settings.peers.iter().find(|peer| peer.id == target_id)
            }
            .ok_or_else(|| {
                format!("本機無法切換，而且沒有其他已配對主機可代為執行：{local_error}")
            })?;
            let response = request_peer(&settings, executor, AgentAction::SwitchInput { input })
                .await
                .map_err(|remote_error| {
                    format!(
                        "本機與 {} 都無法切換。本機：{}；遠端：{}",
                        executor.name, local_error, remote_error
                    )
                })?;
            Ok(OperationResult {
                title: format!("已由 {} 執行切換", executor.name),
                detail: response.message,
                peer_woken,
            })
        }
    }
}

fn requires_network_preflight(target_id: &str, direct: bool) -> bool {
    target_id != "local" && !direct
}

fn outcome_result(
    outcome: SwitchOutcome,
    peer_woken: bool,
    direct_without_readiness_check: bool,
) -> OperationResult {
    let mut result = match outcome {
        SwitchOutcome::DryRun { .. } => OperationResult {
            title: "檢查完成".to_owned(),
            detail: "未變更螢幕輸入。".to_owned(),
            peer_woken,
        },
        SwitchOutcome::AlreadySelected { target, input } => OperationResult {
            title: "已在指定輸入".to_owned(),
            detail: format!("{} 已使用 {}。", target.name, input.display_name()),
            peer_woken,
        },
        SwitchOutcome::Switched {
            target,
            previous,
            selected,
        } => OperationResult {
            title: "共用螢幕已切換".to_owned(),
            detail: format!(
                "{} 已由 {} 切換至 {}。",
                target.name,
                previous.display_name(),
                selected.display_name()
            ),
            peer_woken,
        },
    };
    if direct_without_readiness_check {
        result.title = "已直接切換共用螢幕".to_owned();
        result
            .detail
            .push_str(" 未確認目標主機是否就緒；若該主機離線，螢幕可能暫時呈現黑畫面。");
    }
    result
}

async fn prepare_safe_switch(settings: &AppSettings, peer: &HostRoute) -> Result<bool, String> {
    if !has_valid_shared_key(&settings.shared_key) {
        return Err(format!(
            "網路 Agent 尚未設定至少 {MIN_SHARED_KEY_LENGTH} 個字元的配對密碼。可改用「直接切換」；本機 DDC/CI 不需要配對密碼，但對端離線時可能黑畫面。"
        ));
    }
    if request_peer(settings, peer, AgentAction::Ping)
        .await
        .is_ok()
    {
        return Ok(false);
    }

    if let Err(error) = wake_route(settings, peer).await {
        return Err(format!(
            "無法確認 {} 已就緒，且無法自動喚醒（{}）。可改用「直接切換」，但對端離線時可能黑畫面。",
            peer.name, error
        ));
    }
    wait_until_peer_ready(settings, peer)
        .await
        .map_err(|error| format!("{error} 可改用「直接切換」，但對端離線時可能黑畫面。"))?;
    Ok(true)
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
    Err(format!(
        "已送出喚醒訊號，但 {} 在 {} 秒內沒有回應；為避免黑畫面，尚未切換螢幕",
        peer.name, attempts
    ))
}

async fn request_peer(
    settings: &AppSettings,
    peer: &HostRoute,
    action: AgentAction,
) -> Result<AgentResponse, String> {
    let endpoint = route_endpoint(peer).map_err(user_error)?;
    if !has_valid_shared_key(&settings.shared_key) {
        return Err(format!(
            "請先設定至少 {MIN_SHARED_KEY_LENGTH} 個字元的配對密碼"
        ));
    }
    let response = AgentClient::new(endpoint, Arc::<[u8]>::from(settings.shared_key.as_bytes()))
        .request(action, next_nonce())
        .await
        .map_err(user_error)?;
    if response.ready {
        Ok(response)
    } else {
        Err(response.message)
    }
}

async fn wake_route(settings: &AppSettings, peer: &HostRoute) -> Result<(), String> {
    if peer.mac_address.trim().is_empty() {
        return Err(format!(
            "{} 沒有提供可用的 MAC 位址，因此無法使用 Wake-on-LAN；主機醒著時仍可切換",
            peer.name
        ));
    }
    let mac_address = MacAddress::from_str(&peer.mac_address).map_err(user_error)?;
    let broadcast_address =
        Ipv4Addr::from_str(&settings.broadcast_ip).map_err(|_| "廣播位址格式無效".to_owned())?;
    WakeTarget {
        mac_address,
        broadcast_address,
        port: settings.wake_port,
    }
    .wake()
    .await
    .map_err(user_error)
}

fn route_endpoint(peer: &HostRoute) -> Result<PeerEndpoint, DisplayMuxError> {
    let address = IpAddr::from_str(&peer.address)
        .map_err(|_| DisplayMuxError::PeerUnavailable(format!("{} 的 IP 位址無效", peer.name)))?;
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
        .ok_or_else(|| "找不到這台已配對主機，請重新搜尋並加入".to_owned())
}

fn validate_settings(settings: &AppSettings) -> Result<(), DisplayMuxError> {
    if settings.local_host != local_host() {
        return Err(DisplayMuxError::Backend(
            "這台電腦的主機類型必須由作業系統自動判定".to_owned(),
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
            "螢幕輸入值必須介於 0x01 與 0xFF".to_owned(),
        ));
    }
    for peer in &settings.peers {
        route_endpoint(peer)?;
        if !peer.mac_address.trim().is_empty() {
            MacAddress::from_str(&peer.mac_address)?;
        }
    }
    Ipv4Addr::from_str(&settings.broadcast_ip)
        .map_err(|_| DisplayMuxError::WakeFailed("廣播位址格式無效".to_owned()))?;
    if !settings.shared_key.is_empty() && !has_valid_shared_key(&settings.shared_key) {
        return Err(DisplayMuxError::Backend(format!(
            "配對密碼至少需要 {MIN_SHARED_KEY_LENGTH} 個字元"
        )));
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
                            message: "DisplayMux Agent 已就緒".to_owned(),
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
                                    message: "這台主機尚未選擇共用螢幕".to_owned(),
                                };
                            };
                            match tauri::async_runtime::spawn_blocking(move || {
                                run_switch(fingerprint, input)
                            })
                            .await
                            {
                                Ok(Ok(_)) => AgentResponse {
                                    ready: true,
                                    message: format!("遠端主機已切換至 {}", input.display_name()),
                                },
                                Ok(Err(error)) => AgentResponse {
                                    ready: false,
                                    message: error.to_string(),
                                },
                                Err(error) => AgentResponse {
                                    ready: false,
                                    message: format!("切換工作無法執行：{error}"),
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
    persist_settings(&state.settings_path, &settings).map_err(user_error)?;
    let mut current = state
        .settings
        .write()
        .map_err(|_| "無法更新設定，請重新啟動 DisplayMux".to_owned())?;
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
        .map_err(|_| "無法讀取設定，請重新啟動 DisplayMux".to_owned())
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

fn update_error(error: impl std::fmt::Display) -> String {
    tracing::warn!(error = %error, "application update check failed");
    "無法檢查更新；請確認網路可連線至 GitHub Releases，稍後再試一次".to_owned()
}

fn update_install_error(error: impl std::fmt::Display) -> String {
    tracing::error!(error = %error, "signed application update installation failed");
    "更新下載或簽章驗證失敗；目前版本未變更，請稍後再試一次".to_owned()
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
        .text("tray-open", "開啟 DisplayMux")
        .separator()
        .text("tray-quit", "結束 DisplayMux")
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
    fn direct_switch_bypasses_network_preflight_but_safe_switch_does_not() {
        assert!(requires_network_preflight("peer", false));
        assert!(!requires_network_preflight("peer", true));
        assert!(!requires_network_preflight("local", false));
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
    fn direct_switch_result_always_warns_about_the_black_screen_risk() {
        let target = monitor("external");
        let input = DisplayInput::new(0x11).unwrap();
        let result = outcome_result(
            SwitchOutcome::AlreadySelected { target, input },
            false,
            true,
        );

        assert!(result.title.contains("直接切換"));
        assert!(result.detail.contains("未確認目標主機"));
        assert!(result.detail.contains("黑畫面"));
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
