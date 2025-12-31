// Prismarine Bridge - Tunneling service using bore
// This provides a reliable way to expose Minecraft servers without port forwarding
// bore is super simple - no registration, no tokens, just works!

use anyhow::{Context, Result};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

pub struct PrismarineBridge {
    process: Mutex<Option<Child>>,
    bore_path: PathBuf,
    config_dir: PathBuf,
    status: Arc<Mutex<BridgeStatus>>,
}

impl PrismarineBridge {
    pub fn new() -> Self {
        let app_data = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Prismarine")
            .join("bridge");

        Self {
            process: Mutex::new(None),
            bore_path: app_data.join("bore.exe"),
            config_dir: app_data.clone(),
            status: Arc::new(Mutex::new(BridgeStatus::Stopped)),
        }
    }

    /// Check if bore binary exists
    pub fn is_installed(&self) -> bool {
        self.bore_path.exists()
    }

    /// bore doesn't need authtoken, so always return true
    pub fn has_authtoken(&self) -> bool {
        true // bore doesn't need tokens!
    }

    /// bore doesn't need authtoken, this is a no-op
    pub fn set_authtoken(&self, _token: &str) -> Result<()> {
        Ok(()) // bore doesn't need tokens!
    }

    /// Download bore binary if not present
    pub async fn ensure_installed(&self) -> Result<()> {
        if self.is_installed() {
            return Ok(());
        }

        // Create directory
        std::fs::create_dir_all(&self.config_dir)?;

        // Update status
        *self.status.lock().unwrap() = BridgeStatus::Downloading;

        // Download bore for Windows (x86_64)
        let download_url = "https://github.com/ekzhang/bore/releases/download/v0.6.0/bore-v0.6.0-x86_64-pc-windows-msvc.zip";

        println!("[Prismarine Bridge] Downloading bore...");

        let response = reqwest::get(download_url)
            .await
            .context("Failed to download bore")?;

        let bytes = response.bytes().await?;

        // Save zip to temp file
        let zip_path = self.config_dir.join("bore.zip");
        std::fs::write(&zip_path, &bytes)?;

        // Extract bore.exe from zip
        let file = std::fs::File::open(&zip_path)?;
        let mut archive = zip::ZipArchive::new(file)?;

        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let name = file.name();
            if name.ends_with("bore.exe") || name == "bore.exe" {
                let mut outfile = std::fs::File::create(&self.bore_path)?;
                std::io::copy(&mut file, &mut outfile)?;
                break;
            }
        }

        // Delete zip
        let _ = std::fs::remove_file(&zip_path);

        println!("[Prismarine Bridge] Download complete!");

        Ok(())
    }

    /// Start the bridge
    pub fn start(
        &self,
        port: u16,
        remote_server: Option<String>,
        secret: Option<String>,
    ) -> Result<()> {
        // Kill existing process if any
        self.stop()?;

        *self.status.lock().unwrap() = BridgeStatus::Starting;

        let server = remote_server.unwrap_or_else(|| "bore.pub".to_string());
        println!(
            "[Prismarine Bridge] Starting bore local {} --to {}...",
            port, server
        );

        let mut command = Command::new(&self.bore_path);
        command.arg("local");
        command.arg(port.to_string());
        command.arg("--to");
        command.arg(server);

        if let Some(s) = secret {
            // Only add secret if not empty
            if !s.is_empty() {
                command.arg("--secret");
                command.arg(s);
            }
        }

        let mut child = command
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Failed to start bore")?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        // Store process
        *self.process.lock().unwrap() = Some(child);

        // bore outputs to stderr, so monitor both
        let status_arc = Arc::clone(&self.status);

        // Monitor stderr (main output)
        if let Some(stderr) = stderr {
            let status_clone = Arc::clone(&status_arc);
            thread::spawn(move || {
                println!("[Prismarine Bridge] Stderr monitor thread started");
                let reader = BufReader::new(stderr);
                for line in reader.lines() {
                    if let Ok(line) = line {
                        println!("[bore stderr] {}", line);
                        parse_bore_output(&line, &status_clone);
                    }
                }
                println!("[Prismarine Bridge] Stderr monitor thread ended");
            });
        }

        // Monitor stdout
        if let Some(stdout) = stdout {
            let status_clone = Arc::clone(&status_arc);
            thread::spawn(move || {
                println!("[Prismarine Bridge] Stdout monitor thread started");
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    if let Ok(line) = line {
                        println!("[bore stdout] {}", line);
                        parse_bore_output(&line, &status_clone);
                    }
                }
                println!("[Prismarine Bridge] Stdout monitor thread ended");
            });
        }

        Ok(())
    }

    /// Stop the bridge
    pub fn stop(&self) -> Result<()> {
        if let Some(mut child) = self.process.lock().unwrap().take() {
            println!("[Prismarine Bridge] Stopping bore");
            let _ = child.kill();
        }
        *self.status.lock().unwrap() = BridgeStatus::Stopped;
        Ok(())
    }

    /// Check if running
    pub fn is_running(&self) -> bool {
        self.process.lock().unwrap().is_some()
    }

    /// Get current status
    pub fn get_status(&self) -> BridgeStatus {
        self.status.lock().unwrap().clone()
    }
}

// Parse bore output to find connection address
fn parse_bore_output(line: &str, status: &Arc<Mutex<BridgeStatus>>) {
    let mut status = status.lock().unwrap();

    // bore outputs: "listening at bore.pub:XXXXX"
    if line.to_lowercase().contains("listening") && line.contains("bore.pub") {
        // Extract the address
        if let Some(addr) = extract_bore_address(line) {
            println!("[Prismarine Bridge] Found bore address: {}", addr);
            *status = BridgeStatus::Connected(addr);
            return;
        }
    }

    // Check for errors
    if line.to_lowercase().contains("error") {
        *status = BridgeStatus::Error(line.to_string());
        return;
    }

    // If we see any output and still starting, mark as running
    if matches!(*status, BridgeStatus::Starting) {
        *status = BridgeStatus::Running;
    }
}

// Extract bore.pub:PORT from output
fn extract_bore_address(text: &str) -> Option<String> {
    // Look for pattern like "bore.pub:XXXXX"
    for word in text.split_whitespace() {
        if word.contains("bore.pub:") {
            let clean = word.trim_matches(|c| c == '"' || c == '\'' || c == ',' || c == '.');
            return Some(clean.to_string());
        }
    }

    // Alternative: look for "at" keyword
    if let Some(at_idx) = text.find(" at ") {
        let rest = &text[at_idx + 4..];
        let addr = rest.split_whitespace().next()?;
        if addr.contains("bore.pub") {
            return Some(addr.trim().to_string());
        }
    }

    None
}

#[derive(Debug, Clone, serde::Serialize)]
pub enum BridgeStatus {
    /// Not started
    Stopped,
    /// Downloading bore binary
    Downloading,
    /// Starting up
    Starting,
    /// Running but no address yet
    Running,
    /// Connected with tunnel address
    Connected(String),
    /// Error occurred
    Error(String),
}
