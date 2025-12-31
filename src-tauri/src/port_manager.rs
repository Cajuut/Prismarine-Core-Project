use anyhow::{Context, Result};
use reqwest::{Client, Url};
use serde::{Deserialize, Serialize};
use std::net::UdpSocket;
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct PortMapping {
    pub external_port: u16,
    pub internal_port: u16,
    pub description: String,
    pub enabled: bool,
}

// use std::collections::HashMap; // Unused
use std::fs;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedPort {
    pub slot: u8,
    pub port: u16,
    pub protocol: String, // "TCP", "UDP", or "BOTH"
    pub name: String,
    #[serde(default = "default_active")]
    pub active: bool,
}

fn default_active() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct PortConfig {
    ports: Vec<ManagedPort>,
}

pub struct PortManager {
    http_client: Client,
    config_path: std::path::PathBuf,
}

impl PortManager {
    pub fn new() -> Self {
        let config_path = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("MinecraftServerManager")
            .join("managed_ports.json");

        Self {
            http_client: Client::builder()
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
            config_path,
        }
    }

    // --- Managed Port Methods ---

    pub fn get_managed_ports(&self) -> Vec<ManagedPort> {
        self.load_config().unwrap_or_default().ports
    }

    fn load_config(&self) -> Result<PortConfig> {
        if !self.config_path.exists() {
            return Ok(PortConfig::default());
        }
        let data = fs::read_to_string(&self.config_path)?;
        let config: PortConfig = serde_json::from_str(&data)?;
        Ok(config)
    }

    fn save_config(&self, config: &PortConfig) -> Result<()> {
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let data = serde_json::to_string_pretty(config)?;
        fs::write(&self.config_path, data)?;
        Ok(())
    }

    pub async fn open_managed_port(
        &self,
        port: u16,
        protocol: &str,
        name: &str,
        slot: u8,
    ) -> Result<String> {
        let mut config = self.load_config().unwrap_or_default();

        // Check if slot is occupied
        if let Some(pos) = config.ports.iter().position(|p| p.slot == slot) {
            config.ports.remove(pos);
        }

        // For simplicity in this step, we'll try to support single protocol or loop for both.
        let protocols_to_open = if protocol == "BOTH" {
            vec!["TCP", "UDP"]
        } else {
            vec![protocol]
        };

        let description = format!("Prismarine Port {}", slot);

        // --- Actual UPnP / Firewall Call ---
        let local_ip = get_local_ip()?;

        // Try to open ports (ignore errors if router not found, will rely on FW or error later)
        if let Ok(control_url) = self.find_control_url().await {
            for proto in &protocols_to_open {
                let _ = self
                    .add_port_mapping_proto(&control_url, port, &local_ip, &description, proto)
                    .await;
            }
        }

        #[cfg(target_os = "windows")]
        for proto in &protocols_to_open {
            let _ = add_windows_firewall_rule_proto(port, proto);
        }

        // 5. Save to managed list
        config.ports.push(ManagedPort {
            slot,
            port,
            protocol: protocol.to_string(),
            name: name.to_string(),
            active: true,
        });
        self.save_config(&config)?;

        Ok(format!(
            "ポート {} ({}) を開放しました (Slot {})",
            port, protocol, slot
        ))
    }

    pub async fn set_managed_port_active(&self, slot: u8, active: bool) -> Result<String> {
        let mut config = self.load_config().unwrap_or_default();
        let port_idx = config
            .ports
            .iter()
            .position(|p| p.slot == slot)
            .ok_or_else(|| anyhow::anyhow!("Port slot not found"))?;

        // Update state
        config.ports[port_idx].active = active;
        let port = config.ports[port_idx].port;
        let protocol = config.ports[port_idx].protocol.clone();
        let description = format!("Prismarine Port {}", slot);

        self.save_config(&config)?;

        // Action
        let protocols = if protocol == "BOTH" {
            vec!["TCP", "UDP"]
        } else {
            vec![protocol.as_str()]
        };

        if active {
            // OPEN
            let local_ip = get_local_ip()?;
            if let Ok(control_url) = self.find_control_url().await {
                for proto in &protocols {
                    let _ = self
                        .add_port_mapping_proto(&control_url, port, &local_ip, &description, proto)
                        .await;
                }
            }
            #[cfg(target_os = "windows")]
            for proto in &protocols {
                let _ = add_windows_firewall_rule_proto(port, proto);
            }
            Ok("ポートを再開しました".to_string())
        } else {
            // CLOSE
            if let Ok(control_url) = self.find_control_url().await {
                for proto in &protocols {
                    let _ = self
                        .delete_port_mapping_proto(&control_url, port, proto)
                        .await;
                }
            }
            #[cfg(target_os = "windows")]
            for proto in &protocols {
                let _ = remove_windows_firewall_rule_proto(port, proto);
            }
            Ok("ポートを停止しました".to_string())
        }
    }

