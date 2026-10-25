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

pub struct NetworkModule {
    state: Arc<RwLock<NetState>>,
}

impl NetworkModule {
    pub fn new() -> Self {
        let state = Arc::new(RwLock::new(NetState::default()));
        let module = Self { state: state.clone() };

        module.spawn_monitor(state);
        module
    }

    fn spawn_monitor(&self, state: Arc<RwLock<NetState>>) {
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
                        current_state.net_type = NetType::Wifi;
                        // Query Wi-Fi info via iw or /proc/net/wireless
                        let (ssid, dbm, pct) = Self::query_wifi_details(&iface);
                        current_state.ssid = ssid;
                        current_state.signal_dbm = dbm;
                        current_state.signal_percent = pct;
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
        let pct = (((signal_dbm + 100) * 2).clamp(0, 100)) as u32;
        (if ssid.is_empty() { "Wi-Fi".to_string() } else { ssid }, signal_dbm, pct)
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
}
