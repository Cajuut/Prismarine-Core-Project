use crate::server_manager::ServerInfo;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct AppConfig {
    pub servers: Vec<ServerInfo>,
    pub base_path: PathBuf,
    pub auto_open_ports: bool,
}

impl AppConfig {
    #[allow(dead_code)]
    pub fn new(base_path: PathBuf) -> Self {
        Self {
            servers: Vec::new(),
            base_path,
            auto_open_ports: true,
        }
    }

    #[allow(dead_code)]
    pub async fn load(config_path: &PathBuf) -> Result<Self> {
        if config_path.exists() {
            let content = fs::read_to_string(config_path).await?;
            let config: AppConfig = serde_json::from_str(&content)?;
            Ok(config)
        } else {
            let default_path = dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("MinecraftServerManager")
                .join("servers");

            Ok(Self::new(default_path))
        }
    }

    #[allow(dead_code)]
    pub async fn save(&self, config_path: &PathBuf) -> Result<()> {
        let content = serde_json::to_string_pretty(self)?;

        if let Some(parent) = config_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        fs::write(config_path, content).await?;
        Ok(())
    }
}
