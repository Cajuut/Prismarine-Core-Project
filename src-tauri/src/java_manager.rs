use anyhow::{Context, Result};
use reqwest::Client;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use tokio::fs;

#[derive(Debug, Clone)]
pub struct JavaManager {
    runtimes_path: PathBuf,
}

impl JavaManager {
    pub fn new(base_path: PathBuf) -> Self {
        Self {
            runtimes_path: base_path.join("runtimes"),
        }
    }

    /// Returns the path to the java executable for the requested Java major version.
    /// Checks in this order:
    /// 1. App's local runtimes folder
    /// 2. System Java (JAVA_HOME, PATH, common locations)
    /// 3. Downloads if not found
    pub async fn get_java_executable(&self, version: u8) -> Result<PathBuf> {
        // 1. Check app's local runtimes folder first
        let java_home = self.runtimes_path.join(format!("java-{}", version));
        let java_bin = if cfg!(target_os = "windows") {
            java_home.join("bin").join("java.exe")
        } else {
            java_home.join("bin").join("java")
        };

        if java_bin.exists() {
            println!("Using local Java {} from app runtimes", version);
            return Ok(java_bin);
        }

        // Check subdirectories (Adoptium extracts to a subfolder)
        if let Ok(bin) = self.find_java_executable(&java_home).await {
            println!("Using local Java {} from app runtimes (subfolder)", version);
            return Ok(bin);
        }

        // 2. Check system Java installations
        if let Some(system_java) = self.find_system_java(version) {
            println!("Using system Java {} at: {:?}", version, system_java);
            return Ok(system_java);
        }

        // 3. Download and install
        println!(
            "Java {} not found locally or on system, downloading...",
            version
        );
        self.download_and_install_java(version).await?;

        if java_bin.exists() {
            Ok(java_bin)
        } else {
            self.find_java_executable(&java_home).await
        }
    }

    /// Try to find an existing Java installation on the system
    fn find_system_java(&self, version: u8) -> Option<PathBuf> {
        let java_exe = if cfg!(target_os = "windows") {
            "java.exe"
        } else {
            "java"
        };

        // Check JAVA_HOME environment variable
        if let Ok(java_home) = std::env::var("JAVA_HOME") {
            let java_path = PathBuf::from(&java_home).join("bin").join(java_exe);
            if java_path.exists() {
                if let Some(ver) = self.get_java_version(&java_path) {
                    if ver == version {
                        return Some(java_path);
                    }
                }
            }
        }

        // Check common Windows installation paths
        #[cfg(target_os = "windows")]
        {
            let common_paths = vec![
                format!(
                    "C:\\Program Files\\Java\\jdk-{}\\bin\\{}",
                    version, java_exe
                ),
                format!("C:\\Program Files\\Java\\jdk{}\\bin\\{}", version, java_exe),
                format!(
                    "C:\\Program Files\\Eclipse Adoptium\\jdk-{}.0*-hotspot\\bin\\{}",
                    version, java_exe
                ),
                format!(
                    "C:\\Program Files\\Zulu\\zulu-{}\\bin\\{}",
                    version, java_exe
                ),
                format!(
                    "C:\\Program Files\\Microsoft\\jdk-{}*\\bin\\{}",
                    version, java_exe
                ),
            ];

            // Also check Java folder for any matching version
            if let Ok(entries) = std::fs::read_dir("C:\\Program Files\\Java") {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.contains(&format!("jdk-{}", version))
                        || name.contains(&format!("jdk{}", version))
                    {
                        let java_path = entry.path().join("bin").join(java_exe);
                        if java_path.exists() {
                            return Some(java_path);
                        }
                    }
                }
            }

            // Check Eclipse Adoptium folder
            if let Ok(entries) = std::fs::read_dir("C:\\Program Files\\Eclipse Adoptium") {
                for entry in entries.flatten() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    if name.starts_with(&format!("jdk-{}", version)) {
                        let java_path = entry.path().join("bin").join(java_exe);
                        if java_path.exists() {
                            return Some(java_path);
                        }
                    }
                }
            }