    pub async fn delete_managed_port(&self, slot: u8) -> Result<()> {
        self.close_managed_port(slot).await
    }

    pub async fn close_managed_port(&self, slot: u8) -> Result<()> {
        let mut config = self.load_config().unwrap_or_default();

        if let Some(index) = config.ports.iter().position(|p| p.slot == slot) {
            let managed_port = config.ports.remove(index);
            self.save_config(&config)?;

            // Close actual upnp/fw
            let protocols_to_close = if managed_port.protocol == "BOTH" {
                vec!["TCP", "UDP"]
            } else {
                vec![managed_port.protocol.as_str()]
            };

            if let Ok(control_url) = self.find_control_url().await {
                for proto in &protocols_to_close {
                    let _ = self
                        .delete_port_mapping_proto(&control_url, managed_port.port, proto)
                        .await;
                }
            }

            #[cfg(target_os = "windows")]
            for proto in &protocols_to_close {
                let _ = remove_windows_firewall_rule_proto(managed_port.port, proto);
            }
        }

        Ok(())
    }

    /// Open a port using Universal UPnP (SSDP + SOAP)
    /// Legacy wrapper for backward compatibility - defaults to TCP
    #[allow(dead_code)]
    pub async fn open_port(&self, port: u16, description: &str) -> Result<String> {
        let local_ip = get_local_ip()?;
        println!("[PortManager] Local IP: {}", local_ip);

        // 1. Discover Router via SSDP
        let control_url = self.find_control_url().await?;
        println!("[PortManager] Control URL: {}", control_url);

        // 2. Send AddPortMapping SOAP Request
        self.add_port_mapping_proto(&control_url, port, &local_ip, description, "TCP")
            .await?;
        println!("[PortManager] Port mapping added via UPnP (TCP)");

        let mut status = "UPnP成功 (Universal/TCP)".to_string();

        // 3. Add Firewall Rule (Windows only)
        #[cfg(target_os = "windows")]
        {
            if let Err(e) = add_windows_firewall_rule_proto(port, "TCP") {
                eprintln!("[PortManager] Firewall rule failed: {}", e);
                status.push_str(" (FW設定失敗)");
            } else {
                status.push_str(" + FW設定完了");
            }
        }

        Ok(status)
    }

    /// Close a port
    /// Legacy wrapper - defaults to TCP
    #[allow(dead_code)]
    pub async fn close_port(&self, port: u16) -> Result<()> {
        if let Ok(control_url) = self.find_control_url().await {
            let _ = self
                .delete_port_mapping_proto(&control_url, port, "TCP")
                .await;
        }

        #[cfg(target_os = "windows")]
        {
            let _ = remove_windows_firewall_rule_proto(port, "TCP");
        }

        Ok(())
    }

    /// Get external IP address
    pub async fn get_external_ip(&self) -> Result<String> {
        if let Ok(control_url) = self.find_control_url().await {
            if let Ok(ip) = self.get_external_ip_upnp(&control_url).await {
                return Ok(ip);
            }
        }

        let ip = reqwest::get("https://api.ipify.org").await?.text().await?;
        Ok(ip)
    }

    /// Check if UPnP is available
    pub async fn is_upnp_available(&self) -> bool {
        self.find_control_url().await.is_ok()
    }

    // --- Private UPnP Methods ---

