use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use tokio::fs;
use tokio::sync::Mutex;

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
    #[serde(default)]
    pub players: String, // e.g. "0/20"
    #[serde(default)]
    pub auto_restart: bool,
    #[serde(default = "default_restart_interval")]
    pub restart_interval: u64, // seconds
    #[serde(default)]
    pub last_start_time: Option<u64>,
}

fn default_restart_interval() -> u64 {
    86400 // 24 hours
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

        let server_info = ServerInfo {
            id: id.clone(),
            name,
            version,
            server_type,
            port,
            max_memory,
            status: ServerStatus::Stopped,
            path: server_path,
            pid: None,
            players: "0/20".to_string(),
            auto_restart: false,
            restart_interval: 86400,
            last_start_time: None,
        };

        self.servers.lock().await.insert(id, server_info.clone());
        Ok(server_info)
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

        #[cfg(target_os = "windows")]
        let java_cmd = "java";

        #[cfg(not(target_os = "windows"))]
        let java_cmd = "java";

        let child = Command::new(java_cmd)
            .args(&[
                format!("-Xmx{}", server_info.max_memory),
                format!("-Xms{}", server_info.max_memory),
                "-jar".to_string(),
                jar_path.to_string_lossy().to_string(),
                "nogui".to_string(),
            ])
            .current_dir(&server_info.path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(Stdio::piped()) // Enable stdin for console commands
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
        // Kill process first (synchronous operation)
        {
            let mut processes = self.processes.lock().unwrap();
            if let Some(mut process) = processes.remove(server_id) {
                process.kill().context("Failed to kill server process")?;
            }
        } // MutexGuard dropped here

        // Update server status (async operation)
        let mut servers = self.servers.lock().await;
        if let Some(server) = servers.get_mut(server_id) {
            server.status = ServerStatus::Stopped;
        }

        Ok(())
    }

    /// Send a command to a running server
    pub async fn send_command(&self, server_id: &str, command: &str) -> Result<()> {
        use std::io::Write;

        let mut processes = self.processes.lock().unwrap();
        let process = processes
            .get_mut(server_id)
            .context("Server not found or not running")?;

        // Get stdin handle
        if let Some(stdin) = process.stdin.as_mut() {
            // Write command followed by newline
            let command_line = format!("{}\n", command.trim());
            stdin
                .write_all(command_line.as_bytes())
                .context("Failed to write command to server")?;
            stdin.flush().context("Failed to flush command to server")?;

            println!("Sent command to server {}: {}", server_id, command);
            Ok(())
        } else {
            anyhow::bail!("Server stdin not available")
        }
    }

    pub async fn get_servers(&self) -> Vec<ServerInfo> {
        self.servers.lock().await.values().cloned().collect()
    }

    pub async fn get_server(&self, server_id: &str) -> Option<ServerInfo> {
        self.servers.lock().await.get(server_id).cloned()
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
            _ => {
                return Err(anyhow::anyhow!(
                    "Unsupported server type: {:?}",
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
            ServerType::Vanilla => {
                anyhow::bail!("Vanilla servers do not support plugins. Please use Paper or Spigot.")
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
            .install_modrinth_plugin(server_id, "autoupdategeyser")
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

    pub async fn install_modrinth_plugin(&self, server_id: &str, project_id: &str) -> Result<()> {
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
            ServerType::Paper => "[\"paper\",\"spigot\",\"bukkit\"]",
            ServerType::Spigot => "[\"spigot\",\"bukkit\"]",
            ServerType::Forge => "[\"forge\"]",
            ServerType::Vanilla => "[\"bukkit\"]", // Fallback
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
        let filename = format!("modrinth_{}.jar", project_id);

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
            ServerType::Paper => {
                "[\"categories:paper\",\"categories:spigot\",\"categories:bukkit\"]"
            }
            ServerType::Spigot => "[\"categories:spigot\",\"categories:bukkit\"]",
            ServerType::Forge => "[\"categories:forge\"]",
            ServerType::Vanilla => "[\"categories:bukkit\"]", // Weak fallback
        };

        let version_facet = format!("[\"versions:{}\"]", version);

        let sort_param = if query.is_empty() {
            "&sort=follows" // Better "Trending/Popular" indicator than total downloads
        } else {
            ""
        };

        // Facets: Plugin AND Version AND Loaders
        let facets = format!(
            "[[\"project_type:plugin\"],{},{}]",
            version_facet, loaders_facet
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
        let server = self
            .servers
            .lock()
            .await
            .get(server_id)
            .context("Server not found")?
            .clone();
        let plugins_path = server.path.join("plugins");

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

    pub async fn install_spigot_plugin(&self, server_id: &str, resource_id: &str) -> Result<()> {
        let download_url = format!(
            "https://api.spiget.org/v2/resources/{}/download",
            resource_id
        );
        let filename = format!("spigot_{}.jar", resource_id);
        self.install_plugin_by_url(server_id, &download_url, Some(filename))
            .await
    }

    pub async fn is_plugin_installed(
        &self,
        server_id: &str,
        plugin_id: &str,
        source: &str,
    ) -> Result<bool> {
        let server = self
            .servers
            .lock()
            .await
            .get(server_id)
            .context("Server not found")?
            .clone();
        let plugins_path = server.path.join("plugins");

        let filename = match source {
            "Modrinth" => format!("modrinth_{}.jar", plugin_id),
            "Spigot" => format!("spigot_{}.jar", plugin_id),
            _ => return Ok(false),
        };

        Ok(plugins_path.join(filename).exists())
    }

    pub async fn uninstall_plugin(
        &self,
        server_id: &str,
        plugin_id: &str,
        source: &str,
    ) -> Result<()> {
        let server = self
            .servers
            .lock()
            .await
            .get(server_id)
            .context("Server not found")?
            .clone();
        let plugins_path = server.path.join("plugins");

        let filename = match source {
            "Modrinth" => format!("modrinth_{}.jar", plugin_id),
            "Spigot" => format!("spigot_{}.jar", plugin_id),
            _ => anyhow::bail!("Unknown source"),
        };

        let file_path = plugins_path.join(filename);
        if file_path.exists() {
            fs::remove_file(file_path).await?;
        }

        Ok(())
    }

    pub async fn set_auto_restart(
        &self,
        server_id: &str,
        enabled: bool,
        interval: u64,
    ) -> Result<()> {
        let mut servers = self.servers.lock().await;

        if let Some(server) = servers.get_mut(server_id) {
            server.auto_restart = enabled;
            server.restart_interval = interval;
            Ok(())
        } else {
            anyhow::bail!("Server not found");
        }
    }

    pub async fn restart_server(&self, server_id: &str) -> Result<()> {
        let status = {
            let servers = self.servers.lock().await;
            if let Some(server) = servers.get(server_id) {
                server.status.clone()
            } else {
                return Ok(());
            }
        };

        if status == ServerStatus::Running {
            self.stop_server(server_id).await?;
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        }

        self.start_server(server_id).await
    }
}