            for path in common_paths {
                let p = PathBuf::from(&path);
                if p.exists() {
                    return Some(p);
                }
            }
        }

        // Check PATH for java command and verify version
        if let Ok(output) = std::process::Command::new("where").arg("java").output() {
            if output.status.success() {
                let paths = String::from_utf8_lossy(&output.stdout);
                for line in paths.lines() {
                    let path = PathBuf::from(line.trim());
                    if path.exists() {
                        if let Some(ver) = self.get_java_version(&path) {
                            if ver == version {
                                return Some(path);
                            }
                        }
                    }
                }
            }
        }

        None
    }

    /// Get the major version of a Java executable
    fn get_java_version(&self, java_path: &Path) -> Option<u8> {
        let output = std::process::Command::new(java_path)
            .arg("-version")
            .output()
            .ok()?;

        // Java version is printed to stderr
        let version_str = String::from_utf8_lossy(&output.stderr);

        // Parse version from output like: openjdk version "17.0.1" or java version "1.8.0_301"
        for line in version_str.lines() {
            if line.contains("version") {
                // Extract version number between quotes
                if let Some(start) = line.find('"') {
                    if let Some(end) = line[start + 1..].find('"') {
                        let ver = &line[start + 1..start + 1 + end];
                        // Handle both "17.0.1" and "1.8.0" formats
                        if ver.starts_with("1.") {
                            // Old format: 1.8.0 -> 8
                            let parts: Vec<&str> = ver.split('.').collect();
                            if parts.len() >= 2 {
                                return parts[1].parse().ok();
                            }
                        } else {
                            // New format: 17.0.1 -> 17
                            let parts: Vec<&str> = ver.split('.').collect();
                            if !parts.is_empty() {
                                return parts[0].parse().ok();
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Check if a specific Java version is already installed (locally or on system)
    pub fn is_java_installed(&self, version: u8) -> bool {
        // Check local runtimes folder
        let java_home = self.runtimes_path.join(format!("java-{}", version));
        let java_bin = if cfg!(target_os = "windows") {
            java_home.join("bin").join("java.exe")
        } else {
            java_home.join("bin").join("java")
        };

        if java_bin.exists() {
            return true;
        }

        // Check subdirectories (Adoptium extracts to a subfolder)
        if let Ok(entries) = std::fs::read_dir(&java_home) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let bin_java = if cfg!(target_os = "windows") {
                        path.join("bin").join("java.exe")
                    } else {
                        path.join("bin").join("java")
                    };
                    if bin_java.exists() {
                        return true;
                    }
                }
            }
        }

        // Check system Java
        self.find_system_java(version).is_some()
    }

    async fn find_java_executable(&self, root_dir: &Path) -> Result<PathBuf> {
        let mut read_dir = fs::read_dir(root_dir).await?;

        // Simple search: look for a directory that looks like a JDK root if the direct check failed
        // But first, maybe the zip extracted into a subdirectory?
        while let Ok(Some(entry)) = read_dir.next_entry().await {
            let path = entry.path();
            if path.is_dir() {
                let bin_java = if cfg!(target_os = "windows") {
                    path.join("bin").join("java.exe")
                } else {
                    path.join("bin").join("java")
                };
                if bin_java.exists() {
                    return Ok(bin_java);
                }
            }
        }

        anyhow::bail!("Could not find java executable in installed directory")
    }

    pub async fn download_and_install_java(&self, version: u8) -> Result<()> {
        println!("Installing Java {}...", version);

        let url = self.get_adoptium_download_url(version).await?;
        println!("Downloading from: {}", url);

        let client = Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;

        let response = client.get(&url).send().await?;

        if !response.status().is_success() {
            anyhow::bail!("Failed to download Java: {}", response.status());
        }

        let bytes = response.bytes().await?;

        // Prepare target directory
        let target_dir = self.runtimes_path.join(format!("java-{}", version));
        if target_dir.exists() {
            fs::remove_dir_all(&target_dir).await?;
        }
        fs::create_dir_all(&target_dir).await?;

        // Extract
        println!("Extracting...");
        let cursor = Cursor::new(bytes);
        let mut archive = zip::ZipArchive::new(cursor)?;

        // We extract everything into target_dir.
        // Note: Adoptium zips usually contain a root folder like 'jdk-17.0.1+12'
        archive.extract(&target_dir)?;

        println!("Java {} installed successfully.", version);
        Ok(())
    }

    async fn get_adoptium_download_url(&self, version: u8) -> Result<String> {
        let os = if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "macos") {
            "mac"
        } else {
            "linux"
        };
        let arch = "x64"; // Assuming x64 for now, could detect with cfg! later

        let api_url = format!(
            "https://api.adoptium.net/v3/assets/feature_releases/{}/ga?architecture={}&heap_size=normal&image_type=jdk&jvm_impl=hotspot&os={}&vendor=eclipse",
            version, arch, os
        );

        let client = Client::builder()
            .user_agent("MinecraftServerManager/0.1.0")
            .build()?;

        let resp: serde_json::Value = client.get(&api_url).send().await?.json().await?;

        // Parse response to find the download link
        let releases = resp
            .as_array()
            .context("Invalid response from Adoptium API")?;
        let first_release = releases
            .first()
            .context("No releases found for this version")?;

        let binaries = first_release["binaries"]
            .as_array()
            .context("No binaries found")?;
        let binary = binaries.first().context("No binary details found")?;

        let package = &binary["package"];
        let link = package["link"].as_str().context("No download link found")?;

        Ok(link.to_string())
    }

    /// Heuristic to determine Java version from Minecraft version
    pub fn get_java_version_for_mc(mc_version: &str) -> u8 {
        // Simple parsing of "1.X.Y"
        let parts: Vec<&str> = mc_version.split('.').collect();
        if parts.len() < 2 {
            return 21; // Fallback to latest
        }

        let minor = parts[1].parse::<u32>().unwrap_or(0);
        let patch = if parts.len() > 2 {
            parts[2].parse::<u32>().unwrap_or(0)
        } else {
            0
        };

        if minor >= 20 {
            if minor == 20 && patch < 5 {
                return 17; // 1.20.0 - 1.20.4 -> Java 17
            }
            // 1.20.5+ -> Java 21
            return 21;
        } else if minor >= 18 {
            return 17; // 1.18+ -> Java 17
        } else if minor == 17 {
            return 17; // 1.17 -> Java 16/17
        } else if minor >= 16 {
            // 1.16.5 needed Java 8 or 11, technically 8 is standard for modpacks, 11 for vanilla.
            // Let's stick to 8 for compatibility with old Forge unless user overrides.
            // Actually newer Paper 1.16 supports 11+.
            // Safe bet for pre-1.17 is Java 8.
            return 8;
        }

        8 // Default for old versions
    }
}
