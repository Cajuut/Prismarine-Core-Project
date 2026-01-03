mod bridge;
mod config;
mod java_detector;
mod monitor;
mod port_manager;
mod server_manager;

use bridge::{BridgeStatus, PrismarineBridge};
use monitor::Monitor;
use port_manager::PortManager;
use server_manager::{RestartType, ServerManager, ServerType};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tauri::State;
use tokio::sync::Mutex as TokioMutex;

// App state
pub struct AppState {
    server_manager: Arc<TokioMutex<ServerManager>>,
    port_manager: Arc<PortManager>,
    monitor: Arc<Mutex<Monitor>>,
    bridge: Arc<PrismarineBridge>,
    #[allow(dead_code)]
    config_path: PathBuf,
}

// Tauri commands

#[tauri::command]
async fn create_server(
    name: String,
    version: String,
    server_type: String,
    port: u16,
    max_memory: String,
    state: State<'_, AppState>,
) -> Result<server_manager::ServerInfo, String> {
    let st = match server_type.as_str() {
        "vanilla" => ServerType::Vanilla,
        "paper" => ServerType::Paper,
        "spigot" => ServerType::Spigot,
        "forge" => ServerType::Forge,
        "fabric" => ServerType::Fabric,
        "mohist" => ServerType::Mohist,
        "taiyitist" => ServerType::Taiyitist,
        "purpur" => ServerType::Purpur,
        "banner" => ServerType::Banner,
        "velocity" => ServerType::Velocity,
        "waterfall" => ServerType::Waterfall,
        "bungeecord" => ServerType::BungeeCord,
        _ => return Err("Invalid server type".to_string()),
    };

    let manager = state.server_manager.lock().await;
    let result = manager
        .create_server(name, version, st, port, max_memory)
        .await
        .map_err(|e| e.to_string())?;

    // Save servers after creation
    let _ = manager.save_servers(&state.config_path).await;

    Ok(result)
}

