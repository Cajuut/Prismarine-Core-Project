use std::os::windows::process::CommandExt;
use std::process::Stdio;

#[derive(Debug, Clone)]
pub struct JavaInstallation {
    pub path: String,
    pub version: u8, // Major version (8, 17, 21, etc.)
}

/// Find all Java installations on the system
pub fn find_java_installations() -> Vec<JavaInstallation> {
    let mut installations = Vec::new();

    // Check JAVA_HOME first
    if let Ok(java_home) = std::env::var("JAVA_HOME") {
        #[cfg(target_os = "windows")]
        let java_path = format!("{}\\bin\\java.exe", java_home);
        #[cfg(not(target_os = "windows"))]
        let java_path = format!("{}/bin/java", java_home);

        if let Some(version) = get_java_version(&java_path) {
            installations.push(JavaInstallation {
                path: java_path,
                version,
            });
        }
    }

    // Scan common installation directories
    #[cfg(target_os = "windows")]
    {
        let paths = vec![
            "C:\\Program Files\\Java",
            "C:\\Program Files (x86)\\Java",
            "C:\\Program Files\\Eclipse Adoptium",
            "C:\\Program Files\\Zulu",
        ];

        for base_path in paths {
            if let Ok(entries) = std::fs::read_dir(base_path) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let java_exe = entry.path().join("bin\\java.exe");
                    if java_exe.exists() {
                        if let Some(version) = get_java_version(&java_exe.to_string_lossy()) {
                            installations.push(JavaInstallation {
                                path: java_exe.to_string_lossy().to_string(),
                                version,
                            });
                        }
                    }
                }
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Ok(entries) = std::fs::read_dir("/Library/Java/JavaVirtualMachines") {
            for entry in entries.filter_map(|e| e.ok()) {
                let java_bin = entry.path().join("Contents/Home/bin/java");
                if java_bin.exists() {
                    if let Some(version) = get_java_version(&java_bin.to_string_lossy()) {
                        installations.push(JavaInstallation {
                            path: java_bin.to_string_lossy().to_string(),
                            version,
                        });
                    }
                }
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(entries) = std::fs::read_dir("/usr/lib/jvm") {
            for entry in entries.filter_map(|e| e.ok()) {
                let java_bin = entry.path().join("bin/java");
                if java_bin.exists() {
                    if let Some(version) = get_java_version(&java_bin.to_string_lossy()) {
                        installations.push(JavaInstallation {
                            path: java_bin.to_string_lossy().to_string(),
                            version,
                        });
                    }
                }
            }
        }
    }

    // Check for bundled JDK in application data directory
    if let Ok(data_dir) = std::env::var("LOCALAPPDATA") {
        let bundled_path = std::path::PathBuf::from(data_dir)
            .join("MinecraftServerManager")
            .join("jdk");
        if bundled_path.exists() {
            scan_java_in_directory(&bundled_path, &mut installations);
        }
    }

    // Check for bundled JDK in current directory (for portable/dev)
    let local_jdk = std::path::PathBuf::from("jdk");
    if local_jdk.exists() {
        scan_java_in_directory(&local_jdk, &mut installations);
    } else {
        // Create the directory if it doesn't exist to prepare for downloads
        let _ = std::fs::create_dir_all(&local_jdk);
    }

    installations
}

fn scan_java_in_directory(path: &std::path::Path, installations: &mut Vec<JavaInstallation>) {
    #[cfg(target_os = "windows")]
    let java_exe = path.join("bin").join("java.exe");
    #[cfg(not(target_os = "windows"))]
    let java_exe = path.join("bin").join("java");

    if java_exe.exists() {
        if let Some(version) = get_java_version(&java_exe.to_string_lossy()) {
            installations.push(JavaInstallation {
                path: java_exe.to_string_lossy().to_string(),
                version,
            });
        }
    }

    // Also scan subdirectories (for cases where JDK is in a subfolder like jdk-21.0.1/)
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.filter_map(|e| e.ok()) {
            if entry.path().is_dir() {
                #[cfg(target_os = "windows")]
                let sub_java = entry.path().join("bin").join("java.exe");
                #[cfg(not(target_os = "windows"))]
                let sub_java = entry.path().join("bin").join("java");

                if sub_java.exists() {
                    if let Some(version) = get_java_version(&sub_java.to_string_lossy()) {
                        installations.push(JavaInstallation {
                            path: sub_java.to_string_lossy().to_string(),
                            version,
                        });
                    }
                }
            }
        }
    }
}

/// Get Java major version from a java executable
pub fn get_java_version(java_path: &str) -> Option<u8> {
    let mut cmd = std::process::Command::new(java_path);
    cmd.arg("-version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    #[cfg(target_os = "windows")]
    {
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let output = cmd.output().ok()?;

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Parse version from output like:
    // openjdk version "21.0.1" 2023-10-17
    // java version "1.8.0_292"
    for line in stderr.lines() {
        if line.contains("version") {
            if let Some(version_str) = line.split('"').nth(1) {
                // Handle "21.0.1" format
                if let Some(major) = version_str.split('.').next() {
                    if let Ok(ver) = major.parse::<u8>() {
                        return Some(ver);
                    }
                }
                // Handle "1.8.0_292" format (Java 8)
                if version_str.starts_with("1.") {
                    if let Some(minor) = version_str.split('.').nth(1) {
                        if let Ok(ver) = minor.parse::<u8>() {
                            return Some(ver);
                        }
                    }
                }
            }
        }
    }

    None
}

/// Get required Java version for Minecraft version
pub fn get_required_java_version(mc_version: &str) -> u8 {
    // Parse version like "1.21.1" -> [1, 21, 1]
    let parts: Vec<&str> = mc_version.split('.').collect();

    if parts.len() < 2 {
        return 21; // Default to Java 21 for unknown/Manual versions
    }

    let major = parts[0].parse::<u16>().unwrap_or(1);
    let minor = parts[1].parse::<u16>().unwrap_or(0);
    let patch = parts
        .get(2)
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(0);

    // Minecraft version logic
    if major >= 26 || (major == 1 && minor >= 26) {
        25 // 1.26+ or 26.x
    } else if major == 1 {
        if minor >= 21 || (minor == 20 && patch >= 5) {
            21 // 1.20.5+
        } else if minor >= 17 {
            17 // 1.17 - 1.20.4
        } else {
            8 // 1.16.5 and below
        }
    } else {
        21 // Default for other future versions unless explicitly handled
    }
}

/// Select Java by exact major version
pub async fn select_java_by_version(required: u8) -> Option<String> {
    let installations = find_java_installations();
    if let Some(install) = installations.iter().find(|j| j.version == required) {
        return Some(install.path.clone());
    }
    None
}

/// Select best Java for Minecraft version, download if missing
pub async fn select_java_for_minecraft(mc_version: &str) -> Option<String> {
    let required = get_required_java_version(mc_version);
    let installations = find_java_installations();

    // Check if we already have it
    if let Some(install) = installations.iter().find(|j| j.version == required) {
        return Some(install.path.clone());
    }

    // Fallback: check if we have any compatible higher version
    if let Some(install) = installations.iter().filter(|j| j.version > required).min_by_key(|j| j.version) {
        return Some(install.path.clone());
    }

    // NO JAVA FOUND! Try to download
    println!("[Java Selector] No suitable Java found for MC {}. Attempting to download Java {}...", mc_version, required);
    
    match download_and_extract_jdk(required).await {
        Ok(path) => Some(path),
        Err(e) => {
            println!("[Java Selector] Failed to download Java {}: {}", required, e);
            None
        }
    }
}

/// Download and extract JDK from Adoptium
pub async fn download_and_extract_jdk(major_version: u8) -> anyhow::Result<String> {
    let local_jdk_base = std::path::PathBuf::from("jdk");
    let target_dir = local_jdk_base.join(format!("java-{}", major_version));
    
    if target_dir.exists() {
        // Double check if java.exe exists inside
        #[cfg(target_os = "windows")]
        let java_exe = target_dir.join("bin").join("java.exe");
        #[cfg(not(target_os = "windows"))]
        let java_exe = target_dir.join("bin").join("java");
        
        if java_exe.exists() {
            return Ok(java_exe.to_string_lossy().to_string());
        }
    }

    std::fs::create_dir_all(&target_dir)?;

    // Adoptium API URL
    let arch = if cfg!(target_arch = "x86_64") { "x64" } else { "aarch64" };
    let os = if cfg!(target_os = "windows") { "windows" } else if cfg!(target_os = "macos") { "mac" } else { "linux" };
    
    let url = format!(
        "https://api.adoptium.net/v3/binary/latest/{}/ga/{}/{}/jdk/hotspot/normal/eclipse",
        major_version, os, arch
    );

    println!("[JDK Download] Downloading Java {} from {}...", major_version, url);
    
    let client = reqwest::Client::builder()
        .user_agent("MinecraftServerManager/0.1.0")
        .build()?;
        
    let response = client.get(url).send().await?;
    if !response.status().is_success() {
        anyhow::bail!("Failed to download JDK: {}", response.status());
    }

    let bytes = response.bytes().await?;
    let reader = std::io::Cursor::new(bytes);
    
    println!("[JDK Download] Extracting Java {}...", major_version);
    
    let mut archive = zip::ZipArchive::new(reader)?;
    
    // Most Adoptium zips have a single root directory (e.g. jdk-17.0.1+12)
    // We want to extract its contents directly into target_dir or keep the structure
    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let outpath = match file.enclosed_name() {
            Some(path) => {
                // Remove the first component of the path (the root dir in zip)
                let mut components = path.components();
                components.next(); // skip root dir
                target_dir.join(components.as_path())
            },
            None => continue,
        };

        if (*file.name()).ends_with('/') {
            std::fs::create_dir_all(&outpath)?;
        } else {
            if let Some(p) = outpath.parent() {
                if !p.exists() {
                    std::fs::create_dir_all(&p)?;
                }
            }
            let mut outfile = std::fs::File::create(&outpath)?;
            std::io::copy(&mut file, &mut outfile)?;
        }
        
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = file.unix_mode() {
                std::fs::set_permissions(&outpath, std::fs::Permissions::from_mode(mode))?;
            }
        }
    }

    #[cfg(target_os = "windows")]
    let java_exe = target_dir.join("bin").join("java.exe");
    #[cfg(not(target_os = "windows"))]
    let java_exe = target_dir.join("bin").join("java");

    if java_exe.exists() {
        Ok(java_exe.to_string_lossy().to_string())
    } else {
        anyhow::bail!("Failed to find java executable after extraction")
    }
}
