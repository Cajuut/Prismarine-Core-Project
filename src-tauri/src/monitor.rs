use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::Path;
use sysinfo::{Pid, System};
use tokio::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemStats {
    pub cpu_usage: f32,
    pub memory_used: u64,
    pub memory_total: u64,
    pub memory_percent: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct ServerStats {
    pub cpu_usage: f32,
    pub memory_used: u64,
    pub player_count: u32,
    pub max_players: u32,
}

pub struct Monitor {
    system: System,
}

impl Monitor {
    pub fn new() -> Self {
        let mut sys = System::new_all();
        sys.refresh_all(); // Prime it
        std::thread::sleep(std::time::Duration::from_millis(100));
        sys.refresh_all(); // Refresh again to get initial CPU readings

        Self { system: sys }
    }

    /// Get overall system statistics
    pub fn get_system_stats(&mut self) -> SystemStats {
        // Refresh all components
        self.system.refresh_all();

        let total_memory = self.system.total_memory();
        let used_memory = self.system.used_memory();
        let memory_percent = if total_memory > 0 {
            (used_memory as f32 / total_memory as f32) * 100.0
        } else {
            0.0
        };

        SystemStats {
            cpu_usage: self.system.global_cpu_usage(),
            memory_used: used_memory,
            memory_total: total_memory,
            memory_percent,
        }
    }

    /// Get statistics for a specific server process
    #[allow(dead_code)]
    pub fn get_server_stats(&mut self, pid: u32) -> Option<ServerStats> {
        self.system.refresh_all();

        let sysinfo_pid = Pid::from_u32(pid);

        if let Some(process) = self.system.process(sysinfo_pid) {
            Some(ServerStats {
                cpu_usage: process.cpu_usage(),
                memory_used: process.memory(),
                player_count: 0, // TODO: Implement RCON query
                max_players: 20, // TODO: Read from server.properties
            })
        } else {
            None
        }
    }

    /// Read last N lines from server log
    pub async fn get_server_logs(server_path: &Path, lines: usize) -> Result<Vec<String>> {
        let log_path = server_path.join("logs").join("latest.log");

        if !log_path.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&log_path).await?;
        let all_lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();

        let start = if all_lines.len() > lines {
            all_lines.len() - lines
        } else {
            0
        };

        Ok(all_lines[start..].to_vec())
    }

    /// Parse server.properties to get max players
    #[allow(dead_code)]
    pub async fn get_max_players(server_path: &Path) -> Result<u32> {
        let properties_path = server_path.join("server.properties");

        if !properties_path.exists() {
            return Ok(20); // Default
        }

        let content = fs::read_to_string(&properties_path).await?;

        for line in content.lines() {
            if line.starts_with("max-players=") {
                if let Some(value) = line.split('=').nth(1) {
                    if let Ok(max_players) = value.trim().parse::<u32>() {
                        return Ok(max_players);
                    }
                }
            }
        }

        Ok(20) // Default if not found
    }

    /// Parse server logs to find online players
    pub async fn get_online_players(server_path: &Path) -> Result<Vec<String>> {
        let log_path = server_path.join("logs").join("latest.log");
        if !log_path.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&log_path).await?;
        let mut players = std::collections::HashSet::new();

        // 1.20+ format: [16:32:04] [Server thread/INFO]: PlayerName joined the game
        for line in content.lines() {
            if line.contains(": ") {
                let parts: Vec<&str> = line.split(": ").collect();
                if parts.len() < 2 {
                    continue;
                }
                let message = parts[1];

                if message.contains(" joined the game") {
                    if let Some(name) = message.split(" joined").next() {
                        players.insert(name.trim().to_string());
                    }
                } else if message.contains(" left the game") {
                    if let Some(name) = message.split(" left").next() {
                        players.remove(name.trim());
                    }
                }
            }
        }

        Ok(players.into_iter().collect())
    }
}