    async fn find_control_url(&self) -> Result<String> {
        // 1. Try SSDP Discovery
        match self.discover_ssdp().await {
            Ok(location) => {
                println!("[PortManager] SSDP Location: {}", location);
                // Fetch Description XML
                let xml = self.http_client.get(&location).send().await?.text().await?;
                let doc = roxmltree::Document::parse(&xml)
                    .context("Failed to parse router description XML")?;

                let control_path = doc
                    .descendants()
                    .find(|n| {
                        let tag = n.tag_name().name();
                        if tag == "serviceType" {
                            if let Some(text) = n.text() {
                                return text.contains("WANPPPConnection")
                                    || text.contains("WANIPConnection");
                            }
                        }
                        false
                    })
                    .and_then(|n| {
                        n.parent()?
                            .children()
                            .find(|c| c.tag_name().name() == "controlURL")
                    })
                    .and_then(|n| n.text())
                    .context("Could not find WANPPPConnection/WANIPConnection controlURL")?;

                let base_url = Url::parse(&location)?;
                let control_url = base_url.join(control_path)?;
                Ok(control_url.to_string())
            }
            Err(e) => {
                println!("[PortManager] SSDP failed: {}. Trying fallback...", e);
                // 2. Fallback for NEC Routers (Direct Control URL)
                let fallback_url = "http://192.168.0.1:2869/upnp/control/WANPPPConn1";
                if self.http_client.get(fallback_url).send().await.is_ok() {
                    println!("[PortManager] Using NEC Fallback URL");
                    Ok(fallback_url.to_string())
                } else {
                    Err(anyhow::anyhow!("Router not found via SSDP or Fallback"))
                }
            }
        }
    }

    async fn discover_ssdp(&self) -> Result<String> {
        // Bind to the specific local IP to ensure we use the correct interface
        let local_ip = get_local_ip()?;
        let socket =
            UdpSocket::bind(format!("{}:0", local_ip)).or_else(|_| UdpSocket::bind("0.0.0.0:0"))?; // Fallback to 0.0.0.0 if bind fails

        socket.set_read_timeout(Some(Duration::from_secs(4)))?;

        let msg = "M-SEARCH * HTTP/1.1\r\n\
                   HOST: 239.255.255.250:1900\r\n\
                   MAN: \"ssdp:discover\"\r\n\
                   MX: 3\r\n\
                   ST: urn:schemas-upnp-org:service:WANPPPConnection:1\r\n\
                   \r\n";

        println!("[PortManager] Sending SSDP M-SEARCH from {}...", local_ip);
        socket.send_to(msg.as_bytes(), "239.255.255.250:1900")?;

        let mut buf = [0u8; 2048];
        let end_time = std::time::Instant::now() + Duration::from_secs(4);

        while std::time::Instant::now() < end_time {
            if let Ok(amt) = socket.recv(&mut buf) {
                let response = String::from_utf8_lossy(&buf[..amt]);
                for line in response.lines() {
                    if line.to_lowercase().starts_with("location:") {
                        let location = line[9..].trim();
                        println!("[PortManager] SSDP Found: {}", location);
                        return Ok(location.to_string());
                    }
                }
            }
        }

        Err(anyhow::anyhow!("Timed out"))
    }

    async fn add_port_mapping_proto(
        &self,
        control_url: &str,
        port: u16,
        local_ip: &str,
        description: &str,
        protocol: &str,
    ) -> Result<()> {
        let soap_action = "\"urn:schemas-upnp-org:service:WANPPPConnection:1#AddPortMapping\"";
        let body = format!(
            r#"<?xml version="1.0"?>
            <SOAP-ENV:Envelope xmlns:SOAP-ENV="http://schemas.xmlsoap.org/soap/envelope/" SOAP-ENV:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
            <SOAP-ENV:Body>
                <m:AddPortMapping xmlns:m="urn:schemas-upnp-org:service:WANPPPConnection:1">
                    <NewRemoteHost></NewRemoteHost>
                    <NewExternalPort>{}</NewExternalPort>
                    <NewProtocol>{}</NewProtocol>
                    <NewInternalPort>{}</NewInternalPort>
                    <NewInternalClient>{}</NewInternalClient>
                    <NewEnabled>1</NewEnabled>
                    <NewPortMappingDescription>{}</NewPortMappingDescription>
                    <NewLeaseDuration>0</NewLeaseDuration>
                </m:AddPortMapping>
            </SOAP-ENV:Body>
            </SOAP-ENV:Envelope>"#,
            port, protocol, port, local_ip, description
        );

        self.send_soap_request(control_url, soap_action, &body)
            .await
    }

