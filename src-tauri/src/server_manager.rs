use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_yaml;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use toml;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum RestartType {
    Interval,
    Schedule,
}

impl Default for RestartType {
    fn default() -> Self {
        RestartType::Interval
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ServerInfo {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
    pub version: String,
    pub server_type: ServerType,
    pub status: ServerStatus,
    #[serde(default)]
    pub pid: Option<u32>,
    pub port: u16,
    pub max_memory: String,
    #[serde(default = "default_min_memory")]
    pub min_memory: String,
    #[serde(default)]
    pub players: String, // e.g. "0/20"
    #[serde(default)]
    pub auto_restart: bool,
    #[serde(default = "default_restart_interval")]
    pub restart_interval: u64, // seconds
    #[serde(default)]
    pub restart_type: RestartType,
    #[serde(default)]
    pub restart_schedule: Option<String>, // "HH:MM:SS"
    #[serde(default)]
    pub time_zone: Option<String>, // e.g. "Asia/Tokyo"
    #[serde(default)]
    pub last_start_time: Option<u64>,
}

fn default_restart_interval() -> u64 {
    86400 // 24 hours
}

fn default_min_memory() -> String {
    "1G".to_string()
}

/// Parse memory string (e.g., "4G", "2048M") to megabytes
fn parse_memory_mb(memory: &str) -> Option<u64> {
    let memory = memory.trim().to_uppercase();
    if memory.ends_with('G') {
        memory[..memory.len() - 1]
            .parse::<u64>()
            .ok()
            .map(|g| g * 1024)
    } else if memory.ends_with('M') {
        memory[..memory.len() - 1].parse::<u64>().ok()
    } else {
        memory.parse::<u64>().ok()
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PluginSearchResult {
    pub id: String,
    pub name: String,
    pub description: String,
    pub author: String,
    pub icon_url: Option<String>,
    pub source: String, // "Modrinth" or "Spigot"
    pub external_url: String,
    pub download_url: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub enum ServerType {
    Vanilla,
    Paper,
    Spigot,
    Forge,
    Fabric,
    Mohist,
    Taiyitist,
    Purpur,
    Banner,
    BungeeCord,
    Velocity,
    Waterfall,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyServerEntry {
    pub name: String,
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ServerStatus {
    Stopped,
    Starting,
    Running,
    Stopping,
}

pub struct ServerManager {
    servers: Arc<Mutex<HashMap<String, ServerInfo>>>,
    processes: Arc<std::sync::Mutex<HashMap<String, Child>>>,
    base_path: PathBuf,
}

impl ServerManager {
    pub fn new(base_path: PathBuf) -> Self {
        Self {
            servers: Arc::new(Mutex::new(HashMap::new())),
            processes: Arc::new(std::sync::Mutex::new(HashMap::new())),
            base_path,
        }
    }

    /// Save servers to JSON file
    pub async fn save_servers(&self, config_path: &Path) -> Result<()> {
        let servers = self.servers.lock().await;
        let server_list: Vec<ServerInfo> = servers.values().cloned().collect();

        let content = serde_json::to_string_pretty(&server_list)?;

        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        fs::write(config_path, content).await?;
        Ok(())
    }

    /// Load servers from JSON file
    pub async fn load_servers(&self, config_path: &Path) -> Result<()> {
        if config_path.exists() {
            let content = fs::read_to_string(config_path).await?;
            let server_list: Vec<ServerInfo> = serde_json::from_str(&content)?;

            let mut servers = self.servers.lock().await;
            for server in server_list {
                servers.insert(server.id.clone(), server);
            }
        }
        Ok(())
    }

    pub async fn create_server(
        &self,
        name: String,
        version: String,
        server_type: ServerType,
        port: u16,
        max_memory: String,
    ) -> Result<ServerInfo> {
        let id = uuid::Uuid::new_v4().to_string();
        let server_path = self.base_path.join(&id);

        // Create server directory
        fs::create_dir_all(&server_path)
            .await
            .context("Failed to create server directory")?;

        // Download server JAR
        self.download_server_jar(&server_path, &server_type, &version)
            .await?;

        // Create default server.properties
        self.create_default_properties(&server_path, port).await?;

        // Accept EULA
        fs::write(server_path.join("eula.txt"), "eula=true").await?;

        // Default min_memory to same as max for new servers, or 1G?
        // Let's default to max_memory for simplicity/Aikar's recommendation,
        // but user can change it. Actually user wants to decide.
        // I'll initialize it to max_memory for now so it doesn't break.
        let min_memory = max_memory.clone();

        let server_info = ServerInfo {
            id: id.clone(),
            name,
            version,
            server_type,
            port,
            max_memory,
            min_memory,
            status: ServerStatus::Stopped,
            path: server_path,
            pid: None,
            players: "0/20".to_string(),
            auto_restart: false,
            restart_interval: 86400,
            restart_type: RestartType::Interval,
            restart_schedule: None,
            time_zone: None,
            last_start_time: None,
        };

        self.servers.lock().await.insert(id, server_info.clone());
        Ok(server_info)
    }

    pub async fn set_auto_restart(
        &self,
        server_id: &str,
        enabled: bool,
        restart_type: RestartType,
        interval: u64,
        schedule: Option<String>,
        time_zone: Option<String>,
    ) -> Result<()> {
        let mut servers = self.servers.lock().await;

        if let Some(server) = servers.get_mut(server_id) {
            server.auto_restart = enabled;
            server.restart_type = restart_type;
            server.restart_interval = interval;
            server.restart_schedule = schedule;
            server.time_zone = time_zone;
            Ok(())
        } else {
            anyhow::bail!("Server not found")
        }
    }

    pub async fn start_server(&self, server_id: &str) -> Result<()> {
        let server_info = {
            let mut servers = self.servers.lock().await;
            let server = servers.get_mut(server_id).context("Server not found")?;

            if server.status == ServerStatus::Running {
                anyhow::bail!("Server is already running");
            }

            server.status = ServerStatus::Starting;
            server.last_start_time = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_secs(),
            );
            server.clone()
        };

        let jar_path = server_info.path.join("server.jar");

        // Auto-select Java based on Minecraft version
        let java_cmd = crate::java_detector::select_java_for_minecraft(&server_info.version)
            .unwrap_or_else(|| {
                // Fallback: Try JAVA_HOME, then system java
                std::env::var("JAVA_HOME")
                    .ok()
                    .map(|java_home| {
                        #[cfg(target_os = "windows")]
                        {
                            format!("{}\\bin\\java.exe", java_home)
                        }
                        #[cfg(not(target_os = "windows"))]
                        {
                            format!("{}/bin/java", java_home)
                        }
                    })
                    .unwrap_or_else(|| "java".to_string())
            });

        // Build JVM arguments with performance optimizations
        let mut jvm_args = vec![
            format!("-Xmx{}", server_info.max_memory),
            format!("-Xms{}", server_info.min_memory),
            // G1GC garbage collector (optimal for Minecraft)
            "-XX:+UseG1GC".to_string(),
            "-XX:+ParallelRefProcEnabled".to_string(),
            "-XX:MaxGCPauseMillis=200".to_string(),
            "-XX:+UnlockExperimentalVMOptions".to_string(),
            "-XX:+DisableExplicitGC".to_string(),
            "-XX:+AlwaysPreTouch".to_string(),
            "-XX:G1HeapWastePercent=5".to_string(),
            "-XX:G1MixedGCCountTarget=4".to_string(),
            "-XX:G1MixedGCLiveThresholdPercent=90".to_string(),
            "-XX:G1RSetUpdatingPauseTimePercent=5".to_string(),
            "-XX:SurvivorRatio=32".to_string(),
            "-XX:+PerfDisableSharedMem".to_string(),
            "-XX:MaxTenuringThreshold=1".to_string(),
            // Server JAR arguments
            "-jar".to_string(),
            jar_path.to_string_lossy().to_string(),
            "nogui".to_string(),
        ];

        // Add G1NewSizePercent and G1ReservePercent for larger heap sizes
        if let Some(mem_mb) = parse_memory_mb(&server_info.max_memory) {
            if mem_mb >= 12288 {
                // 12GB+
                jvm_args.insert(7, "-XX:G1NewSizePercent=40".to_string());
                jvm_args.insert(8, "-XX:G1MaxNewSizePercent=50".to_string());
                jvm_args.insert(9, "-XX:G1ReservePercent=15".to_string());
                jvm_args.insert(10, "-XX:InitiatingHeapOccupancyPercent=15".to_string());
            } else {
                jvm_args.insert(7, "-XX:G1NewSizePercent=30".to_string());
                jvm_args.insert(8, "-XX:G1MaxNewSizePercent=40".to_string());
                jvm_args.insert(9, "-XX:G1ReservePercent=20".to_string());
                jvm_args.insert(10, "-XX:InitiatingHeapOccupancyPercent=20".to_string());
            }
        }

        let child = Command::new(java_cmd)
            .args(&jvm_args)
            .current_dir(&server_info.path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()
            .context("Failed to start server process")?;

        self.processes
            .lock()
            .unwrap()
            .insert(server_id.to_string(), child);

        let mut servers = self.servers.lock().await;
        if let Some(server) = servers.get_mut(server_id) {
            server.status = ServerStatus::Running;
        }

        Ok(())
    }

    pub async fn stop_server(&self, server_id: &str) -> Result<()> {
        // Set status to Stopping first
        {
            let mut servers = self.servers.lock().await;
            if let Some(server) = servers.get_mut(server_id) {
                // Already stopped or stopping - skip
                if server.status == ServerStatus::Stopped || server.status == ServerStatus::Stopping
                {
                    return Ok(());
                }
                server.status = ServerStatus::Stopping;
            }
        }

        // Try to send "stop" command for graceful shutdown
        let graceful_attempt = self.send_command(server_id, "stop").await;
        let start_time = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(30);

        if graceful_attempt.is_ok() {
            // Wait for server to shut down gracefully using try_wait()
            // Poll every 200ms for faster response
            loop {
                if start_time.elapsed() >= timeout {
                    println!("[ServerManager] Graceful shutdown timeout reached");
                    break;
                }

                // Check if process has exited using try_wait()
                let process_exited = {
                    let mut processes = self.processes.lock().unwrap();
                    if let Some(process) = processes.get_mut(server_id) {
                        match process.try_wait() {
                            Ok(Some(_exit_status)) => {
                                // Process has exited
                                println!("[ServerManager] Process exited gracefully");
                                true
                            }
                            Ok(None) => {
                                // Process still running
                                false
                            }
                            Err(_) => {
                                // Error checking status, assume still running
                                false
                            }
                        }
                    } else {
                        // Process not in map, already removed
                        true
                    }
                };

                if process_exited {
                    // Remove from processes map
                    self.processes.lock().unwrap().remove(server_id);
                    break;
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;
            }

            println!(
                "[ServerManager] Graceful stop completed in {:?}",
                start_time.elapsed()
            );
        }

        // Force kill if still running (fallback)
        {
            let mut processes = self.processes.lock().unwrap();
            if let Some(mut process) = processes.remove(server_id) {
                // Try to kill if still running
                if let Err(e) = process.start_kill() {
                    println!("[ServerManager] Failed to kill process: {}", e);
                } else {
                    println!("[ServerManager] Process force killed after graceful attempt");
                }
            }
        }

        // Update server status
        let mut servers = self.servers.lock().await;
        if let Some(server) = servers.get_mut(server_id) {
            server.status = ServerStatus::Stopped;
            server.last_start_time = None;
        }

        Ok(())
    }

    /// Send a command to a running server
    pub async fn send_command(&self, server_id: &str, command: &str) -> Result<()> {
        // Get stdin handle - we need to release the lock before await
        let mut stdin_handle = {
            let mut processes = self.processes.lock().unwrap();
            let process = processes
                .get_mut(server_id)
                .context("Server not found or not running")?;

            process.stdin.take().context("Server stdin not available")?
        };

        // Write command followed by newline
        let command_line = format!("{}\n", command.trim());
        stdin_handle
            .write_all(command_line.as_bytes())
            .await
            .context("Failed to write command to server")?;
        stdin_handle
            .flush()
            .await
            .context("Failed to flush command to server")?;

        // Put stdin back
        {
            let mut processes = self.processes.lock().unwrap();
            if let Some(process) = processes.get_mut(server_id) {
                process.stdin = Some(stdin_handle);
            }
        }

        println!("Sent command to server {}: {}", server_id, command);
        Ok(())
    }

    pub async fn get_servers(&self) -> Vec<ServerInfo> {
        self.servers.lock().await.values().cloned().collect()
    }

    pub async fn get_server(&self, server_id: &str) -> Option<ServerInfo> {
        self.servers.lock().await.get(server_id).cloned()
    }

    pub async fn get_plugins_path(&self, server_id: &str) -> Result<PathBuf> {
        let server = self
            .servers
            .lock()
            .await
            .get(server_id)
            .context("Server not found")?
            .clone();

        match server.server_type {
            ServerType::Fabric
            | ServerType::Mohist
            | ServerType::Forge
            | ServerType::Taiyitist
            | ServerType::Banner => Ok(server.path.join("mods")),
            _ => Ok(server.path.join("plugins")),
        }
    }

    pub async fn delete_server(&self, server_id: &str) -> Result<()> {
        // Stop server if running
        let _ = self.stop_server(server_id).await;

        let server_info = {
            let mut servers = self.servers.lock().await;
            servers.remove(server_id).context("Server not found")?
        };

        // Delete server directory
        fs::remove_dir_all(&server_info.path)
            .await
            .context("Failed to delete server directory")?;

        Ok(())
    }

    async fn download_server_jar(
        &self,
        server_path: &Path,
        server_type: &ServerType,
        version: &str,
    ) -> Result<()> {
        let jar_path = server_path.join("server.jar");

        let url = match server_type {
            ServerType::Vanilla => self.get_vanilla_url(version).await?,
            ServerType::Paper => self.get_paper_url(version).await?,
            ServerType::Fabric => self.get_fabric_url(version).await?,
            ServerType::Mohist => self.get_mohist_url(version).await?,
            ServerType::Taiyitist => self.get_taiyitist_url(version).await?,
            ServerType::Velocity => self.get_velocity_url(version).await?,
            ServerType::Waterfall => self.get_waterfall_url(version).await?,
            ServerType::BungeeCord => self.get_bungeecord_url(version).await?,
            ServerType::Purpur => self.get_purpur_url(version).await?,
            ServerType::Banner => self.get_banner_url(version).await?,
            ServerType::Spigot => {
                // Spigot requires BuildTools - handle separately
                return self.build_spigot(server_path, version).await;
            }
            ServerType::Forge => {
                return Err(anyhow::anyhow!(
                    "Automatic download not supported for {:?}",
                    server_type
                ))
            }
        };

        println!("Downloading server JAR from: {}", url);
        // Use client with UA
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;
        let response = client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Failed to download server JAR: Status {}",
                response.status()
            ));
        }

        let content = response.bytes().await?;
        fs::write(&jar_path, content).await?;

        Ok(())
    }

    async fn get_vanilla_url(&self, version: &str) -> Result<String> {
        let manifest_url = "https://launchermeta.mojang.com/mc/game/version_manifest.json";
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;
        let manifest: serde_json::Value = client.get(manifest_url).send().await?.json().await?;

        let versions = manifest["versions"]
            .as_array()
            .context("Invalid manifest format")?;
        let version_info = versions
            .iter()
            .find(|v| v["id"].as_str() == Some(version))
            .context(format!("Version {} not found", version))?;

        let url = version_info["url"]
            .as_str()
            .context("Invalid version URL")?;
        let packet: serde_json::Value = client.get(url).send().await?.json().await?;

        let download_url = packet["downloads"]["server"]["url"]
            .as_str()
            .context("Server download URL not found")?
            .to_string();

        Ok(download_url)
    }

    async fn get_paper_url(&self, version: &str) -> Result<String> {
        let builds_url = format!(
            "https://api.papermc.io/v2/projects/paper/versions/{}/builds",
            version
        );
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;
        let builds_resp: serde_json::Value = client.get(&builds_url).send().await?.json().await?;

        let builds = builds_resp["builds"]
            .as_array()
            .context("No builds found")?;
        let latest_build = builds.last().context("No builds found")?;
        let build_number = latest_build["build"]
            .as_u64()
            .context("Invalid build number")?;
        let default_name = format!("paper-{}-{}.jar", version, build_number);
        let file_name = latest_build["downloads"]["application"]["name"]
            .as_str()
            .unwrap_or(&default_name);

        Ok(format!(
            "https://api.papermc.io/v2/projects/paper/versions/{}/builds/{}/downloads/{}",
            version, build_number, file_name
        ))
    }

    async fn get_fabric_url(&self, version: &str) -> Result<String> {
        // Step 1: Get latest loader version
        let loader_api = "https://meta.fabricmc.net/v2/versions/loader";
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;

        let loader_data: serde_json::Value = client.get(loader_api).send().await?.json().await?;
        let latest_loader = loader_data
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| v["version"].as_str())
            .context("Failed to get latest Fabric loader version")?;

        // Step 2: Get latest installer version
        let installer_api = "https://meta.fabricmc.net/v2/versions/installer";
        let installer_data: serde_json::Value =
            client.get(installer_api).send().await?.json().await?;
        let latest_installer = installer_data
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|v| v["version"].as_str())
            .context("Failed to get latest Fabric installer version")?;

        // Step 3: Build download URL
        Ok(format!(
            "https://meta.fabricmc.net/v2/versions/loader/{}/{}/{}/server/jar",
            version, latest_loader, latest_installer
        ))
    }

    async fn get_mohist_url(&self, version: &str) -> Result<String> {
        // Mohist API: Get latest build info first
        let builds_url = format!(
            "https://api.mohistmc.com/project/mohist/{}/builds/latest",
            version
        );

        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;

        let build_info: serde_json::Value = client.get(&builds_url).send().await?.json().await?;

        let build_id = build_info["id"]
            .as_i64()
            .context("Failed to get Mohist build ID")?;

        // Construct download URL with build ID
        let download_url = format!(
            "https://api.mohistmc.com/project/mohist/{}/builds/{}/download",
            version, build_id
        );

        Ok(download_url)
    }

    async fn get_taiyitist_url(&self, version: &str) -> Result<String> {
        // Taiyitist uses GitHub releases: https://github.com/Teneted/Taiyitist/releases
        // Tag format is "{version}-release" (e.g., "1.20.1-release")
        let tag = format!("{}-release", version);
        let releases_url = format!(
            "https://api.github.com/repos/Teneted/Taiyitist/releases/tags/{}",
            tag
        );

        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;

        let release_info: serde_json::Value =
            client.get(&releases_url).send().await?.json().await?;

        // Find the first .jar asset
        let assets = release_info["assets"]
            .as_array()
            .context("Failed to get release assets")?;

        for asset in assets {
            let name = asset["name"].as_str().unwrap_or("");
            if name.ends_with(".jar") {
                let download_url = asset["browser_download_url"]
                    .as_str()
                    .context("Failed to get download URL")?;
                return Ok(download_url.to_string());
            }
        }

        Err(anyhow::anyhow!(
            "No JAR file found in Taiyitist release {}",
            version
        ))
    }

    async fn get_velocity_url(&self, version: &str) -> Result<String> {
        // Papermc API for Velocity
        let base_url = "https://api.papermc.io/v2/projects/velocity";
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;

        // Get latest build for the version
        let builds_url = format!("{}/versions/{}/builds", base_url, version);
        let resp: serde_json::Value = client.get(&builds_url).send().await?.json().await?;

        let builds = resp["builds"].as_array().context("No builds found")?;
        let latest_build = builds.last().context("No builds found")?;
        let build_number = latest_build["build"]
            .as_i64()
            .context("Invalid build number")?;
        let name = latest_build["downloads"]["application"]["name"]
            .as_str()
            .context("Invalid download name")?;

        Ok(format!(
            "{}/versions/{}/builds/{}/downloads/{}",
            base_url, version, build_number, name
        ))
    }

    async fn get_waterfall_url(&self, version: &str) -> Result<String> {
        // Papermc API for Waterfall
        let base_url = "https://api.papermc.io/v2/projects/waterfall";
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;

        // Get latest build for the version
        let builds_url = format!("{}/versions/{}/builds", base_url, version);
        let resp: serde_json::Value = client.get(&builds_url).send().await?.json().await?;

        let builds = resp["builds"].as_array().context("No builds found")?;
        let latest_build = builds.last().context("No builds found")?;
        let build_number = latest_build["build"]
            .as_i64()
            .context("Invalid build number")?;
        let name = latest_build["downloads"]["application"]["name"]
            .as_str()
            .context("Invalid download name")?;

        Ok(format!(
            "{}/versions/{}/builds/{}/downloads/{}",
            base_url, version, build_number, name
        ))
    }

    async fn get_bungeecord_url(&self, _version: &str) -> Result<String> {
        // BungeeCord (Jenkins) - For now just return latest stable
        // The version string might be ignored or used if we support specific builds
        // Official CI: https://ci.md-5.net/job/BungeeCord/
        Ok("https://ci.md-5.net/job/BungeeCord/lastSuccessfulBuild/artifact/bootstrap/target/BungeeCord.jar".to_string())
    }

    async fn get_purpur_url(&self, version: &str) -> Result<String> {
        // Purpur API: https://api.purpurmc.org/v2/purpur/{version}
        let url = format!("https://api.purpurmc.org/v2/purpur/{}", version);
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;
        let resp: serde_json::Value = client.get(&url).send().await?.json().await?;

        let latest_build = resp["builds"]["latest"]
            .as_str()
            .context("No latest build found for Purpur")?;

        Ok(format!(
            "https://api.purpurmc.org/v2/purpur/{}/{}/download",
            version, latest_build
        ))
    }

    async fn get_banner_url(&self, version: &str) -> Result<String> {
        // Banner is available on mohistmc.com builds-raw
        // Filenames use git hashes: Banner-1.20.1-{hash}.jar
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;

        // Get directory listing from builds-raw
        let dir_url = format!("https://mohistmc.com/builds-raw/Banner-{}/", version);
        println!("Fetching Banner builds from: {}", dir_url);

        let resp = client.get(&dir_url).send().await?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "Banner {} のビルドディレクトリにアクセスできません (HTTP {})",
                version,
                resp.status()
            );
        }

        let html = resp.text().await?;

        // Parse HTML directory listing for JAR files
        // Format: href="Banner-1.20.1-{hash}.jar"
        let prefix = format!("Banner-{}-", version);
        let mut latest_jar: Option<String> = None;

        for part in html.split("href=\"") {
            if let Some(end_quote) = part.find('"') {
                let href = &part[..end_quote];
                if href.starts_with(&prefix) && href.ends_with(".jar") {
                    // Keep track of the last JAR found (directory listings are usually sorted)
                    latest_jar = Some(href.to_string());
                }
            }
        }

        let jar_name =
            latest_jar.context(format!("Banner {} のビルドが見つかりません。", version))?;

        let download_url = format!(
            "https://mohistmc.com/builds-raw/Banner-{}/{}",
            version, jar_name
        );
        println!("Banner direct download: {}", download_url);
        Ok(download_url)
    }

    async fn build_spigot(&self, server_path: &Path, version: &str) -> Result<()> {
        // Spigot requires BuildTools to build
        // 1. Download BuildTools.jar
        // 2. Run BuildTools with specified version
        // 3. Copy resulting spigot-*.jar to server.jar

        println!("[Spigot BuildTools] Starting build for version {}", version);

        let buildtools_url = "https://hub.spigotmc.org/jenkins/job/BuildTools/lastSuccessfulBuild/artifact/target/BuildTools.jar";
        let buildtools_path = server_path.join("BuildTools.jar");
        let jar_path = server_path.join("server.jar");

        // Download BuildTools.jar
        println!("[Spigot BuildTools] Downloading BuildTools.jar...");
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;
        let response = client.get(buildtools_url).send().await?;

        if !response.status().is_success() {
            anyhow::bail!(
                "Failed to download BuildTools.jar: HTTP {}",
                response.status()
            );
        }

        let content = response.bytes().await?;
        fs::write(&buildtools_path, content).await?;

        // Get appropriate Java version for building
        let java_cmd = crate::java_detector::select_java_for_minecraft(version)
            .unwrap_or_else(|| "java".to_string());

        println!("[Spigot BuildTools] Using Java: {}", java_cmd);
        println!(
            "[Spigot BuildTools] Building Spigot {}... (this may take a while)",
            version
        );

        // Run BuildTools
        let output = Command::new(&java_cmd)
            .args(&["-jar", "BuildTools.jar", "--rev", version])
            .current_dir(server_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .context("Failed to run BuildTools")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("BuildTools failed: {}", stderr);
        }

        println!("[Spigot BuildTools] Build completed, locating JAR...");

        // Find the built spigot JAR
        let mut found_jar = false;
        if let Ok(entries) = std::fs::read_dir(server_path) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("spigot-") && name.ends_with(".jar") {
                    // Copy to server.jar
                    std::fs::copy(entry.path(), &jar_path)?;
                    found_jar = true;
                    println!("[Spigot BuildTools] Found and copied: {}", name);
                    break;
                }
            }
        }

