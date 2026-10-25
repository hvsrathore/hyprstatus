use crate::config::{parse_hex_color, Config};
use crate::modules::{ClickAction, ClickRegion, ModuleBlock};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::RwLock;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetType {
    Wifi,
    Ethernet,
    Disconnected,
}

#[derive(Debug, Clone)]
pub struct NetState {
    pub net_type: NetType,
    pub interface: String,
    pub ssid: String,
    pub signal_dbm: i32,
    pub signal_percent: u32,
    pub rx_bytes_sec: u64,
    pub tx_bytes_sec: u64,
}

impl Default for NetState {
    fn default() -> Self {
        Self {
            net_type: NetType::Disconnected,
            interface: String::new(),
            ssid: String::new(),
            signal_dbm: -100,
            signal_percent: 0,
            rx_bytes_sec: 0,
            tx_bytes_sec: 0,
        }
    }
}

#[derive(Clone)]
pub struct NetworkModule {
    state: Arc<RwLock<NetState>>,
    cached_networks: Arc<RwLock<Vec<crate::modules::popup::WifiNetworkItem>>>,
    wifi_enabled: Arc<std::sync::atomic::AtomicBool>,
    status_message: Arc<RwLock<Option<String>>>,
}

impl NetworkModule {
    pub fn new() -> Self {
        let state = Arc::new(RwLock::new(NetState::default()));
        let cached_networks = Arc::new(RwLock::new(Self::query_wifi_networks_sync()));
        let wifi_enabled = Arc::new(std::sync::atomic::AtomicBool::new(Self::is_wifi_enabled_sync()));
        let status_message = Arc::new(RwLock::new(None));
        let module = Self {
            state: state.clone(),
            cached_networks: cached_networks.clone(),
            wifi_enabled: wifi_enabled.clone(),
            status_message: status_message.clone(),
        };

        module.spawn_monitor(state, cached_networks, wifi_enabled);
        module
    }