    async fn delete_port_mapping_proto(
        &self,
        control_url: &str,
        port: u16,
        protocol: &str,
    ) -> Result<()> {
        let soap_action = "\"urn:schemas-upnp-org:service:WANPPPConnection:1#DeletePortMapping\"";
        let body = format!(
            r#"<?xml version="1.0"?>
            <SOAP-ENV:Envelope xmlns:SOAP-ENV="http://schemas.xmlsoap.org/soap/envelope/" SOAP-ENV:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
            <SOAP-ENV:Body>
                <m:DeletePortMapping xmlns:m="urn:schemas-upnp-org:service:WANPPPConnection:1">
                    <NewRemoteHost></NewRemoteHost>
                    <NewExternalPort>{}</NewExternalPort>
                    <NewProtocol>{}</NewProtocol>
                </m:DeletePortMapping>
            </SOAP-ENV:Body>
            </SOAP-ENV:Envelope>"#,
            port, protocol
        );

        self.send_soap_request(control_url, soap_action, &body)
            .await
    }

    async fn get_external_ip_upnp(&self, control_url: &str) -> Result<String> {
        let soap_action =
            "\"urn:schemas-upnp-org:service:WANPPPConnection:1#GetExternalIPAddress\"";
        let body = r#"<?xml version="1.0"?>
            <SOAP-ENV:Envelope xmlns:SOAP-ENV="http://schemas.xmlsoap.org/soap/envelope/" SOAP-ENV:encodingStyle="http://schemas.xmlsoap.org/soap/encoding/">
            <SOAP-ENV:Body>
                <m:GetExternalIPAddress xmlns:m="urn:schemas-upnp-org:service:WANPPPConnection:1">
                </m:GetExternalIPAddress>
            </SOAP-ENV:Body>
            </SOAP-ENV:Envelope>"#;

        let response = self
            .http_client
            .post(control_url)
            .header("SOAPAction", soap_action)
            .header("Content-Type", "text/xml; charset=\"utf-8\"")
            .body(body)
            .send()
            .await?
            .text()
            .await?;

        let doc = roxmltree::Document::parse(&response)?;
        let ip = doc
            .descendants()
            .find(|n| n.tag_name().name() == "NewExternalIPAddress")
            .and_then(|n| n.text())
            .context("No IP found")?;

        Ok(ip.to_string())
    }

    async fn send_soap_request(&self, url: &str, action: &str, body: &str) -> Result<()> {
        let response = self
            .http_client
            .post(url)
            .header("SOAPAction", action)
            .header("Content-Type", "text/xml; charset=\"utf-8\"")
            .body(body.to_string())
            .send()
            .await?;

        if !response.status().is_success() {
            let error_text = response.text().await?;
            return Err(anyhow::anyhow!("SOAP Error: {}", error_text));
        }

        Ok(())
    }
}

/// Get local IP address
fn get_local_ip() -> Result<String> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect("8.8.8.8:80")?;
    let local_addr = socket.local_addr()?;
    Ok(local_addr.ip().to_string())
}

#[cfg(target_os = "windows")]
fn add_windows_firewall_rule_proto(port: u16, protocol: &str) -> Result<()> {
    // Protocol must be TCP or UDP
    let cmd = format!(
        "advfirewall firewall add rule name=\"Minecraft Server Port {} ({})\" dir=in action=allow protocol={} localport={}",
        port, protocol, protocol, port
    );

    std::process::Command::new("powershell")
        .args([
            "-Command",
            "Start-Process",
            "netsh",
            "-ArgumentList",
            &format!("'{}'", cmd),
            "-Verb",
            "RunAs",
            "-WindowStyle",
            "Hidden",
        ])
        .output()?;

    Ok(())
}

#[cfg(target_os = "windows")]
fn remove_windows_firewall_rule_proto(port: u16, protocol: &str) -> Result<()> {
    let cmd = format!(
        "advfirewall firewall delete rule name=\"Minecraft Server Port {} ({})\" protocol={} localport={}",
        port, protocol, protocol, port
    );

    std::process::Command::new("powershell")
        .args([
            "-Command",
            "Start-Process",
            "netsh",
            "-ArgumentList",
            &format!("'{}'", cmd),
            "-Verb",
            "RunAs",
            "-WindowStyle",
            "Hidden",
        ])
        .output()?;

    Ok(())
}