        if !found_jar {
            anyhow::bail!("BuildTools completed but spigot-*.jar not found");
        }

        // Cleanup BuildTools files (optional, keep for re-builds)
        // let _ = std::fs::remove_file(&buildtools_path);

        println!("[Spigot BuildTools] Spigot server ready!");
        Ok(())
    }

    pub async fn fetch_vanilla_versions(&self) -> Result<Vec<String>> {
        let manifest_url = "https://launchermeta.mojang.com/mc/game/version_manifest.json";
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;
        let manifest: serde_json::Value = client.get(manifest_url).send().await?.json().await?;

        let versions = manifest["versions"]
            .as_array()
            .context("Invalid manifest format")?
            .iter()
            .filter(|v| v["type"].as_str() == Some("release"))
            .filter_map(|v| v["id"].as_str().map(|s| s.to_string()))
            .collect();

        Ok(versions)
    }

    pub async fn fetch_paper_versions(&self) -> Result<Vec<String>> {
        let url = "https://api.papermc.io/v2/projects/paper";
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;
        let resp: serde_json::Value = client.get(url).send().await?.json().await?;

        let mut versions: Vec<String> = resp["versions"]
            .as_array()
            .context("Invalid response format")?
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        // Reverse to show newest first (Paper API returns oldest first usually)
        versions.reverse();

        Ok(versions)
    }

    pub async fn fetch_fabric_versions(&self) -> Result<Vec<String>> {
        let url = "https://meta.fabricmc.net/v2/versions/game";
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;
        let resp: serde_json::Value = client.get(url).send().await?.json().await?;

        let versions: Vec<String> = resp
            .as_array()
            .context("Invalid response format")?
            .iter()
            .filter(|v| v["stable"].as_bool().unwrap_or(false))
            .filter_map(|v| v["version"].as_str().map(|s| s.to_string()))
            .collect();

        Ok(versions)
    }

    pub async fn fetch_mohist_versions(&self) -> Result<Vec<String>> {
        // Fetch versions from new Mohist API
        let url = "https://api.mohistmc.com/project/mohist/versions";
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;

        let resp: serde_json::Value = client.get(url).send().await?.json().await?;

        let mut versions: Vec<String> = resp
            .as_array()
            .context("Invalid response format")?
            .iter()
            .filter_map(|v| v["name"].as_str().map(|s| s.to_string()))
            .collect();

        // Reverse to show newest first
        versions.reverse();

        Ok(versions)
    }

    pub async fn fetch_taiyitist_versions(&self) -> Result<Vec<String>> {
        // Fetch releases from GitHub API
        let url = "https://api.github.com/repos/Teneted/Taiyitist/releases";
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;

        let resp: serde_json::Value = client.get(url).send().await?.json().await?;

        // Tag format is "{version}-release", strip the "-release" suffix for UI display
        let versions: Vec<String> = resp
            .as_array()
            .context("Invalid response format")?
            .iter()
            .filter_map(|v| {
                v["tag_name"]
                    .as_str()
                    .map(|s| s.strip_suffix("-release").unwrap_or(s).to_string())
            })
            .collect();

        Ok(versions)
    }

    pub async fn fetch_velocity_versions(&self) -> Result<Vec<String>> {
        let url = "https://api.papermc.io/v2/projects/velocity";
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;
        let resp: serde_json::Value = client.get(url).send().await?.json().await?;

        let mut versions: Vec<String> = resp["versions"]
            .as_array()
            .context("Invalid response format")?
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        versions.reverse();
        Ok(versions)
    }

    pub async fn fetch_waterfall_versions(&self) -> Result<Vec<String>> {
        let url = "https://api.papermc.io/v2/projects/waterfall";
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;
        let resp: serde_json::Value = client.get(url).send().await?.json().await?;

        let mut versions: Vec<String> = resp["versions"]
            .as_array()
            .context("Invalid response format")?
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        versions.reverse();
        Ok(versions)
    }

    pub async fn fetch_bungeecord_versions(&self) -> Result<Vec<String>> {
        // BungeeCord doesn't have a clean version list API easily accessible like Paper
        // It's usually just "Latest" or build numbers.
        // We'll return a single "latest" version for now.
        Ok(vec!["latest".to_string()])
    }

    pub async fn fetch_purpur_versions(&self) -> Result<Vec<String>> {
        let url = "https://api.purpurmc.org/v2/purpur";
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;
        let resp: serde_json::Value = client.get(url).send().await?.json().await?;

        let mut versions: Vec<String> = resp["versions"]
            .as_array()
            .context("Invalid Purpur response format")?
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        versions.reverse();
        Ok(versions)
    }

    pub async fn fetch_banner_versions(&self) -> Result<Vec<String>> {
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;

        // Get Banner versions from builds-raw directory listing
        let url = "https://mohistmc.com/builds-raw/";
        let resp = client.get(url).send().await?;
        let html = resp.text().await?;

        // Parse directory listing for Banner-X.Y.Z folders
        let mut versions: Vec<String> = Vec::new();
        for part in html.split("href=\"Banner-") {
            if let Some(end) = part.find('/') {
                let ver = &part[..end];
                if !ver.is_empty()
                    && ver
                        .chars()
                        .next()
                        .map(|c| c.is_ascii_digit())
                        .unwrap_or(false)
                {
                    versions.push(ver.to_string());
                }
            }
        }

        // Remove duplicates
        versions.sort();
        versions.dedup();

        // Sort by version (newest first)
        versions.sort_by(|a, b| {
            let a_parts: Vec<u32> = a.split('.').filter_map(|s| s.parse().ok()).collect();
            let b_parts: Vec<u32> = b.split('.').filter_map(|s| s.parse().ok()).collect();
            b_parts.cmp(&a_parts)
        });

        Ok(versions)
    }

    pub async fn fetch_spigot_versions(&self) -> Result<Vec<String>> {
        // Spigot versions typically mirror vanilla releases
        // But only certain versions are supported by BuildTools
        // We'll use vanilla versions for now, BuildTools will inform if unsupported
        self.fetch_vanilla_versions().await
    }

    async fn create_default_properties(&self, server_path: &Path, port: u16) -> Result<()> {
        let properties = format!(
            "server-port={}\n\
             enable-command-block=true\n\
             gamemode=survival\n\
             difficulty=normal\n\
             max-players=20\n\
             view-distance=10\n\
             motd=A Minecraft Server managed by Prismarine\n",
            port
        );

        fs::write(server_path.join("server.properties"), properties).await?;
        Ok(())
    }

    pub async fn set_server_motd(&self, server_id: &str, motd: &str) -> Result<()> {
        let server = self
            .servers
            .lock()
            .await
            .get(server_id)
            .context("Server not found")?
            .clone();

        let props_path = server.path.join("server.properties");
        if !props_path.exists() {
            // If missing, create default? Or error? Error is safer but we initialized it.
            // Just let it error or return.
            return Ok(());
        }

        let content = fs::read_to_string(&props_path).await?;
        let mut new_lines = Vec::new();
        let mut found = false;

        for line in content.lines() {
            if line.trim().starts_with("motd=") {
                new_lines.push(format!("motd={}", motd));
                found = true;
            } else {
                new_lines.push(line.to_string());
            }
        }

        if !found {
            new_lines.push(format!("motd={}", motd));
        }

        fs::write(&props_path, new_lines.join("\n")).await?;
        Ok(())
    }

    pub async fn get_server_motd(&self, server_id: &str) -> Result<String> {
        let server = self
            .servers
            .lock()
            .await
            .get(server_id)
            .context("Server not found")?
            .clone();

        let props_path = server.path.join("server.properties");
        if !props_path.exists() {
            return Ok("".to_string());
        }

        let content = fs::read_to_string(&props_path).await?;
        for line in content.lines() {
            if let Some(val) = line.trim().strip_prefix("motd=") {
                return Ok(val.to_string());
            }
        }
        Ok("".to_string())
    }

    pub async fn set_server_max_players(&self, server_id: &str, max_players: u32) -> Result<()> {
        let server = self
            .servers
            .lock()
            .await
            .get(server_id)
            .context("Server not found")?
            .clone();

        let props_path = server.path.join("server.properties");
        if !props_path.exists() {
            return Ok(());
        }

        let content = fs::read_to_string(&props_path).await?;
        let mut new_lines = Vec::new();
        let mut found = false;

        for line in content.lines() {
            if line.trim().starts_with("max-players=") {
                new_lines.push(format!("max-players={}", max_players));
                found = true;
            } else {
                new_lines.push(line.to_string());
            }
        }

        if !found {
            new_lines.push(format!("max-players={}", max_players));
        }

        fs::write(&props_path, new_lines.join("\n")).await?;
        Ok(())
    }

    pub async fn get_server_max_players(&self, server_id: &str) -> Result<u32> {
        let server = self
            .servers
            .lock()
            .await
            .get(server_id)
            .context("Server not found")?
            .clone();

        let props_path = server.path.join("server.properties");
        if !props_path.exists() {
            return Ok(20);
        }

        let content = fs::read_to_string(&props_path).await?;
        for line in content.lines() {
            if let Some(val) = line.trim().strip_prefix("max-players=") {
                return Ok(val.parse().unwrap_or(20));
            }
        }
        Ok(20)
    }

    pub async fn install_geyser(&self, server_id: &str) -> Result<()> {
        let server = self
            .servers
            .lock()
            .await
            .get(server_id)
            .context("Server not found")?
            .clone();

        match server.server_type {
            ServerType::Vanilla | ServerType::Fabric | ServerType::Mohist => {
                anyhow::bail!("このサーバータイプはBukkit/Spigotプラグインに対応していません。PaperまたはSpigotを使用してください。")
            }
            _ => {}
        }

        let plugins_path = server.path.join("plugins");
        fs::create_dir_all(&plugins_path).await?;

        // Geyser for Spigot/Paper
        self.install_plugin(
            &plugins_path,
            "https://download.geysermc.org/v2/projects/geyser/versions/latest/builds/latest/downloads/spigot",
            "Geyser-Spigot.jar"
        ).await.context("Failed to install Geyser")?;

        // Floodgate for Spigot/Paper
        self.install_plugin(
            &plugins_path,
            "https://download.geysermc.org/v2/projects/floodgate/versions/latest/builds/latest/downloads/spigot",
            "floodgate-spigot.jar"
        ).await.context("Failed to install Floodgate")?;

        // Disable enforce-secure-profile in server.properties
        self.update_server_property(&server.path, "enforce-secure-profile", "false")
            .await?;

        // "True" AutoGeyser: Install AutoUpdateGeyser plugin to keep them updated
        // Slug: autoupdategeyser (NewAmazingPVP)
        println!("Installing AutoUpdateGeyser...");
        if let Err(e) = self
            .install_modrinth_plugin(server_id, "autoupdategeyser", "AutoUpdateGeyser")
            .await
        {
            println!("Failed to install AutoUpdateGeyser: {}", e);
            // Don't fail the whole process, manual update is better than nothing
        }

        Ok(())
    }

    pub async fn install_viaversion(&self, server_id: &str) -> Result<()> {
        let server = self
            .servers
            .lock()
            .await
            .get(server_id)
            .context("Server not found")?
            .clone();

        match server.server_type {
            ServerType::Vanilla => {
                anyhow::bail!("Vanilla servers do not support plugins. Please use Paper or Spigot.")
            }
            _ => {}
        }

        let plugins_path = server.path.join("plugins");
        fs::create_dir_all(&plugins_path).await?;

        // Fetch latest ViaVersion from Hangar API
        let api_url =
            "https://hangar.papermc.io/api/v1/projects/ViaVersion/versions?limit=1&platform=PAPER";
        println!("Fetching ViaVersion info from: {}", api_url);

        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;

        let resp: serde_json::Value = client.get(api_url).send().await?.json().await?;

        let results = resp["result"]
            .as_array()
            .context("Invalid Hangar API response")?;

        let latest_version = results.first().context("No ViaVersion versions found")?;

        let download_url = latest_version["downloads"]["PAPER"]["downloadUrl"]
            .as_str()
            .context("Download URL not found in Hangar response")?;

        println!("Found ViaVersion download URL: {}", download_url);

        self.install_plugin(&plugins_path, download_url, "ViaVersion.jar")
            .await
            .context("Failed to install ViaVersion")?;

        Ok(())
    }

    async fn install_plugin(&self, plugins_path: &Path, url: &str, filename: &str) -> Result<()> {
        println!("Downloading plugin: {} from {}", filename, url);

        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;

        let response = client.get(url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Failed to download plugin {}: Status {}",
                filename,
                response.status()
            ));
        }

        let content = response.bytes().await?;
        fs::write(plugins_path.join(filename), content).await?;
        Ok(())
    }

    pub async fn uninstall_geyser(&self, server_id: &str) -> Result<()> {
        let server = self
            .servers
            .lock()
            .await
            .get(server_id)
            .context("Server not found")?
            .clone();
        let plugins_path = server.path.join("plugins");

        // Remove Geyser-Spigot.jar
        let jar_path = plugins_path.join("Geyser-Spigot.jar");
        if jar_path.exists() {
            fs::remove_file(jar_path).await?;
        }

        // Remove floodgate-spigot.jar
        let floodgate_path = plugins_path.join("floodgate-spigot.jar");
        if floodgate_path.exists() {
            fs::remove_file(floodgate_path).await?;
        }

        // Restore enforce-secure-profile in server.properties
        self.update_server_property(&server.path, "enforce-secure-profile", "true")
            .await?;

        Ok(())
    }

    pub async fn uninstall_viaversion(&self, server_id: &str) -> Result<()> {
        let server = self
            .servers
            .lock()
            .await
            .get(server_id)
            .context("Server not found")?
            .clone();
        let plugins_path = server.path.join("plugins");

        // Remove ViaVersion.jar
        let jar_path = plugins_path.join("ViaVersion.jar");
        if jar_path.exists() {
            fs::remove_file(jar_path).await?;
        }

        Ok(())
    }

    async fn update_server_property(
        &self,
        server_path: &Path,
        key: &str,
        value: &str,
    ) -> Result<()> {
        let props_path = server_path.join("server.properties");

        // Read existing content or start empty
        let content = if props_path.exists() {
            fs::read_to_string(&props_path).await?
        } else {
            String::new()
        };

        let mut new_lines = Vec::new();
        let mut found = false;

        for line in content.lines() {
            let mut matched = false;
            // Ignore comments for keys
            if !line.trim().starts_with('#') {
                if let Some((k, _)) = line.split_once('=') {
                    if k.trim() == key {
                        new_lines.push(format!("{}={}", key, value));
                        matched = true;
                        found = true;
                    }
                }
            }

            if !matched {
                new_lines.push(line.to_string());
            }
        }

        if !found {
            new_lines.push(format!("{}={}", key, value));
        }

        if !found {
            new_lines.push(format!("{}={}", key, value));
        }

        fs::write(props_path, new_lines.join("\n")).await?;
        Ok(())
    }

    pub async fn check_geyser_installed(&self, server_id: &str) -> Result<bool> {
        let server = self
            .servers
            .lock()
            .await
            .get(server_id)
            .context("Server not found")?
            .clone();
        let plugins_path = server.path.join("plugins");

        let geyser_exists = plugins_path.join("Geyser-Spigot.jar").exists();
        let floodgate_exists = plugins_path.join("floodgate-spigot.jar").exists();

        println!(
            "[Check] Server: {}, Geyser: {}, Floodgate: {}",
            server_id, geyser_exists, floodgate_exists
        );

        // Check server.properties for enforce-secure-profile=false
        let props_path = server.path.join("server.properties");
        let mut secure_profile_bg_check = false;

        if props_path.exists() {
            let content = fs::read_to_string(&props_path).await?;
            for line in content.lines() {
                let trimmed = line.trim();
                // Ignore comments
                if trimmed.starts_with('#') {
                    continue;
                }

                if let Some((k, v)) = trimmed.split_once('=') {
                    if k.trim() == "enforce-secure-profile" {
                        println!("[Check] Found enforce-secure-profile value: '{}'", v.trim());
                        if v.trim() == "false" {
                            secure_profile_bg_check = true;
                        }
                        break;
                    }
                }
            }
        } else {
            println!("[Check] server.properties not found at {:?}", props_path);
            secure_profile_bg_check = false;
        }

        println!(
            "[Check] Secure Profile Disabled: {}",
            secure_profile_bg_check
        );

        // Treat as installed only if ALL conditions match.
        Ok(geyser_exists && floodgate_exists && secure_profile_bg_check)
    }

    pub async fn check_viaversion_installed(&self, server_id: &str) -> Result<bool> {
        let server = self
            .servers
            .lock()
            .await
            .get(server_id)
            .context("Server not found")?
            .clone();
        let plugins_path = server.path.join("plugins");

        Ok(plugins_path.join("ViaVersion.jar").exists())
    }

    pub async fn search_plugins(
        &self,
        server_id: &str,
        query: &str,
        source: &str,
    ) -> Result<Vec<PluginSearchResult>> {
        let (version, server_type) = {
            let servers = self.servers.lock().await;
            let server = servers.get(server_id).context("Server not found")?;
            (server.version.clone(), server.server_type.clone())
        };

        match source {
            "Modrinth" => self.search_modrinth(query, &version, &server_type).await,
            "Spigot" => self.search_spigot(query).await,
            _ => Err(anyhow::anyhow!("Unknown source: {}", source)),
        }
    }

    pub async fn install_modrinth_plugin(
        &self,
        server_id: &str,
        project_id: &str,
        plugin_name: &str,
    ) -> Result<()> {
        let (version, server_type) = {
            let servers = self.servers.lock().await;
            let server = servers.get(server_id).context("Server not found")?;
            (server.version.clone(), server.server_type.clone())
        };

        // Map ServerType to Modrinth loaders
        // Paper keys can include "paper", "spigot", "bukkit"
        // Spigot keys: "spigot", "bukkit"
        // Vanilla: usually doesn't have plugins, but maybe "datapack"? Assuming plugin for now.
        let loaders = match server_type {
            ServerType::Paper | ServerType::Purpur => "[\"bukkit\", \"paper\", \"spigot\"]",
            ServerType::Spigot => "[\"bukkit\", \"spigot\"]",
            ServerType::Forge => "[\"forge\"]",
            ServerType::Vanilla => "[\"bukkit\"]", // Fallback
            ServerType::Fabric
            | ServerType::Mohist
            | ServerType::Taiyitist
            | ServerType::Banner => "[]", // No plugin support or different system
            ServerType::Velocity => "[\"velocity\"]",
            ServerType::BungeeCord => "[\"bungeecord\"]",
            ServerType::Waterfall => "[\"bungeecord\",\"waterfall\"]",
        };

        let game_versions = format!("[\"{}\"]", version);

        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0 (antigravity)")
            .build()?;

        // Fetch versions filtered by loader and game version
        let url = format!(
            "https://api.modrinth.com/v2/project/{}/version?loaders={}&game_versions={}",
            project_id, loaders, game_versions
        );

        let resp = client.get(&url).send().await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            println!("Modrinth API Error: {} - Body: {}", status, text);
            anyhow::bail!("Modrinth API failed with status {}: {}", status, text);
        }

        let resp_text = resp.text().await?;
        let versions: serde_json::Value =
            serde_json::from_str(&resp_text).context("Failed to parse Modrinth JSON")?;

        let versions = versions
            .as_array()
            .context("Invalid Modrinth version response")?;

        if versions.is_empty() {
            anyhow::bail!(
                "No compatible version found for Minecraft {} ({:?})",
                version,
                server_type
            );
        }

        // Pick the first one (latest compatible)
        let latest = &versions[0];
        let files = latest["files"]
            .as_array()
            .context("No files found in version")?;

        // Find the primary file or first .jar
        let file = files
            .iter()
            .find(|f| {
                f["primary"].as_bool().unwrap_or(false)
                    || f["filename"].as_str().unwrap_or("").ends_with(".jar")
            })
            .or(files.first())
            .context("No suitable file found")?;

        let download_url = file["url"].as_str().context("No download URL")?.to_string();

        // Sanitize plugin name for filename (remove invalid characters)
        let safe_name: String = plugin_name
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == ' ')
            .collect();
        let filename = format!("{}.jar", safe_name.trim());

        self.install_plugin_by_url(server_id, &download_url, Some(filename))
            .await?;
        Ok(())
    }

    async fn search_modrinth(
        &self,
        query: &str,
        version: &str,
        server_type: &ServerType,
    ) -> Result<Vec<PluginSearchResult>> {
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0 (antigravity)")
            .build()?;

        // Map ServerType to Modrinth categories (loaders)
        let loaders_facet = match server_type {
            ServerType::Paper | ServerType::Purpur => {
                "[\"categories:paper\",\"categories:spigot\",\"categories:bukkit\"]"
            }
            ServerType::Spigot => "[\"categories:spigot\",\"categories:bukkit\"]",
            ServerType::Forge => "[\"categories:forge\"]",
            ServerType::Vanilla => "[\"categories:bukkit\"]", // Weak fallback
            ServerType::Fabric | ServerType::Banner => "[\"categories:fabric\"]",
            ServerType::Mohist => "[\"categories:forge\"]", // Mohist runs Forge mods
            ServerType::Taiyitist => "[\"categories:forge\"]", // Taiyitist runs Forge mods
            ServerType::Velocity => "[\"categories:velocity\"]",
            ServerType::BungeeCord => "[\"categories:bungeecord\"]",
            ServerType::Waterfall => "[\"categories:bungeecord\",\"categories:waterfall\"]",
        };

        let version_facet = format!("[\"versions:{}\"]", version);

        let sort_param = if query.is_empty() {
            "&sort=follows" // Better "Trending/Popular" indicator than total downloads
        } else {
            ""
        };

        let project_type_facet = match server_type {
            ServerType::Fabric | ServerType::Forge | ServerType::Mohist | ServerType::Taiyitist => {
                "[\"project_type:mod\"]"
            }
            _ => "[\"project_type:plugin\"]",
        };

        // Facets: ProjectType AND Version AND Loaders
        let facets = format!(
            "[{},{},{}]",
            project_type_facet, version_facet, loaders_facet
        );

        let url = format!(
            "https://api.modrinth.com/v2/search?query={}&facets={}&limit=20{}",
            query, facets, sort_param
        );

        let resp: serde_json::Value = client.get(&url).send().await?.json().await?;
        let hits = resp["hits"]
            .as_array()
            .context("Invalid Modrinth response")?;

        let mut results = Vec::new();
        for hit in hits {
            let id = hit["project_id"].as_str().unwrap_or("").to_string();
            let name = hit["title"].as_str().unwrap_or("").to_string();
            let description = hit["description"].as_str().unwrap_or("").to_string();
            let author = hit["author"].as_str().unwrap_or("").to_string();
            let icon_url = hit["icon_url"].as_str().map(|s| s.to_string());
            let slug = hit["slug"].as_str().unwrap_or("");
            let external_url = format!("https://modrinth.com/plugin/{}", slug);

            results.push(PluginSearchResult {
                id,
                name,
                description,
                author,
                icon_url,
                source: "Modrinth".to_string(),
                external_url,
                download_url: None, // Modrinth needs version fetch
            });
        }
        Ok(results)
    }

    async fn search_spigot(&self, query: &str) -> Result<Vec<PluginSearchResult>> {
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;

        let url = if query.is_empty() {
            "https://api.spiget.org/v2/resources?limit=20&sort=-downloads".to_string()
        } else {
            format!(
                "https://api.spiget.org/v2/search/resources/{}?limit=20&sort=-downloads",
                query
            )
        };

        // Spiget returns array directly or inside content? Usually array.
        let resp: serde_json::Value = client.get(&url).send().await?.json().await?;

        let mut results = Vec::new();
        // Spiget behavior: if no results, might return empty array.
        if let Some(items) = resp.as_array() {
            for item in items {
                let id = item["id"]
                    .as_i64()
                    .map(|i| i.to_string())
                    .unwrap_or_default();
                let name = item["name"].as_str().unwrap_or("").to_string();
                let tag = item["tag"].as_str().unwrap_or("").to_string(); // Short desc
                let author_id = item["author"]["id"].as_i64().unwrap_or(0);

                // Icon handling in Spiget is weird, usually https://www.spigotmc.org/data/resource_icons/<id_prefix>/<id>.jpg
                // But we can skip or try to construct.
                let icon_url = if !item["icon"]["data"].as_str().unwrap_or("").is_empty() {
                    Some(format!(
                        "https://www.spigotmc.org/data/resource_icons/{}/{}.jpg",
                        id.parse::<i64>().unwrap_or(0) / 1000,
                        id
                    ))
                } else {
                    None
                };

                let external_url = format!("https://www.spigotmc.org/resources/{}", id);

                results.push(PluginSearchResult {
                    id: id.clone(),
                    name,
                    description: tag,
                    author: format!("User {}", author_id), // Fetching author name requires extra call, skip for now
                    icon_url,
                    source: "Spigot".to_string(),
                    external_url,
                    download_url: Some(format!(
                        "https://api.spiget.org/v2/resources/{}/download",
                        id
                    )),
                });
            }
        }
        Ok(results)
    }

    pub async fn install_plugin_by_url(
        &self,
        server_id: &str,
        download_url: &str,
        filename: Option<String>,
    ) -> Result<()> {
        let plugins_path = self.get_plugins_path(server_id).await?;

        let fname = if let Some(n) = filename {
            n
        } else {
            // Try to guess from URL or Content-Disposition?
            // Simple fallback: "plugin.jar" or derive from end of URL.
            // Spiget download urls don't have filename.
            // Modrinth version urls might.
            "unknown_plugin.jar".to_string()
        };

        self.install_plugin(&plugins_path, download_url, &fname)
            .await?;
        Ok(())
    }

    pub async fn install_spigot_plugin(
        &self,
        server_id: &str,
        resource_id: &str,
        plugin_name: &str,
    ) -> Result<()> {
        let download_url = format!(
            "https://api.spiget.org/v2/resources/{}/download",
            resource_id
        );
        // Sanitize plugin name for filename (remove invalid characters)
        let safe_name: String = plugin_name
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == ' ')
            .collect();
        let filename = format!("{}.jar", safe_name.trim());
        self.install_plugin_by_url(server_id, &download_url, Some(filename))
            .await
    }

    pub async fn set_server_memory(
        &self,
        server_id: &str,
        max_memory: &str,
        min_memory: &str,
    ) -> Result<()> {
        let mut servers = self.servers.lock().await;
        if let Some(server) = servers.get_mut(server_id) {
            server.max_memory = max_memory.to_string();
            server.min_memory = min_memory.to_string();
            Ok(())
        } else {
            anyhow::bail!("Server not found")
        }
    }

    pub async fn is_plugin_installed(&self, server_id: &str, plugin_name: &str) -> Result<bool> {
        let plugins_path = self.get_plugins_path(server_id).await?;

        // Sanitize plugin name for filename (same logic as install)
        let safe_name: String = plugin_name
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == ' ')
            .collect();
        let filename = format!("{}.jar", safe_name.trim());

        Ok(plugins_path.join(filename).exists())
    }

    pub async fn uninstall_plugin(&self, server_id: &str, plugin_name: &str) -> Result<()> {
        let plugins_path = self.get_plugins_path(server_id).await?;

        // Sanitize plugin name for filename (same logic as install)
        let safe_name: String = plugin_name
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == ' ')
            .collect();
        let filename = format!("{}.jar", safe_name.trim());

        let file_path = plugins_path.join(filename);
        if file_path.exists() {
            fs::remove_file(file_path).await?;
        }

        Ok(())
    }

    pub async fn check_and_restart_servers(&self) {
        let servers_to_restart = {
            let mut servers = self.servers.lock().await;
            let mut restart_ids = Vec::new();
            let now_params = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs();

            for (id, server) in servers.iter_mut() {
                if !server.auto_restart || server.status != ServerStatus::Running {
                    continue;
                }

                match server.restart_type {
                    RestartType::Interval => {
                        if let Some(last_start) = server.last_start_time {
                            // Restart interval must be at least 60 seconds to prevent loops
                            let interval = std::cmp::max(server.restart_interval, 60);
                            if now_params >= last_start + interval {
                                println!("Interval Trigger: Restarting server {}", server.name);
                                restart_ids.push(id.clone());
                            }
                        }
                    }
                    RestartType::Schedule => {
                        if let (Some(schedule), Some(tz_str)) =
                            (&server.restart_schedule, &server.time_zone)
                        {
                            if let Ok(tz) = tz_str.parse::<chrono_tz::Tz>() {
                                use chrono::Timelike;
                                let now = chrono::Utc::now().with_timezone(&tz);

                                if let Ok(target_time) =
                                    chrono::NaiveTime::parse_from_str(schedule, "%H:%M")
                                {
                                    // Check if current time matches target time (minute precision)
                                    if now.hour() == target_time.hour()
                                        && now.minute() == target_time.minute()
                                    {
                                        // Prevent double restart: check if last_start_time was recently (e.g. < 5 mins ago)
                                        if let Some(last_start) = server.last_start_time {
                                            if now_params < last_start + 300 {
                                                continue;
                                            }
                                        }

                                        println!(
                                            "Schedule Trigger: Restarting server {}",
                                            server.name
                                        );
                                        restart_ids.push(id.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
            restart_ids
        };

        for id in servers_to_restart {
            let _ = self.restart_server(&id).await;
        }
    }

    pub async fn restart_server(&self, server_id: &str) -> Result<()> {
        let status = {
            let servers = self.servers.lock().await;
            if let Some(server) = servers.get(server_id) {
                server.status.clone()
            } else {
                anyhow::bail!("Server not found");
            }
        };

        // Only stop if running or starting
        if status == ServerStatus::Running || status == ServerStatus::Starting {
            self.stop_server(server_id).await?;
            // No fixed delay needed - stop_server now properly waits for process exit
        }

        // Start the server
        self.start_server(server_id).await
    }

    pub async fn get_proxy_registered_servers(
        &self,
        proxy_id: &str,
    ) -> Result<Vec<ProxyServerEntry>> {
        let server = self
            .get_server(proxy_id)
            .await
            .context("Server not found")?;

        match server.server_type {
            ServerType::Velocity => {
                let config_path = server.path.join("velocity.toml");
                if !config_path.exists() {
                    return Ok(vec![]);
                }
                let content = fs::read_to_string(&config_path).await?;
                let config: toml::Value =
                    toml::from_str(&content).context("Failed to parse velocity.toml")?;

                let mut entries = Vec::new();
                if let Some(servers) = config.get("servers").and_then(|v| v.as_table()) {
                    for (name, addr) in servers {
                        if let Some(addr_str) = addr.as_str() {
                            entries.push(ProxyServerEntry {
                                name: name.clone(),
                                address: addr_str.to_string(),
                            });
                        }
                    }
                }
                Ok(entries)
            }
            ServerType::BungeeCord | ServerType::Waterfall => {
                let config_path = server.path.join("config.yml");
                if !config_path.exists() {
                    return Ok(vec![]);
                }
                let content = fs::read_to_string(&config_path).await?;
                let config: serde_yaml::Value =
                    serde_yaml::from_str(&content).context("Failed to parse config.yml")?;

                let mut entries = Vec::new();
                if let Some(servers) = config.get("servers").and_then(|v| v.as_mapping()) {
                    for (name, info) in servers {
                        let name_str = name.as_str().unwrap_or("").to_string();
                        let addr = info
                            .get("address")
                            .and_then(|a| a.as_str())
                            .unwrap_or("")
                            .to_string();
                        if !name_str.is_empty() {
                            entries.push(ProxyServerEntry {
                                name: name_str,
                                address: addr,
                            });
                        }
                    }
                }
                Ok(entries)
            }
            _ => Err(anyhow::anyhow!("Not a proxy server")),
        }
    }

    pub async fn add_server_to_proxy(
        &self,
        proxy_id: &str,
        name: &str,
        address: &str,
        add_to_try: bool,
    ) -> Result<()> {
        let server = self
            .get_server(proxy_id)
            .await
            .context("Server not found")?;
        match server.server_type {
            ServerType::Velocity => {
                let config_path = server.path.join("velocity.toml");

                // If config doesn't exist, create a minimal default
                let content = if config_path.exists() {
                    fs::read_to_string(&config_path).await?
                } else {
                    // Create proper velocity.toml with modern forwarding
                    let default_config = format!(
                        r#"# Velocity Configuration - Auto-generated
online-mode = true
player-info-forwarding-mode = "modern"
forwarding-secret-file = "forwarding.secret"

[servers]
{} = "{}"
try = ["{}"]

[forced-hosts]

[advanced]
"#,
                        name, address, name
                    );

                    // Also create forwarding.secret if it doesn't exist
                    let secret_path = server.path.join("forwarding.secret");
                    if !secret_path.exists() {
                        let secret =
                            format!("{:x}{:x}", rand::random::<u64>(), rand::random::<u64>());
                        fs::write(&secret_path, &secret).await?;
                    }

                    fs::write(&config_path, &default_config).await?;
                    return Ok(());
                };

                let mut config: toml::Value = toml::from_str(&content)?;

                // Ensure modern forwarding is enabled
                if let Some(table) = config.as_table_mut() {
                    table
                        .entry("player-info-forwarding-mode".to_string())
                        .or_insert(toml::Value::String("modern".to_string()));
                    table
                        .entry("online-mode".to_string())
                        .or_insert(toml::Value::Boolean(true));

                    // Also ensure forwarding.secret exists
                    let secret_path = server.path.join("forwarding.secret");
                    if !secret_path.exists() {
                        let secret =
                            format!("{:x}{:x}", rand::random::<u64>(), rand::random::<u64>());
                        fs::write(&secret_path, &secret).await?;
                    }
                }

                if let Some(servers) = config.get_mut("servers").and_then(|v| v.as_table_mut()) {
                    servers.insert(name.to_string(), toml::Value::String(address.to_string()));

                    // Only add to try array if add_to_try is true (direct connection)
                    if add_to_try {
                        if let Some(try_arr) = servers.get_mut("try").and_then(|v| v.as_array_mut())
                        {
                            let name_val = toml::Value::String(name.to_string());
                            if !try_arr.contains(&name_val) {
                                try_arr.push(name_val);
                            }
                        } else {
                            // Create try array with this server
                            servers.insert(
                                "try".to_string(),
                                toml::Value::Array(vec![toml::Value::String(name.to_string())]),
                            );
                        }
                    }
                } else {
                    // Create servers table if missing
                    let mut servers_table = toml::value::Table::new();
                    servers_table
                        .insert(name.to_string(), toml::Value::String(address.to_string()));
                    servers_table.insert(
                        "try".to_string(),
                        toml::Value::Array(vec![toml::Value::String(name.to_string())]),
                    );
                    if let Some(table) = config.as_table_mut() {
                        table.insert("servers".to_string(), toml::Value::Table(servers_table));
                    }
                }

                let new_content = toml::to_string(&config)?;
                fs::write(config_path, new_content).await?;
                Ok(())
            }
            ServerType::BungeeCord | ServerType::Waterfall => {
                let config_path = server.path.join("config.yml");

                // If config doesn't exist, create a minimal default
                let content = if config_path.exists() {
                    fs::read_to_string(&config_path).await?
                } else {
                    // Create minimal config.yml with servers section
                    // Use quotes around server name to ensure it's treated as string
                    let default_config = format!(
                        r#"servers:
  "{}":
    address: "{}"
    restricted: false
    motd: "A Minecraft Server"
listeners:
  - query_port: 25577
    motd: "A Minecraft Proxy"
    priorities:
      - "{}"
    max_players: 100
    force_default_server: false
    host: 0.0.0.0:25565
    query_enabled: false
"#,
                        name, address, name
                    );
                    fs::write(&config_path, &default_config).await?;
                    return Ok(());
                };

                let mut config: serde_yaml::Value = serde_yaml::from_str(&content)?;

                if let Some(servers) = config.get_mut("servers").and_then(|v| v.as_mapping_mut()) {
                    let mut server_info = serde_yaml::Mapping::new();
                    server_info.insert(
                        serde_yaml::Value::String("address".to_string()),
                        serde_yaml::Value::String(address.to_string()),
                    );
                    server_info.insert(
                        serde_yaml::Value::String("restricted".to_string()),
                        serde_yaml::Value::Bool(false),
                    );
                    server_info.insert(
                        serde_yaml::Value::String("motd".to_string()),
                        serde_yaml::Value::String(format!("Just another {} Server", name)),
                    );

                    servers.insert(
                        serde_yaml::Value::String(name.to_string()),
                        serde_yaml::Value::Mapping(server_info),
                    );

                    // Add to priorities if add_to_try is true (direct connection)
                    if add_to_try {
                        if let Some(listeners) = config
                            .get_mut("listeners")
                            .and_then(|v| v.as_sequence_mut())
                        {
                            if let Some(first_listener) =
                                listeners.get_mut(0).and_then(|v| v.as_mapping_mut())
                            {
                                if let Some(priorities) = first_listener
                                    .get_mut(&serde_yaml::Value::String("priorities".to_string()))
                                    .and_then(|v| v.as_sequence_mut())
                                {
                                    let name_val = serde_yaml::Value::String(name.to_string());
                                    if !priorities.contains(&name_val) {
                                        priorities.push(name_val);
                                    }
                                } else {
                                    // Create priorities array
                                    first_listener.insert(
                                        serde_yaml::Value::String("priorities".to_string()),
                                        serde_yaml::Value::Sequence(vec![
                                            serde_yaml::Value::String(name.to_string()),
                                        ]),
                                    );
                                }
                            }
                        }
                    }
                } else {
                    // Create servers section if missing
                    let mut servers_map = serde_yaml::Mapping::new();
                    let mut server_info = serde_yaml::Mapping::new();
                    server_info.insert(
                        serde_yaml::Value::String("address".to_string()),
                        serde_yaml::Value::String(address.to_string()),
                    );
                    server_info.insert(
                        serde_yaml::Value::String("restricted".to_string()),
                        serde_yaml::Value::Bool(false),
                    );
                    servers_map.insert(
                        serde_yaml::Value::String(name.to_string()),
                        serde_yaml::Value::Mapping(server_info),
                    );
                    if let Some(map) = config.as_mapping_mut() {
                        map.insert(
                            serde_yaml::Value::String("servers".to_string()),
                            serde_yaml::Value::Mapping(servers_map),
                        );
                    }
                }

                let new_content = serde_yaml::to_string(&config)?;
                fs::write(config_path, new_content).await?;
                Ok(())
            }
            _ => Err(anyhow::anyhow!("Not a proxy server")),
        }
    }

    pub async fn remove_server_from_proxy(&self, proxy_id: &str, name: &str) -> Result<()> {
        let server = self
            .get_server(proxy_id)
            .await
            .context("Server not found")?;
        match server.server_type {
            ServerType::Velocity => {
                let config_path = server.path.join("velocity.toml");
                let content = fs::read_to_string(&config_path).await?;
                let mut config: toml::Value = toml::from_str(&content)?;

                if let Some(servers) = config.get_mut("servers").and_then(|v| v.as_table_mut()) {
                    servers.remove(name);
                }

                let new_content = toml::to_string(&config)?;
                fs::write(config_path, new_content).await?;
                Ok(())
            }
            ServerType::BungeeCord | ServerType::Waterfall => {
                let config_path = server.path.join("config.yml");
                let content = fs::read_to_string(&config_path).await?;
                let mut config: serde_yaml::Value = serde_yaml::from_str(&content)?;

                if let Some(servers) = config.get_mut("servers").and_then(|v| v.as_mapping_mut()) {
                    servers.remove(&serde_yaml::Value::String(name.to_string()));
                }

                let new_content = serde_yaml::to_string(&config)?;
                fs::write(config_path, new_content).await?;
                Ok(())
            }
            _ => Err(anyhow::anyhow!("Not a proxy server")),
        }
    }

    /// Configure a backend server for use with a proxy (sets online-mode=false, server-ip=127.0.0.1)
    pub async fn configure_backend_for_proxy(
        &self,
        backend_id: &str,
        proxy_id: &str,
    ) -> Result<()> {
        let backend = self
            .get_server(backend_id)
            .await
            .context("Backend server not found")?;
        let proxy = self
            .get_server(proxy_id)
            .await
            .context("Proxy server not found")?;

        // Update server.properties
        let props_path = backend.path.join("server.properties");
        if props_path.exists() {
            let content = fs::read_to_string(&props_path).await?;
            let mut new_lines: Vec<String> = Vec::new();
            let mut has_online_mode = false;
            let mut has_server_ip = false;

            for line in content.lines() {
                if line.starts_with("online-mode=") {
                    new_lines.push("online-mode=false".to_string());
                    has_online_mode = true;
                } else if line.starts_with("server-ip=") {
                    new_lines.push("server-ip=127.0.0.1".to_string());
                    has_server_ip = true;
                } else {
                    new_lines.push(line.to_string());
                }
            }

            if !has_online_mode {
                new_lines.push("online-mode=false".to_string());
            }
            if !has_server_ip {
                new_lines.push("server-ip=127.0.0.1".to_string());
            }

            fs::write(&props_path, new_lines.join("\n")).await?;
        }

        // For Paper servers, configure velocity forwarding
        if matches!(backend.server_type, ServerType::Paper) {
            // Read the forwarding secret from proxy
            let secret_path = proxy.path.join("forwarding.secret");
            let secret = if secret_path.exists() {
                fs::read_to_string(&secret_path)
                    .await
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            } else {
                // Generate a new secret if it doesn't exist
                let new_secret = format!("{:x}", rand::random::<u64>());
                fs::write(&secret_path, &new_secret).await?;
                new_secret
            };

            // Update paper-global.yml if it exists
            let paper_config_path = backend.path.join("config").join("paper-global.yml");
            if paper_config_path.exists() {
                let content = fs::read_to_string(&paper_config_path).await?;
                let mut config: serde_yaml::Value = serde_yaml::from_str(&content)?;

                if let Some(proxies) = config.get_mut("proxies") {
                    if let Some(velocity) = proxies.get_mut("velocity") {
                        if let Some(mapping) = velocity.as_mapping_mut() {
                            mapping.insert(
                                serde_yaml::Value::String("enabled".to_string()),
                                serde_yaml::Value::Bool(true),
                            );
                            mapping.insert(
                                serde_yaml::Value::String("online-mode".to_string()),
                                serde_yaml::Value::Bool(true),
                            );
                            mapping.insert(
                                serde_yaml::Value::String("secret".to_string()),
                                serde_yaml::Value::String(secret),
                            );
                        }
                    }
                }

                let new_content = serde_yaml::to_string(&config)?;
                fs::write(paper_config_path, new_content).await?;
            }
        }

        println!(
            "Configured backend {} for proxy {}",
            backend.name, proxy.name
        );
        Ok(())
    }
}