    pub fn get_state(&self) -> NetState {
        self.state.try_read().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn get_cached_networks(&self) -> Vec<crate::modules::popup::WifiNetworkItem> {
        self.cached_networks.try_read().map(|n| n.clone()).unwrap_or_default()
    }

    pub fn get_wifi_enabled(&self) -> bool {
        self.wifi_enabled.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn get_status_message(&self) -> Option<String> {
        self.status_message.try_read().ok().and_then(|m| m.clone())
    }

    pub fn take_status_message(&self) -> Option<String> {
        if let Ok(mut lock) = self.status_message.try_write() {
            lock.take()
        } else {
            None
        }
    }

    pub fn set_status_message(&self, msg: Option<String>) {
        if let Ok(mut lock) = self.status_message.try_write() {
            *lock = msg;
        }
    }

    pub async fn set_status_message_async(&self, msg: Option<String>) {
        let mut lock = self.status_message.write().await;
        *lock = msg;
    }

    pub async fn refresh_networks(&self) {
        let is_enabled = Self::is_wifi_enabled().await;
        self.wifi_enabled.store(is_enabled, std::sync::atomic::Ordering::Relaxed);
        if is_enabled {
            let fresh = Self::query_wifi_networks().await;
            let mut cn = self.cached_networks.write().await;
            *cn = fresh;
        } else {
            let mut cn = self.cached_networks.write().await;
            cn.clear();
        }
    }

    pub async fn trigger_rescan(&self) {
        Self::rescan_wifi().await;
        tokio::time::sleep(tokio::time::Duration::from_millis(600)).await;
        self.refresh_networks().await;
    }

    pub fn get_saved_connections_sync() -> std::collections::HashSet<String> {
        let mut saved = std::collections::HashSet::new();
        if let Ok(output) = std::process::Command::new("nmcli")
            .args(["-t", "-f", "NAME,TYPE", "connection", "show"])
            .output()
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let parts: Vec<&str> = line.split(':').collect();
                    if parts.len() >= 2 {
                        let name = parts[0].trim();
                        let conn_type = parts[1].trim();
                        if conn_type == "802-11-wireless" || conn_type == "wifi" {
                            saved.insert(name.to_string());
                        }
                    } else if !line.trim().is_empty() {
                        saved.insert(line.trim().to_string());
                    }
                }
            }
        }
        saved
    }

    pub async fn get_saved_connections() -> std::collections::HashSet<String> {
        let mut saved = std::collections::HashSet::new();
        if let Ok(output) = tokio::process::Command::new("nmcli")
            .args(["-t", "-f", "NAME,TYPE", "connection", "show"])
            .output()
            .await
        {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let parts: Vec<&str> = line.split(':').collect();
                    if parts.len() >= 2 {
                        let name = parts[0].trim();
                        let conn_type = parts[1].trim();
                        if conn_type == "802-11-wireless" || conn_type == "wifi" {
                            saved.insert(name.to_string());
                        }
                    } else if !line.trim().is_empty() {
                        saved.insert(line.trim().to_string());
                    }
                }
            }
        }
        saved
    }

    pub fn query_wifi_networks_sync() -> Vec<crate::modules::popup::WifiNetworkItem> {
        let saved_connections = Self::get_saved_connections_sync();
        let mut list = Vec::new();
        if let Ok(output) = std::process::Command::new("nmcli")
            .args(["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "dev", "wifi", "list", "--rescan", "auto"])
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout);
            let mut seen_ssids = std::collections::HashSet::new();
            for line in text.lines() {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 4 {
                    let in_use = parts[0] == "*";
                    let ssid = parts[1].trim();
                    if ssid.is_empty() || seen_ssids.contains(ssid) {
                        continue;
                    }
                    seen_ssids.insert(ssid.to_string());
                    let signal = parts[2].parse::<u32>().unwrap_or(0);
                    let security = parts[3].trim().to_string();
                    let is_saved = saved_connections.contains(ssid);
                    list.push(crate::modules::popup::WifiNetworkItem {
                        in_use,
                        ssid: ssid.to_string(),
                        signal,
                        security,
                        is_saved,
                    });
                }
            }
        }
        list
    }

    pub fn is_wifi_enabled_sync() -> bool {
        if let Ok(output) = std::process::Command::new("nmcli")
            .args(["radio", "wifi"])
            .output()
        {
            let text = String::from_utf8_lossy(&output.stdout);
            text.trim() == "enabled"
        } else {
            true
        }
    }

    pub async fn query_wifi_networks() -> Vec<crate::modules::popup::WifiNetworkItem> {
        let saved_connections = Self::get_saved_connections().await;
        let mut list = Vec::new();
        let output = tokio::process::Command::new("nmcli")
            .args(["-t", "-f", "IN-USE,SSID,SIGNAL,SECURITY", "dev", "wifi", "list", "--rescan", "auto"])
            .output()
            .await;
        if let Ok(o) = output {
            let text = String::from_utf8_lossy(&o.stdout);
            let mut seen_ssids = std::collections::HashSet::new();
            for line in text.lines() {
                let parts: Vec<&str> = line.split(':').collect();
                if parts.len() >= 4 {
                    let in_use = parts[0] == "*";
                    let ssid = parts[1].trim();
                    if ssid.is_empty() || seen_ssids.contains(ssid) {
                        continue;
                    }
                    seen_ssids.insert(ssid.to_string());
                    let signal = parts[2].parse::<u32>().unwrap_or(0);
                    let security = parts[3].trim().to_string();
                    let is_saved = saved_connections.contains(ssid);
                    list.push(crate::modules::popup::WifiNetworkItem {
                        in_use,
                        ssid: ssid.to_string(),
                        signal,
                        security,
                        is_saved,
                    });
                }
            }
        }
        list
    }

    pub async fn is_wifi_enabled() -> bool {
        let output = tokio::process::Command::new("nmcli")
            .args(["radio", "wifi"])
            .output()
            .await;
        if let Ok(o) = output {
            let text = String::from_utf8_lossy(&o.stdout);
            text.trim() == "enabled"
        } else {
            true
        }
    }

    pub async fn toggle_wifi_radio() {
        let enabled = Self::is_wifi_enabled().await;
        let arg = if enabled { "off" } else { "on" };
        let _ = tokio::process::Command::new("nmcli")
            .args(["radio", "wifi", arg])
            .output()
            .await;
    }

    pub async fn rescan_wifi() {
        let _ = tokio::process::Command::new("nmcli")
            .args(["dev", "wifi", "rescan"])
            .output()
            .await;
    }

    pub async fn connect_wifi(ssid: &str) -> Result<(), String> {
        let output = tokio::process::Command::new("nmcli")
            .args(["dev", "wifi", "connect", ssid])
            .output()
            .await;
        match output {
            Ok(o) if o.status.success() => Ok(()),
            Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
            Err(e) => Err(e.to_string()),
        }
    }

    pub async fn forget_wifi(ssid: &str) -> Result<(), String> {
        let output = tokio::process::Command::new("nmcli")
            .args(["connection", "delete", "id", ssid])
            .output()
            .await;
        match output {
            Ok(o) if o.status.success() => Ok(()),
            Ok(o) => {
                let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
                // Also attempt deleting by UUID in case name lookup had special characters or multiple profiles
                if let Ok(list_out) = tokio::process::Command::new("nmcli")
                    .args(["-t", "-f", "UUID,NAME", "connection", "show"])
                    .output()
                    .await
                {
                    let list_str = String::from_utf8_lossy(&list_out.stdout);
                    let mut deleted_any = false;
                    for line in list_str.lines() {
                        let parts: Vec<&str> = line.split(':').collect();
                        if parts.len() >= 2 && parts[1].trim() == ssid {
                            let uuid = parts[0].trim();
                            let _ = tokio::process::Command::new("nmcli")
                                .args(["connection", "delete", "uuid", uuid])
                                .output()
                                .await;
                            deleted_any = true;
                        }
                    }
                    if deleted_any {
                        return Ok(());
                    }
                }
                Err(err)
            }
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn is_connection_saved(ssid: &str) -> bool {
        Self::get_saved_connections_sync().contains(ssid)
    }

    pub async fn connect_wifi_with_password(ssid: &str, password: &str) -> Result<(), String> {
        let output = tokio::process::Command::new("nmcli")
            .args(["dev", "wifi", "connect", ssid, "password", password])
            .output()
            .await;
        match output {
            Ok(o) if o.status.success() => Ok(()),
            Ok(o) => Err(String::from_utf8_lossy(&o.stderr).trim().to_string()),
            Err(e) => Err(e.to_string()),
        }
    }

    pub async fn prompt_wifi_password(ssid: &str) -> Option<String> {
        let prompt_text = format!("Password for {}:", ssid);
        let output = tokio::process::Command::new("wofi")
            .args([
                "--dmenu",
                "--password",
                "--prompt",
                &prompt_text,
                "--lines",
                "0",
                "--width",
                "360",
                "--height",
                "60",
            ])
            .output()
            .await;

        if let Ok(out) = output {
            if out.status.success() {
                let pass = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !pass.is_empty() {
                    return Some(pass);
                }
            }
        }

        // Fallback to rofi if wofi is unavailable
        let rofi_out = tokio::process::Command::new("rofi")
            .args(["-dmenu", "-password", "-p", &prompt_text, "-lines", "0"])
            .output()
            .await;

        if let Ok(out) = rofi_out {
            if out.status.success() {
                let pass = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !pass.is_empty() {
                    return Some(pass);
                }
            }
        }

        None
    }

    pub async fn disconnect_wifi(ssid: &str) {
        let _ = tokio::process::Command::new("nmcli")
            .args(["connection", "down", "id", ssid])
            .output()
            .await;
    }

    fn spawn_monitor(
        &self,
        state: Arc<RwLock<NetState>>,
        cached_networks: Arc<RwLock<Vec<crate::modules::popup::WifiNetworkItem>>>,
        wifi_enabled: Arc<std::sync::atomic::AtomicBool>,
    ) {
        tokio::spawn(async move {
            let mut last_sample_time = Instant::now();
            let mut last_rx: HashMap<String, u64> = HashMap::new();
            let mut last_tx: HashMap<String, u64> = HashMap::new();

            loop {
                let now = Instant::now();
                let dt = now.duration_since(last_sample_time).as_secs_f64().max(0.1);
                last_sample_time = now;

                // 1. Read /proc/net/dev for throughput
                let (current_rx, current_tx) = Self::read_net_dev();

                // 2. Find active default or up interface
                let active_iface = Self::find_active_interface();

                let mut current_state = NetState::default();

                if let Some(iface) = active_iface {
                    current_state.interface = iface.clone();

                    // Calculate speed for this interface
                    let cur_rx_bytes = current_rx.get(&iface).copied().unwrap_or(0);
                    let cur_tx_bytes = current_tx.get(&iface).copied().unwrap_or(0);

                    if let (Some(&prev_rx), Some(&prev_tx)) = (last_rx.get(&iface), last_tx.get(&iface)) {
                        let rx_diff = cur_rx_bytes.saturating_sub(prev_rx);
                        let tx_diff = cur_tx_bytes.saturating_sub(prev_tx);
                        current_state.rx_bytes_sec = (rx_diff as f64 / dt) as u64;
                        current_state.tx_bytes_sec = (tx_diff as f64 / dt) as u64;
                    }

                    if iface.starts_with("wl") || Path::new(&format!("/sys/class/net/{}/wireless", iface)).exists() {
                        // Query Wi-Fi info via iw or /proc/net/wireless
                        let (ssid, dbm, pct) = Self::query_wifi_details(&iface);
                        if ssid.is_empty() {
                            current_state.net_type = NetType::Disconnected;
                            current_state.ssid = String::new();
                            current_state.signal_dbm = -100;
                            current_state.signal_percent = 0;
                        } else {
                            current_state.net_type = NetType::Wifi;
                            current_state.ssid = ssid;
                            current_state.signal_dbm = dbm;
                            current_state.signal_percent = pct;
                        }
                    } else if iface.starts_with("en") || iface.starts_with("eth") {
                        current_state.net_type = NetType::Ethernet;
                    } else {
                        current_state.net_type = NetType::Ethernet;
                    }
                } else {
                    current_state.net_type = NetType::Disconnected;
                }

                last_rx = current_rx;
                last_tx = current_tx;

                {
                    let mut st = state.write().await;
                    *st = current_state;
                }

                // Check wifi radio & refresh cached networks
                let is_enabled = Self::is_wifi_enabled_sync();
                wifi_enabled.store(is_enabled, std::sync::atomic::Ordering::Relaxed);
                if is_enabled {
                    let fresh = Self::query_wifi_networks_sync();
                    let mut cn = cached_networks.write().await;
                    *cn = fresh;
                } else {
                    let mut cn = cached_networks.write().await;
                    cn.clear();
                }

                tokio::time::sleep(tokio::time::Duration::from_millis(1500)).await;
            }
        });
    }

    fn read_net_dev() -> (HashMap<String, u64>, HashMap<String, u64>) {
        let mut rx_map = HashMap::new();
        let mut tx_map = HashMap::new();

        if let Ok(content) = fs::read_to_string("/proc/net/dev") {
            for line in content.lines().skip(2) {
                if let Some((iface_part, stats_part)) = line.split_once(':') {
                    let iface = iface_part.trim().to_string();
                    let fields: Vec<&str> = stats_part.split_whitespace().collect();
                    if fields.len() >= 9 {
                        if let (Ok(rx), Ok(tx)) = (fields[0].parse::<u64>(), fields[8].parse::<u64>()) {
                            rx_map.insert(iface.clone(), rx);
                            tx_map.insert(iface, tx);
                        }
                    }
                }
            }
        }

        (rx_map, tx_map)
    }

    fn find_active_interface() -> Option<String> {
        let net_dir = Path::new("/sys/class/net");
        if let Ok(entries) = fs::read_dir(net_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name == "lo" {
                    continue;
                }

                let operstate_path = entry.path().join("operstate");
                let carrier_path = entry.path().join("carrier");

                let operstate = fs::read_to_string(operstate_path).unwrap_or_default();
                let carrier = fs::read_to_string(carrier_path).unwrap_or_default();

                if operstate.trim() == "up" || carrier.trim() == "1" {
                    return Some(name);
                }
            }
        }
        None
    }

    fn query_wifi_details(iface: &str) -> (String, i32, u32) {
        // Try iw dev <iface> link
        let mut ssid = String::new();
        let mut signal_dbm = -65;

        if let Ok(out) = std::process::Command::new("iw")
            .args(["dev", iface, "link"])
            .output()
        {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                for line in text.lines() {
                    let line = line.trim();
                    if line.starts_with("SSID:") {
                        ssid = line.trim_start_matches("SSID:").trim().to_string();
                    } else if line.starts_with("signal:") {
                        let parts: Vec<&str> = line.split_whitespace().collect();
                        if parts.len() >= 2 {
                            if let Ok(val) = parts[1].parse::<i32>() {
                                signal_dbm = val;
                            }
                        }
                    }
                }
            }
        }

        if ssid.is_empty() {
            // Fallback to nmcli
            if let Ok(out) = std::process::Command::new("nmcli")
                .args(["-t", "-f", "active,ssid,signal", "dev", "wifi"])
                .output()
            {
                if out.status.success() {
                    let text = String::from_utf8_lossy(&out.stdout);
                    for line in text.lines() {
                        if line.starts_with("yes:") {
                            let parts: Vec<&str> = line.split(':').collect();
                            if parts.len() >= 3 {
                                ssid = parts[1].to_string();
                                if let Ok(sig_pct) = parts[2].parse::<u32>() {
                                    return (ssid, -100 + (sig_pct as i32 / 2), sig_pct);
                                }
                            }
                        }
                    }
                }
            }
        }

        // Convert dBm to percentage roughly (-100 to -50)
        let pct = if ssid.is_empty() {
            0
        } else {
            (((signal_dbm + 100) * 2).clamp(0, 100)) as u32
        };
        (ssid, signal_dbm, pct)
    }

    fn format_speed(bytes_per_sec: u64) -> String {
        if bytes_per_sec >= 1_000_000 {
            format!("{:.1}M", bytes_per_sec as f64 / 1_000_000.0)
        } else if bytes_per_sec >= 1_000 {
            format!("{}K", bytes_per_sec / 1_000)
        } else {
            format!("{}B", bytes_per_sec)
        }
    }

    pub fn render(&self, config: &Config) -> ModuleBlock {
        let st = {
            if let Ok(guard) = self.state.try_read() {
                guard.clone()
            } else {
                NetState::default()
            }
        };

        let (icon, text) = match st.net_type {
            NetType::Wifi => {
                let ic = if st.signal_percent >= 80 {
                    "󰤨"
                } else if st.signal_percent >= 60 {
                    "󰤥"
                } else if st.signal_percent >= 40 {
                    "󰤢"
                } else if st.signal_percent >= 20 {
                    "󰤟"
                } else {
                    "󰤮"
                };
                let speed_str = if st.rx_bytes_sec > 1000 {
                    format!(" {}↓", Self::format_speed(st.rx_bytes_sec))
                } else {
                    String::new()
                };
                (ic, format!("{} {}{}", ic, st.ssid, speed_str))
            }
            NetType::Ethernet => {
                let speed_str = if st.rx_bytes_sec > 1000 {
                    format!(" {}↓", Self::format_speed(st.rx_bytes_sec))
                } else {
                    String::new()
                };
                ("󰈀", format!("󰈀 {}{}", st.interface, speed_str))
            }
            NetType::Disconnected => ("󰤮", "󰤮 Offline".to_string()),
        };

        let fg = parse_hex_color(&config.colors.network);
        let bg = parse_hex_color(&config.colors.block_background);

        let mut block = ModuleBlock::new("network", &text, fg, bg);
        block.icon = icon.to_string();
        block.click_regions.push(ClickRegion {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            action: ClickAction::NetworkClick,
        });

        block
    }

    pub async fn open_wifi_modal() {
        use tokio::process::Command;

        let has_wofi = Command::new("which").arg("wofi").output().await.map(|o| o.status.success()).unwrap_or(false);
        let has_rofi = Command::new("which").arg("rofi").output().await.map(|o| o.status.success()).unwrap_or(false);

        if !has_wofi && !has_rofi {
            let _ = Command::new("nm-connection-editor").spawn();
            return;
        }

        let wifi_status = Command::new("nmcli")
            .args(["radio", "wifi"])
            .output()
            .await
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|_| "enabled".to_string());

        let is_wifi_enabled = wifi_status == "enabled";

        struct WifiEntry {
            in_use: bool,
            ssid: String,
            signal: u32,
            security: String,
            bars: String,
        }

        let mut networks: Vec<WifiEntry> = Vec::new();
        let mut seen_ssids = std::collections::HashSet::new();

        if is_wifi_enabled {
            if let Ok(out) = Command::new("nmcli")
                .args(["--terse", "--fields", "IN-USE,SSID,SIGNAL,SECURITY,BARS", "dev", "wifi", "list"])
                .output()
                .await
            {
                let text = String::from_utf8_lossy(&out.stdout);
                for line in text.lines() {
                    let parts: Vec<&str> = line.split(':').collect();
                    if parts.len() >= 5 {
                        let in_use = parts[0] == "*";
                        let ssid = parts[1].trim().to_string();
                        if ssid.is_empty() {
                            continue;
                        }
                        let signal = parts[2].parse::<u32>().unwrap_or(0);
                        let security = parts[3].trim().to_string();
                        let bars = parts[4].trim().to_string();

                        if seen_ssids.insert(ssid.clone()) {
                            networks.push(WifiEntry {
                                in_use,
                                ssid,
                                signal,
                                security,
                                bars,
                            });
                        }
                    }
                }
            }
        }

        networks.sort_by(|a, b| {
            b.in_use.cmp(&a.in_use).then_with(|| b.signal.cmp(&a.signal))
        });

        let mut menu_lines = Vec::new();

        if is_wifi_enabled {
            menu_lines.push("󰤮  Turn Wi-Fi OFF".to_string());
            menu_lines.push("󰑐  Rescan Networks".to_string());
        } else {
            menu_lines.push("󰤨  Turn Wi-Fi ON".to_string());
        }

        for net in &networks {
            let icon = if net.signal >= 80 {
                "󰤨"
            } else if net.signal >= 60 {
                "󰤥"
            } else if net.signal >= 40 {
                "󰤢"
            } else if net.signal >= 20 {
                "󰤟"
            } else {
                "󰤮"
            };

            let status = if net.in_use { " [Connected]" } else { "" };
            let sec = if net.security.is_empty() || net.security == "--" {
                "Open".to_string()
            } else {
                net.security.clone()
            };

            menu_lines.push(format!(
                "{}  {:<22}  {:>3}%  {:<4}  ({}){}",
                icon, net.ssid, net.signal, net.bars, sec, status
            ));
        }

        let input_str = menu_lines.join("\n");

        let choice = if has_wofi {
            Self::run_wofi_dmenu(&input_str, "Wi-Fi Networks", false).await
        } else {
            Self::run_rofi_dmenu(&input_str, "Wi-Fi Networks", false).await
        };

        let selection = match choice {
            Some(s) if !s.trim().is_empty() => s.trim().to_string(),
            _ => return,
        };

        if selection.contains("Turn Wi-Fi OFF") {
            let _ = Command::new("nmcli").args(["radio", "wifi", "off"]).output().await;
            let _ = Command::new("notify-send")
                .args(["-a", "hyprstatus", "Wi-Fi", "Wi-Fi radio disabled"])
                .spawn();
        } else if selection.contains("Turn Wi-Fi ON") {
            let _ = Command::new("nmcli").args(["radio", "wifi", "on"]).output().await;
            let _ = Command::new("notify-send")
                .args(["-a", "hyprstatus", "Wi-Fi", "Wi-Fi radio enabled"])
                .spawn();
        } else if selection.contains("Rescan Networks") {
            let _ = Command::new("nmcli").args(["dev", "wifi", "rescan"]).output().await;
            Box::pin(Self::open_wifi_modal()).await;
        } else {
            for net in &networks {
                if selection.contains(&net.ssid) {
                    if net.in_use {
                        let confirm_input = format!("Disconnect from {}\nCancel", net.ssid);
                        let confirm_choice = if has_wofi {
                            Self::run_wofi_dmenu(&confirm_input, "Disconnect?", false).await
                        } else {
                            Self::run_rofi_dmenu(&confirm_input, "Disconnect?", false).await
                        };
                        if let Some(c) = confirm_choice {
                            if c.trim().starts_with("Disconnect") {
                                let _ = Command::new("nmcli")
                                    .args(["connection", "down", "id", &net.ssid])
                                    .output()
                                    .await;
                                let _ = Command::new("notify-send")
                                    .args(["-a", "hyprstatus", "Wi-Fi", &format!("Disconnected from {}", net.ssid)])
                                    .spawn();
                            }
                        }
                    } else {
                        let is_saved = Command::new("nmcli")
                            .args(["-t", "-f", "NAME", "connection", "show"])
                            .output()
                            .await
                            .map(|o| {
                                let text = String::from_utf8_lossy(&o.stdout);
                                text.lines().any(|l| l.trim() == net.ssid)
                            })
                            .unwrap_or(false);

                        let is_open = net.security.is_empty() || net.security == "--";

                        if is_saved || is_open {
                            let _ = Command::new("notify-send")
                                .args(["-a", "hyprstatus", "Wi-Fi", &format!("Connecting to {}...", net.ssid)])
                                .spawn();
                            let res = Command::new("nmcli")
                                .args(["dev", "wifi", "connect", &net.ssid])
                                .output()
                                .await;
                            match res {
                                Ok(o) if o.status.success() => {
                                    let _ = Command::new("notify-send")
                                        .args(["-a", "hyprstatus", "Wi-Fi", &format!("Connected to {}", net.ssid)])
                                        .spawn();
                                }
                                Ok(o) => {
                                    let err = String::from_utf8_lossy(&o.stderr);
                                    let _ = Command::new("notify-send")
                                        .args(["-a", "hyprstatus", "Wi-Fi Failed", err.trim()])
                                        .spawn();
                                }
                                Err(e) => {
                                    let _ = Command::new("notify-send")
                                        .args(["-a", "hyprstatus", "Wi-Fi Error", &e.to_string()])
                                        .spawn();
                                }
                            }
                        } else {
                            let pwd_choice = if has_wofi {
                                Self::run_wofi_dmenu("", &format!("Password for {}:", net.ssid), true).await
                            } else {
                                Self::run_rofi_dmenu("", &format!("Password for {}:", net.ssid), true).await
                            };

                            if let Some(pwd) = pwd_choice {
                                let pwd = pwd.trim();
                                if !pwd.is_empty() {
                                    let _ = Command::new("notify-send")
                                        .args(["-a", "hyprstatus", "Wi-Fi", &format!("Connecting to {}...", net.ssid)])
                                        .spawn();
                                    let res = Command::new("nmcli")
                                        .args(["dev", "wifi", "connect", &net.ssid, "password", pwd])
                                        .output()
                                        .await;
                                    match res {
                                        Ok(o) if o.status.success() => {
                                            let _ = Command::new("notify-send")
                                                .args(["-a", "hyprstatus", "Wi-Fi", &format!("Connected to {}", net.ssid)])
                                                .spawn();
                                        }
                                        Ok(o) => {
                                            let err = String::from_utf8_lossy(&o.stderr);
                                            let _ = Command::new("notify-send")
                                                .args(["-a", "hyprstatus", "Wi-Fi Failed", err.trim()])
                                                .spawn();
                                        }
                                        Err(e) => {
                                            let _ = Command::new("notify-send")
                                                .args(["-a", "hyprstatus", "Wi-Fi Error", &e.to_string()])
                                                .spawn();
                                        }
                                    }
                                }
                            }
                        }
                    }
                    break;
                }
            }
        }
    }

    async fn run_wofi_dmenu(input: &str, prompt: &str, password: bool) -> Option<String> {
        use tokio::io::AsyncWriteExt;
        use tokio::process::Command;

        let mut cmd = Command::new("wofi");
        cmd.args(["--dmenu", "-p", prompt, "-i", "--width", "480", "--height", if password { "120" } else { "360" }]);
        if password {
            cmd.arg("--password");
        }
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::null());

        let mut child = cmd.spawn().ok()?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(input.as_bytes()).await;
        }
        let output = child.wait_with_output().await.ok()?;
        if output.status.success() {
            Some(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            None
        }
    }

    async fn run_rofi_dmenu(input: &str, prompt: &str, password: bool) -> Option<String> {
        use tokio::io::AsyncWriteExt;
        use tokio::process::Command;

        let mut cmd = Command::new("rofi");
        cmd.args(["-dmenu", "-p", prompt, "-i"]);
        if password {
            cmd.arg("-password");
        }
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::null());

        let mut child = cmd.spawn().ok()?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(input.as_bytes()).await;
        }
        let output = child.wait_with_output().await.ok()?;
        if output.status.success() {
            Some(String::from_utf8_lossy(&output.stdout).to_string())
        } else {
            None
        }
    }
}
