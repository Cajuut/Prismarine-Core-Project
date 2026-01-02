use crate::java_manager::JavaManager;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use tauri::Emitter;
use tokio::fs;
use tokio::io::AsyncBufReadExt;
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

/// Sanitize a server name for use as a folder name
fn sanitize_folder_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            // Replace invalid filename characters with underscore
            if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();

    // Trim whitespace and limit length
    let sanitized = sanitized.trim();
    if sanitized.len() > 64 {
        sanitized[..64].trim().to_string()
    } else if sanitized.is_empty() {
        "server".to_string()
    } else {
        sanitized.to_string()
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
    Mohist,
    Banner,
    Purpur,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ServerStatus {
    Stopped,
    Starting,
    Downloading(String), // Downloads in progress (e.g., "Java 21", "Server JAR")
    Running,
    Stopping,
}

pub struct ServerManager {
    servers: Arc<Mutex<HashMap<String, ServerInfo>>>,
    processes: Arc<std::sync::Mutex<HashMap<String, Child>>>,
    base_path: PathBuf,
    java_manager: Arc<JavaManager>,
}

impl ServerManager {
    pub fn new(base_path: PathBuf) -> Self {
        Self {
            servers: Arc::new(Mutex::new(HashMap::new())),
            processes: Arc::new(std::sync::Mutex::new(HashMap::new())),
            java_manager: Arc::new(JavaManager::new(base_path.clone())),
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
        app_handle: Option<tauri::AppHandle>,
        name: String,
        version: String,
        server_type: ServerType,
        port: u16,
        max_memory: String,
    ) -> Result<ServerInfo> {
        let id = uuid::Uuid::new_v4().to_string();

        // Sanitize name for folder (remove invalid characters, limit length)
        let folder_name = sanitize_folder_name(&name);
        let mut server_path = self.base_path.join(&folder_name);

        // If folder already exists, append ID suffix to make unique
        if server_path.exists() {
            server_path = self.base_path.join(format!("{}_{}", folder_name, &id[..8]));
        }

        // Create server directory
        fs::create_dir_all(&server_path)
            .await
            .context("Failed to create server directory")?;

        if let Some(h) = &app_handle {
            let _ = h.emit(
                "build-log",
                format!("Creating server directory: {:?}", server_path),
            );
        }

        // Download server JAR
        self.download_server_jar(app_handle, &server_path, &server_type, &version)
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

        let required_java = JavaManager::get_java_version_for_mc(&server_info.version);
        println!(
            "Server {} requires Java {}",
            server_info.version, required_java
        );

        // Check if Java needs to be downloaded and update status
        if !self.java_manager.is_java_installed(required_java) {
            // Update status to Downloading
            {
                let mut servers = self.servers.lock().await;
                if let Some(server) = servers.get_mut(server_id) {
                    server.status = ServerStatus::Downloading(format!(
                        "Java {}をダウンロード中...",
                        required_java
                    ));
                }
            }
        }

        let java_path = self.java_manager.get_java_executable(required_java).await?;
        let java_cmd = java_path.to_string_lossy().to_string();

        println!("Using Java: {}", java_cmd);

        let mut child = Command::new(java_cmd)
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

        // --- Log Capture Start ---
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let log_path = server_info.path.join("logs").join("server_console.log");

        // Ensure logs dir exists
        std::fs::create_dir_all(log_path.parent().unwrap())?;

        // Clone path for threads
        let log_path_out = log_path.clone();
        let log_path_err = log_path.clone();

        // Spawn threads to pipe output to file
        if let Some(out) = stdout {
            std::thread::spawn(move || {
                use std::io::{BufRead, BufReader, Write};
                let reader = BufReader::new(out);
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(log_path_out)
                    .unwrap(); // TODO: Handle error better

                for line in reader.lines() {
                    if let Ok(l) = line {
                        // Also print to terminal for debugging
                        println!("[SERVER] {}", l);
                        let _ = writeln!(file, "{}", l);
                    }
                }
            });
        }

        if let Some(err) = stderr {
            std::thread::spawn(move || {
                use std::io::{BufRead, BufReader, Write};
                let reader = BufReader::new(err);
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(log_path_err)
                    .unwrap();

                for line in reader.lines() {
                    if let Ok(l) = line {
                        // Also print to terminal for debugging
                        println!("[SERVER STDERR] {}", l);
                        let _ = writeln!(file, "[STDERR] {}", l);
                    }
                }
            });
        }
        // --- Log Capture End ---

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
        app_handle: Option<tauri::AppHandle>,
        server_path: &Path,
        server_type: &ServerType,
        version: &str,
    ) -> Result<()> {
        let jar_path = server_path.join("server.jar");

        if let Some(h) = &app_handle {
            let _ = h.emit(
                "build-log",
                format!("Fetching download URL for {:?} {}...", server_type, version),
            );
        }

        let url = match server_type {
            ServerType::Vanilla => self.get_vanilla_url(version).await?,
            ServerType::Paper => self.get_paper_url(version).await?,
            ServerType::Forge => self.get_forge_url(version).await?,
            ServerType::Mohist => self.get_mohist_url(version).await?,
            ServerType::Banner => self.get_banner_url(version).await?,
            ServerType::Spigot => "build_tools".to_string(),
            ServerType::Purpur => self.get_purpur_url(version).await?,
        };

        if server_type == &ServerType::Spigot {
            println!("Starting Spigot build process via BuildTools...");
            // Spigot requires special handling to build from source
            return self
                .build_spigot_server(app_handle, server_path, version)
                .await;
        }

        if let Some(h) = &app_handle {
            let _ = h.emit("build-log", format!("Downloading server JAR from: {}", url));
        }
        println!("Downloading server JAR from: {}", url);
        // Use client with UA and redirect following
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .redirect(reqwest::redirect::Policy::limited(10))
            .build()?;
        let response = client.get(&url).send().await?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Failed to download server JAR: Status {}",
                response.status()
            ));
        }

        let content = response.bytes().await?;

        // Validate that we got a JAR file (starts with ZIP magic number PK)
        if content.len() < 4 || &content[0..2] != b"PK" {
            // Log the actual content for debugging
            let preview = String::from_utf8_lossy(&content[..std::cmp::min(500, content.len())]);
            println!("Download failed - received content preview: {}", preview);

            // Check if it's HTML (likely an error page)
            if preview.contains("<html") || preview.contains("<!DOCTYPE") {
                return Err(anyhow::anyhow!(
                    "Download failed: Server returned an HTML page instead of JAR file."
                ));
            }
            // Check if it's a redirect response
            if preview.contains("Moved") || preview.contains("redirect") || preview.contains("302")
            {
                return Err(anyhow::anyhow!(
                    "Download failed: Server returned a redirect that wasn't followed. Content: {}",
                    preview
                ));
            }
            return Err(anyhow::anyhow!(
                "Downloaded file is not a valid JAR (size: {} bytes). Preview: {}",
                content.len(),
                &preview[..std::cmp::min(100, preview.len())]
            ));
        }

        // Special handling for Forge - download installer and run it
        if matches!(server_type, ServerType::Forge) {
            let installer_path = server_path.join("forge-installer.jar");
            fs::write(&installer_path, &content).await?;

            if let Some(h) = &app_handle {
                let _ = h.emit(
                    "build-log",
                    "Forge installer downloaded. Running installer...",
                );
            }

            // Get Java executable for running installer
            let required_java = crate::java_manager::JavaManager::get_java_version_for_mc(version);
            let java_path = self.java_manager.get_java_executable(required_java).await?;

            println!("Running Forge installer...");

            let mut command = tokio::process::Command::new(&java_path);
            command
                .args(&["-jar", "forge-installer.jar", "--installServer"])
                .current_dir(server_path)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .env_remove("M2_HOME")
                .env_remove("MAVEN_HOME")
                .env_remove("MVN_HOME");

            let mut child = command.spawn()?;

            let stdout = child.stdout.take().context("Failed to open stdout")?;
            let stderr = child.stderr.take().context("Failed to open stderr")?;

            let mut stdout_reader = tokio::io::BufReader::new(stdout).lines();
            let mut stderr_reader = tokio::io::BufReader::new(stderr).lines();

            if let Some(handle) = app_handle.clone() {
                let handle_out = handle.clone();
                tokio::spawn(async move {
                    while let Ok(Some(line)) = stdout_reader.next_line().await {
                        println!("[Forge] {}", line);
                        let _ = handle_out.emit("build-log", line);
                    }
                });
                let handle_err = handle.clone();
                tokio::spawn(async move {
                    while let Ok(Some(line)) = stderr_reader.next_line().await {
                        println!("[Forge ERR] {}", line);
                        let _ = handle_err.emit("build-log", format!("ERROR: {}", line));
                    }
                });
            } else {
                // Fallback logging without emit
                tokio::spawn(async move {
                    while let Ok(Some(line)) = stdout_reader.next_line().await {
                        println!("[Forge] {}", line);
                    }
                });
                tokio::spawn(async move {
                    while let Ok(Some(line)) = stderr_reader.next_line().await {
                        println!("[Forge ERR] {}", line);
                    }
                });
            }

            let status = child.wait().await?;

            if !status.success() {
                return Err(anyhow::anyhow!(
                    "Forge installer failed with status: {}",
                    status
                ));
            }

            // Find the generated server jar (it's usually named like forge-*-server.jar or just run.sh)
            // For newer Forge versions, we need to find the actual jar
            let mut found_jar = false;
            if let Ok(entries) = std::fs::read_dir(server_path) {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if (name.starts_with("forge-") || name.starts_with("minecraft_server"))
                        && name.ends_with(".jar")
                        && !name.contains("installer")
                    {
                        // Rename to server.jar
                        let src = entry.path();
                        std::fs::rename(&src, &jar_path)?;
                        found_jar = true;
                        break;
                    }
                }
            }

            // New Forge versions (1.17+) use a different system with run.sh/run.bat
            // Check for libraries/net/minecraftforge/forge/*/server.jar pattern
            if !found_jar {
                // Look for run.bat/run.sh which indicates new Forge format
                let run_script = if cfg!(target_os = "windows") {
                    server_path.join("run.bat")
                } else {
                    server_path.join("run.sh")
                };

                if run_script.exists() {
                    // For new Forge, we'll create a wrapper script that calls run.bat/run.sh
                    // For now, just create a dummy server.jar that points users to the right file
                    println!("Forge installed in new format. Server will use run script.");
                    found_jar = true;
                }
            }

            // Clean up installer
            let _ = std::fs::remove_file(&installer_path);
            let _ = std::fs::remove_file(server_path.join("forge-installer.jar.log"));

            if !found_jar {
                println!("Warning: Could not locate Forge server JAR, but installer completed.");
            }
        } else {
            fs::write(&jar_path, content).await?;
        }

        // Remove Zone.Identifier to prevent Windows SmartScreen/Defender warnings
        #[cfg(target_os = "windows")]
        {
            let zone_id_path = format!("{}:Zone.Identifier", jar_path.to_string_lossy());
            let _ = std::fs::remove_file(&zone_id_path);
        }

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

    async fn get_purpur_url(&self, version: &str) -> Result<String> {
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

    async fn get_forge_url(&self, version: &str) -> Result<String> {
        // Forge downloads from the official Maven repository
        // First, we need to find the latest Forge version for this MC version
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;

        // Forge promotions API to get recommended/latest version
        let promos_url =
            "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";
        let promos: serde_json::Value = client.get(promos_url).send().await?.json().await?;

        let promos_obj = promos["promos"]
            .as_object()
            .context("Invalid Forge promos format")?;

        // Try recommended first, then latest
        let forge_version = promos_obj
            .get(&format!("{}-recommended", version))
            .or_else(|| promos_obj.get(&format!("{}-latest", version)))
            .and_then(|v| v.as_str())
            .context(format!("No Forge version found for Minecraft {}", version))?;

        // Forge server installer URL format
        let full_version = format!("{}-{}", version, forge_version);
        let url = format!(
            "https://maven.minecraftforge.net/net/minecraftforge/forge/{}/forge-{}-installer.jar",
            full_version, full_version
        );

        Ok(url)
    }

    #[allow(dead_code)]
    async fn get_spigot_url(&self, _version: &str) -> Result<String> {
        // Spigot is disabled due to licensing restrictions
        Err(anyhow::anyhow!(
            "Spigot is not supported due to licensing restrictions"
        ))
    }

    async fn get_mohist_url(&self, version: &str) -> Result<String> {
        // Use builds-raw directory for direct JAR downloads
        // URL pattern: https://mohistmc.com/builds-raw/Mohist-{version}/Mohist-{version}-{build}.jar
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;

        // Get directory listing from builds-raw
        let dir_url = format!("https://mohistmc.com/builds-raw/Mohist-{}/", version);
        println!("Fetching Mohist builds from: {}", dir_url);

        let resp = client.get(&dir_url).send().await?;

        if !resp.status().is_success() {
            anyhow::bail!(
                "Mohist {} のビルドディレクトリにアクセスできません (HTTP {})",
                version,
                resp.status()
            );
        }

        let html = resp.text().await?;

        // Parse HTML directory listing for JAR files with numeric build numbers
        // Format: href="Mohist-1.20.1-923.jar"
        let prefix = format!("Mohist-{}-", version);
        let mut max_build: u32 = 0;

        for part in html.split("href=\"") {
            if let Some(end_quote) = part.find('"') {
                let href = &part[..end_quote];
                if href.starts_with(&prefix) && href.ends_with(".jar") {
                    // Extract number between prefix and .jar
                    let num_str = &href[prefix.len()..href.len() - 4];
                    if let Ok(num) = num_str.parse::<u32>() {
                        if num > max_build {
                            max_build = num;
                        }
                    }
                }
            }
        }

        if max_build == 0 {
            anyhow::bail!("Mohist {} のビルドが見つかりません。", version);
        }

        let download_url = format!(
            "https://mohistmc.com/builds-raw/Mohist-{}/Mohist-{}-{}.jar",
            version, version, max_build
        );
        println!(
            "Mohist direct download: {} (build {})",
            download_url, max_build
        );
        Ok(download_url)
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

    pub async fn fetch_forge_versions(&self) -> Result<Vec<String>> {
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;

        let promos_url =
            "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json";
        let promos: serde_json::Value = client.get(promos_url).send().await?.json().await?;

        let promos_obj = promos["promos"]
            .as_object()
            .context("Invalid Forge promos format")?;

        // Extract unique MC versions (remove -recommended/-latest suffix)
        let mut versions: Vec<String> = promos_obj
            .keys()
            .filter_map(|k| {
                k.strip_suffix("-recommended")
                    .or_else(|| k.strip_suffix("-latest"))
            })
            .map(|s| s.to_string())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();

        // Sort by version (newest first)
        versions.sort_by(|a, b| {
            let a_parts: Vec<u32> = a.split('.').filter_map(|s| s.parse().ok()).collect();
            let b_parts: Vec<u32> = b.split('.').filter_map(|s| s.parse().ok()).collect();
            b_parts.cmp(&a_parts)
        });

        Ok(versions)
    }

    pub async fn fetch_mohist_versions(&self) -> Result<Vec<String>> {
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;

        let url = "https://mohistmc.com/api/v2/projects/mohist";
        let resp: serde_json::Value = client.get(url).send().await?.json().await?;

        // Known versions without builds (latest MC versions often have no Mohist builds yet)
        let versions_without_builds = ["1.21.4", "1.21.3", "1.21.2", "1.21.1", "1.21", "1.20.6"];

        let mut versions: Vec<String> = resp["versions"]
            .as_array()
            .context("Invalid Mohist response format")?
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .filter(|v| !versions_without_builds.contains(&v.as_str()))
            .collect();

        // Sort by version (newest first)
        versions.sort_by(|a, b| {
            let a_parts: Vec<u32> = a.split('.').filter_map(|s| s.parse().ok()).collect();
            let b_parts: Vec<u32> = b.split('.').filter_map(|s| s.parse().ok()).collect();
            b_parts.cmp(&a_parts)
        });

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

        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;

        // --- ViaVersion ---
        let api_url =
            "https://hangar.papermc.io/api/v1/projects/ViaVersion/versions?limit=1&platform=PAPER";
        println!("Fetching ViaVersion info from: {}", api_url);

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

        // --- ViaBackwards ---
        let api_url_backwards =
            "https://hangar.papermc.io/api/v1/projects/ViaBackwards/versions?limit=1&platform=PAPER";
        println!("Fetching ViaBackwards info from: {}", api_url_backwards);

        let resp_backwards: serde_json::Value =
            client.get(api_url_backwards).send().await?.json().await?;
        let results_backwards = resp_backwards["result"]
            .as_array()
            .context("Invalid Hangar API response for ViaBackwards")?;

        if let Some(latest_backwards) = results_backwards.first() {
            if let Some(dl_url) = latest_backwards["downloads"]["PAPER"]["downloadUrl"].as_str() {
                println!("Found ViaBackwards download URL: {}", dl_url);
                self.install_plugin(&plugins_path, dl_url, "ViaBackwards.jar")
                    .await
                    .context("Failed to install ViaBackwards")?;
            }
        }

        // --- ViaRewind (optional, for 1.8 clients) ---
        let api_url_rewind =
            "https://hangar.papermc.io/api/v1/projects/ViaRewind/versions?limit=1&platform=PAPER";
        println!("Fetching ViaRewind info from: {}", api_url_rewind);

        let resp_rewind: serde_json::Value =
            client.get(api_url_rewind).send().await?.json().await?;
        let results_rewind = resp_rewind["result"].as_array();

        if let Some(results) = results_rewind {
            if let Some(latest_rewind) = results.first() {
                if let Some(dl_url) = latest_rewind["downloads"]["PAPER"]["downloadUrl"].as_str() {
                    println!("Found ViaRewind download URL: {}", dl_url);
                    let _ = self
                        .install_plugin(&plugins_path, dl_url, "ViaRewind.jar")
                        .await;
                    // ViaRewind is optional, don't fail if it doesn't work
                }
            }
        }

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
            ServerType::Mohist => "[\"forge\",\"bukkit\"]", // Mohist is Forge + Bukkit hybrid
            ServerType::Banner => "[\"fabric\",\"bukkit\"]", // Banner is Fabric + Bukkit hybrid
            ServerType::Purpur => "[\"purpur\",\"paper\",\"spigot\",\"bukkit\"]",
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
            ServerType::Mohist => "[\"categories:forge\",\"categories:bukkit\"]",
            ServerType::Banner => "[\"categories:fabric\",\"categories:bukkit\"]",
            ServerType::Purpur => "[\"categories:purpur\",\"categories:paper\",\"categories:spigot\",\"categories:bukkit\"]",
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

    async fn build_spigot_server(
        &self,
        app_handle: Option<tauri::AppHandle>,
        server_path: &Path,
        version: &str,
    ) -> Result<()> {
        let build_tools_url = "https://hub.spigotmc.org/jenkins/job/BuildTools/lastSuccessfulBuild/artifact/target/BuildTools.jar";
        let build_tools_path = server_path.join("BuildTools.jar");

        // Download BuildTools.jar
        if let Some(h) = &app_handle {
            let _ = h.emit("build-log", "Downloading BuildTools.jar...");
        }
        println!("Downloading BuildTools.jar...");
        let client = reqwest::Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;

        let response = client.get(build_tools_url).send().await?;
        if !response.status().is_success() {
            return Err(anyhow::anyhow!("Failed to download BuildTools.jar"));
        }
        let content = response.bytes().await?;
        fs::write(&build_tools_path, content).await?;

        // Ensure Java is available
        let required_java = JavaManager::get_java_version_for_mc(version);
        if !self.java_manager.is_java_installed(required_java) {
            let msg = format!(
                "Java {} missing for BuildTools, downloading...",
                required_java
            );
            println!("{}", msg);
            if let Some(h) = &app_handle {
                let _ = h.emit("build-log", msg);
            }
            self.java_manager
                .download_and_install_java(required_java)
                .await?;
        }
        let java_path = self.java_manager.get_java_executable(required_java).await?;
        let java_cmd = java_path.to_string_lossy().to_string();

        println!("Running BuildTools (this may take a while)...");
        if let Some(h) = &app_handle {
            let _ = h.emit("build-log", "Running BuildTools (this may take a while)...");
        }

        #[cfg(target_os = "windows")]
        let mut command = tokio::process::Command::new(&java_cmd);
        #[cfg(target_os = "windows")]
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW

        #[cfg(not(target_os = "windows"))]
        let mut command = tokio::process::Command::new(&java_cmd);

        command
            .arg("-jar")
            .arg(&build_tools_path)
            .arg("--rev")
            .arg(version)
            .current_dir(server_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_remove("M2_HOME")
            .env_remove("MAVEN_HOME")
            .env_remove("MVN_HOME");

        let mut child = command.spawn()?;

        let stdout = child.stdout.take().context("Failed to open stdout")?;
        let stderr = child.stderr.take().context("Failed to open stderr")?;

        let mut stdout_reader = tokio::io::BufReader::new(stdout).lines();
        let mut stderr_reader = tokio::io::BufReader::new(stderr).lines();

        if let Some(handle) = app_handle {
            let handle_out = handle.clone();
            tokio::spawn(async move {
                while let Ok(Some(line)) = stdout_reader.next_line().await {
                    println!("[BuildTools] {}", line);
                    let _ = handle_out.emit("build-log", line);
                }
            });

            let handle_err = handle.clone();
            tokio::spawn(async move {
                while let Ok(Some(line)) = stderr_reader.next_line().await {
                    println!("[BuildTools ERR] {}", line);
                    let _ = handle_err.emit("build-log", format!("ERROR: {}", line));
                }
            });
        } else {
            tokio::spawn(async move {
                while let Ok(Some(line)) = stdout_reader.next_line().await {
                    println!("[BuildTools] {}", line);
                }
            });
            tokio::spawn(async move {
                while let Ok(Some(line)) = stderr_reader.next_line().await {
                    println!("[BuildTools ERR] {}", line);
                }
            });
        }

        let status = child.wait().await?;

        if !status.success() {
            return Err(anyhow::anyhow!("BuildTools failed with status: {}", status));
        }

        // Find spigot-*.jar and rename to server.jar
        let spigot_jar = format!("spigot-{}.jar", version);
        let built_jar = server_path.join(&spigot_jar);

        if built_jar.exists() {
            println!("Build successful! Renaming {} to server.jar", spigot_jar);
            fs::rename(built_jar, server_path.join("server.jar")).await?;
        } else {
            return Err(anyhow::anyhow!(
                "BuildTools finished but {} not found",
                spigot_jar
            ));
        }

        Ok(())
    }
}