#[tauri::command]
async fn start_server(server_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let manager = state.server_manager.lock().await;
    manager
        .start_server(&server_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn stop_server(server_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let manager = state.server_manager.lock().await;
    manager
        .stop_server(&server_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_server(server_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let manager = state.server_manager.lock().await;
    manager
        .delete_server(&server_id)
        .await
        .map_err(|e| e.to_string())?;

    // Save servers after deletion
    let _ = manager.save_servers(&state.config_path).await;

    Ok(())
}

#[tauri::command]
async fn send_server_command(
    server_id: String,
    command: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let manager = state.server_manager.lock().await;
    manager
        .send_command(&server_id, &command)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn install_geyser_support(
    server_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let manager = state.server_manager.lock().await;
    manager
        .install_geyser(&server_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn install_viaversion_support(
    server_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let manager = state.server_manager.lock().await;
    manager
        .install_viaversion(&server_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn is_geyser_installed(
    server_id: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let manager = state.server_manager.lock().await;
    manager
        .check_geyser_installed(&server_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn is_viaversion_installed(
    server_id: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let manager = state.server_manager.lock().await;
    manager
        .check_viaversion_installed(&server_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn uninstall_geyser_support(
    server_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let manager = state.server_manager.lock().await;
    manager
        .uninstall_geyser(&server_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn uninstall_viaversion_support(
    server_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let manager = state.server_manager.lock().await;
    manager
        .uninstall_viaversion(&server_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn search_plugins(
    server_id: String,
    query: String,
    source: String,
    state: State<'_, AppState>,
) -> Result<Vec<server_manager::PluginSearchResult>, String> {
    let manager = state.server_manager.lock().await;
    manager
        .search_plugins(&server_id, &query, &source)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn install_plugin(
    server_id: String,
    download_url: String,
    filename: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let manager = state.server_manager.lock().await;
    manager
        .install_plugin_by_url(&server_id, &download_url, filename)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn install_modrinth_plugin(
    server_id: String,
    project_id: String,
    plugin_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let manager = state.server_manager.lock().await;
    manager
        .install_modrinth_plugin(&server_id, &project_id, &plugin_name)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn install_spigot_plugin(
    server_id: String,
    resource_id: String,
    plugin_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let manager = state.server_manager.lock().await;
    manager
        .install_spigot_plugin(&server_id, &resource_id, &plugin_name)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn uninstall_plugin(
    server_id: String,
    plugin_name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let manager = state.server_manager.lock().await;
    manager
        .uninstall_plugin(&server_id, &plugin_name)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn is_plugin_installed(
    server_id: String,
    plugin_name: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let manager = state.server_manager.lock().await;
    manager
        .is_plugin_installed(&server_id, &plugin_name)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_servers(
    state: State<'_, AppState>,
) -> Result<Vec<server_manager::ServerInfo>, String> {
    let manager = state.server_manager.lock().await;
    Ok(manager.get_servers().await)
}

#[tauri::command]
async fn get_server(
    server_id: String,
    state: State<'_, AppState>,
) -> Result<Option<server_manager::ServerInfo>, String> {
    let manager = state.server_manager.lock().await;
    Ok(manager.get_server(&server_id).await)
}

#[tauri::command]
async fn open_managed_port(
    port: u16,
    protocol: String,
    name: String,
    slot: u8,
    state: State<'_, AppState>,
) -> Result<String, String> {
    state
        .port_manager
        .open_managed_port(port, &protocol, &name, slot)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn close_managed_port(slot: u8, state: State<'_, AppState>) -> Result<(), String> {
    state
        .port_manager
        .close_managed_port(slot)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn delete_managed_port(slot: u8, state: State<'_, AppState>) -> Result<(), String> {
    state
        .port_manager
        .delete_managed_port(slot)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_managed_port_active(
    slot: u8,
    active: bool,
    state: State<'_, AppState>,
) -> Result<String, String> {
    state
        .port_manager
        .set_managed_port_active(slot, active)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_managed_ports(
    state: State<'_, AppState>,
) -> Result<Vec<port_manager::ManagedPort>, String> {
    Ok(state.port_manager.get_managed_ports())
}

#[tauri::command]
async fn get_external_ip(state: State<'_, AppState>) -> Result<String, String> {
    state
        .port_manager
        .get_external_ip()
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn is_upnp_available(state: State<'_, AppState>) -> Result<bool, String> {
    Ok(state.port_manager.is_upnp_available().await)
}

#[tauri::command]
fn get_system_stats(state: State<'_, AppState>) -> Result<monitor::SystemStats, String> {
    let mut monitor = state.monitor.lock().unwrap();
    Ok(monitor.get_system_stats())
}

#[tauri::command]
async fn get_server_logs(
    server_id: String,
    lines: usize,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let server_path = {
        let manager = state.server_manager.lock().await;
        if let Some(server) = manager.get_server(&server_id).await {
            server.path.clone()
        } else {
            return Err("Server not found".to_string());
        }
    };

    Monitor::get_server_logs(&server_path, lines)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn open_folder(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Failed to open folder: {}", e))?;
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }

    Ok(())
}

#[tauri::command]
async fn get_motd(server_id: String, state: State<'_, AppState>) -> Result<String, String> {
    let manager = state.server_manager.lock().await;
    manager
        .get_server_motd(&server_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_motd(
    server_id: String,
    motd: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let manager = state.server_manager.lock().await;
    manager
        .set_server_motd(&server_id, &motd)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_max_players(server_id: String, state: State<'_, AppState>) -> Result<u32, String> {
    let manager = state.server_manager.lock().await;
    manager
        .get_server_max_players(&server_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_max_players(
    server_id: String,
    max_players: u32,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let manager = state.server_manager.lock().await;
    manager
        .set_server_max_players(&server_id, max_players)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn open_server_folder(server_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let server_path = {
        let manager = state.server_manager.lock().await;
        if let Some(server) = manager.get_server(&server_id).await {
            server.path.clone()
        } else {
            return Err("Server not found".to_string());
        }
    };

    open_folder(server_path.to_string_lossy().to_string()).await
}

#[tauri::command]
async fn open_plugins_folder(server_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let server_path = {
        let manager = state.server_manager.lock().await;
        manager
            .get_plugins_path(&server_id)
            .await
            .map_err(|e| e.to_string())?
    };

    // Ensure plugins folder exists
    if !server_path.exists() {
        let _ = std::fs::create_dir_all(&server_path);
    }

    open_folder(server_path.to_string_lossy().to_string()).await
}

#[tauri::command]
async fn restart_server(server_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let manager = state.server_manager.lock().await;
    manager
        .restart_server(&server_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_online_players(
    server_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let server_path = {
        let manager = state.server_manager.lock().await;
        if let Some(server) = manager.get_server(&server_id).await {
            server.path.clone()
        } else {
            return Err("Server not found".to_string());
        }
    };

    Monitor::get_online_players(&server_path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_auto_restart(
    server_id: String,
    enabled: bool,
    restart_type: String, // "Interval" or "Schedule"
    interval: u64,
    schedule: Option<String>,
    time_zone: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let r_type = match restart_type.as_str() {
        "Schedule" => RestartType::Schedule,
        _ => RestartType::Interval,
    };

    let manager = state.server_manager.lock().await;
    manager
        .set_auto_restart(&server_id, enabled, r_type, interval, schedule, time_zone)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn set_server_memory(
    server_id: String,
    memory: String, // This will be max_memory from frontend
    min_memory: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let manager = state.server_manager.lock().await;
    manager
        .set_server_memory(&server_id, &memory, &min_memory)
        .await
        .map_err(|e| e.to_string())?;

    manager
        .save_servers(&state.config_path)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn fetch_versions(
    server_type: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let manager = state.server_manager.lock().await;
    match server_type.as_str() {
        "vanilla" => manager
            .fetch_vanilla_versions()
            .await
            .map_err(|e| e.to_string()),
        "paper" => manager
            .fetch_paper_versions()
            .await
            .map_err(|e| e.to_string()),
        "fabric" => manager
            .fetch_fabric_versions()
            .await
            .map_err(|e| e.to_string()),
        "mohist" => manager
            .fetch_mohist_versions()
            .await
            .map_err(|e| e.to_string()),
        "taiyitist" => manager
            .fetch_taiyitist_versions()
            .await
            .map_err(|e| e.to_string()),
        "purpur" => manager
            .fetch_purpur_versions()
            .await
            .map_err(|e| e.to_string()),
        "banner" => manager
            .fetch_banner_versions()
            .await
            .map_err(|e| e.to_string()),
        "spigot" => manager
            .fetch_spigot_versions()
            .await
            .map_err(|e| e.to_string()),
        "velocity" => manager
            .fetch_velocity_versions()
            .await
            .map_err(|e| e.to_string()),
        "waterfall" => manager
            .fetch_waterfall_versions()
            .await
            .map_err(|e| e.to_string()),
        "bungeecord" => manager
            .fetch_bungeecord_versions()
            .await
            .map_err(|e| e.to_string()),
        _ => Err("Unsupported server type".to_string()),
    }
}

#[tauri::command]
async fn get_proxy_servers(
    proxy_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<server_manager::ProxyServerEntry>, String> {
    let manager = state.server_manager.lock().await;
    manager
        .get_proxy_registered_servers(&proxy_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn add_proxy_server(
    proxy_id: String,
    name: String,
    address: String,
    add_to_try: Option<bool>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let manager = state.server_manager.lock().await;
    manager
        .add_server_to_proxy(&proxy_id, &name, &address, add_to_try.unwrap_or(true))
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn remove_proxy_server(
    proxy_id: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let manager = state.server_manager.lock().await;
    manager
        .remove_server_from_proxy(&proxy_id, &name)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn configure_backend_for_proxy(
    backend_id: String,
    proxy_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let manager = state.server_manager.lock().await;
    manager
        .configure_backend_for_proxy(&backend_id, &proxy_id)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn start_bridge(
    port: u16,
    remote_server: Option<String>,
    secret: Option<String>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    // First ensure binary is installed
    state
        .bridge
        .ensure_installed()
        .await
        .map_err(|e| e.to_string())?;
    // Then start the bridge
    state
        .bridge
        .start(port, remote_server, secret)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn stop_bridge(state: State<'_, AppState>) -> Result<(), String> {
    state.bridge.stop().map_err(|e| e.to_string())
}

#[tauri::command]
fn get_bridge_status(state: State<'_, AppState>) -> BridgeStatus {
    state.bridge.get_status()
}

#[tauri::command]
fn is_bridge_installed(state: State<'_, AppState>) -> bool {
    state.bridge.is_installed()
}

#[tauri::command]
fn is_bridge_running(state: State<'_, AppState>) -> bool {
    state.bridge.is_running()
}

#[tauri::command]
fn set_bridge_authtoken(token: String, state: State<'_, AppState>) -> Result<(), String> {
    state
        .bridge
        .set_authtoken(&token)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn has_bridge_authtoken(state: State<'_, AppState>) -> bool {
    state.bridge.has_authtoken()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Initialize app state
    let config_path = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("MinecraftServerManager")
        .join("config.json");

    let base_path = dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("MinecraftServerManager")
        .join("servers");

    let server_manager = Arc::new(TokioMutex::new(ServerManager::new(base_path)));
    let port_manager = Arc::new(PortManager::new());
    let monitor = Arc::new(Mutex::new(Monitor::new()));
    let bridge = Arc::new(PrismarineBridge::new());

    let app_state = AppState {
        server_manager: Arc::clone(&server_manager),
        port_manager,
        monitor,
        bridge,
        config_path: config_path.clone(),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_shell::init())
        .manage(app_state)
        .setup(move |_app| {
            // Spawn background task for auto-restart monitor
            let monitor_manager = Arc::clone(&server_manager);
            tauri::async_runtime::spawn(async move {
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_secs(30)).await;
                    let manager = monitor_manager.lock().await;
                    manager.check_and_restart_servers().await;
                }
            });

            // Load saved servers in setup hook (inside Tauri's async runtime)
            tauri::async_runtime::spawn(async move {
                let manager = server_manager.lock().await;
                let _ = manager.load_servers(&config_path).await;
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            create_server,
            start_server,
            stop_server,
            delete_server,
            get_servers,
            get_server,
            open_managed_port,
            close_managed_port,
            delete_managed_port,
            set_managed_port_active,
            get_managed_ports,
            get_external_ip,
            is_upnp_available,
            get_system_stats,
            get_server_logs,
            send_server_command,
            open_folder,
            fetch_versions,
            get_motd,
            set_motd,
            get_max_players,
            set_max_players,
            start_bridge,
            stop_bridge,
            get_bridge_status,
            is_bridge_installed,
            is_bridge_running,
            set_bridge_authtoken,
            has_bridge_authtoken,
            install_geyser_support,
            install_viaversion_support,
            is_geyser_installed,
            is_viaversion_installed,
            uninstall_geyser_support,
            uninstall_viaversion_support,
            search_plugins,
            install_plugin,
            install_modrinth_plugin,
            install_spigot_plugin,
            uninstall_plugin,
            is_plugin_installed,
            open_server_folder,
            open_plugins_folder,
            restart_server,
            get_online_players,
            set_auto_restart,
            set_server_memory,
            get_proxy_servers,
            add_proxy_server,
            remove_proxy_server,
            configure_backend_for_proxy,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
