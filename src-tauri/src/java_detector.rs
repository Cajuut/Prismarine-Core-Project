use std::process::Command;

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

    // Fallback: try system java
    if let Some(version) = get_java_version("java") {
        installations.push(JavaInstallation {
            path: "java".to_string(),
            version,
        });
    }

    installations
}

/// Get Java major version from a java executable
pub fn get_java_version(java_path: &str) -> Option<u8> {
    let output = Command::new(java_path).arg("-version").output().ok()?;

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
        return 8; // Default to Java 8
    }

    let major = parts[0].parse::<u16>().unwrap_or(1);
    let minor = parts[1].parse::<u16>().unwrap_or(0);
    let patch = parts
        .get(2)
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(0);

    // Minecraft version logic
    if major == 1 {
        if minor >= 21 || (minor == 20 && patch >= 5) {
            21 // 1.20.5+
        } else if minor >= 17 {
            17 // 1.17 - 1.20.4
        } else {
            8 // 1.16.5 and below
        }
    } else {
        21 // Future versions
    }
}

/// Select best Java for Minecraft version
pub fn select_java_for_minecraft(mc_version: &str) -> Option<String> {
    let required = get_required_java_version(mc_version);
    let installations = find_java_installations();

    println!(
        "[Java Selector] Minecraft {} requires Java {}",
        mc_version, required
    );
    println!(
        "[Java Selector] Found {} Java installations",
        installations.len()
    );

    for install in &installations {
        println!(
            "[Java Selector] - Java {} at {}",
            install.version, install.path
        );
    }

    // Find Java that meets requirements (prefer closest version)
    installations
        .iter()
        .filter(|j| j.version >= required)
        .min_by_key(|j| j.version)
        .map(|j| {
            println!("[Java Selector] Selected: Java {} at {}", j.version, j.path);
            j.path.clone()
        })
}
