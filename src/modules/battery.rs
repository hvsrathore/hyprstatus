#![allow(dead_code)]
use crate::config::{parse_hex_color, Config};
use crate::modules::ModuleBlock;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatteryStatus {
    Charging,
    Discharging,
    Full,
    NotCharging,
    ACOnly,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct BatteryMetrics {
    pub status: BatteryStatus,
    pub capacity: u32,
    pub power_watts: f32,
    pub time_remaining_mins: Option<u32>,
    pub is_ac_online: bool,
}

impl Default for BatteryMetrics {
    fn default() -> Self {
        Self {
            status: BatteryStatus::Unknown,
            capacity: 100,
            power_watts: 0.0,
            time_remaining_mins: None,
            is_ac_online: true,
        }
    }
}

pub struct BatteryModule {
    metrics: Arc<RwLock<BatteryMetrics>>,
    notified_low: Arc<AtomicBool>,
}

impl BatteryModule {
    pub fn new() -> Self {
        let metrics = Arc::new(RwLock::new(BatteryMetrics::default()));
        let notified_low = Arc::new(AtomicBool::new(false));
        let module = Self {
            metrics: metrics.clone(),
            notified_low: notified_low.clone(),
        };

        module.spawn_monitor(metrics, notified_low);
        module
    }

    fn spawn_monitor(&self, metrics: Arc<RwLock<BatteryMetrics>>, notified_low: Arc<AtomicBool>) {
        tokio::spawn(async move {
            loop {
                let m = Self::sample_metrics();

                // Trigger low battery warning notification once if below 15% and discharging
                if m.status == BatteryStatus::Discharging && m.capacity < 15 {
                    if !notified_low.swap(true, Ordering::Relaxed) {
                        let _ = tokio::process::Command::new("notify-send")
                            .args([
                                "-u",
                                "critical",
                                "-a",
                                "hyprstatus",
                                "Low Battery Warning",
                                &format!("Battery is at {}%! Please connect AC adapter.", m.capacity),
                            ])
                            .spawn();
                    }
                } else if m.status == BatteryStatus::Charging || m.capacity >= 20 {
                    notified_low.store(false, Ordering::Relaxed);
                }

                {
                    let mut guard = metrics.write().unwrap();
                    *guard = m;
                }

                tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
            }
        });
    }

    fn sample_metrics() -> BatteryMetrics {
        let base = Path::new("/sys/class/power_supply");
        let mut bat_path: Option<PathBuf> = None;
        let mut ac_online = false;

        if let Ok(entries) = fs::read_dir(base) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with("BAT") {
                    bat_path = Some(entry.path());
                } else if name.starts_with("AC") || name == "ADP1" {
                    let online_p = entry.path().join("online");
                    if let Ok(val) = fs::read_to_string(online_p) {
                        ac_online = val.trim() == "1";
                    }
                }
            }
        }

        let Some(p) = bat_path else {
            return BatteryMetrics {
                status: BatteryStatus::ACOnly,
                capacity: 100,
                power_watts: 0.0,
                time_remaining_mins: None,
                is_ac_online: ac_online,
            };
        };

        let read_num = |file: &str| -> Option<f32> {
            fs::read_to_string(p.join(file)).ok()?.trim().parse::<f32>().ok()
        };

        let capacity = read_num("capacity").unwrap_or(100.0) as u32;

        let status_str = fs::read_to_string(p.join("status")).unwrap_or_default();
        let status = match status_str.trim() {
            "Charging" => BatteryStatus::Charging,
            "Discharging" => BatteryStatus::Discharging,
            "Full" => BatteryStatus::Full,
            "Not charging" => BatteryStatus::NotCharging,
            _ => BatteryStatus::Unknown,
        };

        let energy_now = read_num("energy_now").or_else(|| read_num("charge_now")).unwrap_or(0.0);
        let power_now = read_num("power_now").or_else(|| {
            let cur = read_num("current_now")?;
            let vol = read_num("voltage_now")?;
            Some((cur * vol) / 1_000_000.0)
        }).unwrap_or(0.0);

        let power_watts = power_now / 1_000_000.0;

        let time_remaining_mins = if power_now > 10_000.0 {
            if status == BatteryStatus::Discharging {
                Some(((energy_now / power_now) * 60.0) as u32)
            } else if status == BatteryStatus::Charging {
                let energy_full = read_num("energy_full").or_else(|| read_num("charge_full")).unwrap_or(energy_now);
                let diff = (energy_full - energy_now).max(0.0);
                Some(((diff / power_now) * 60.0) as u32)
            } else {
                None
            }
        } else {
            None
        };

        BatteryMetrics {
            status,
            capacity,
            power_watts,
            time_remaining_mins,
            is_ac_online: ac_online,
        }
    }

    pub fn render(&self, config: &Config) -> ModuleBlock {
        let m = self.metrics.read().map(|g| g.clone()).unwrap_or_default();

        if m.status == BatteryStatus::ACOnly {
            let fg = parse_hex_color(&config.colors.battery_good);
            let bg = parse_hex_color(&config.colors.block_background);
            let text = "󰚥 AC".to_string();
            let mut block = ModuleBlock::new("battery", &text, fg, bg);
            block.icon = "󰚥".to_string();
            return block;
        }

        let (icon, fg) = match m.status {
            BatteryStatus::Charging => ("󰂄", parse_hex_color(&config.colors.battery_good)),
            BatteryStatus::Full => ("󰁹", parse_hex_color(&config.colors.battery_good)),
            BatteryStatus::Discharging | BatteryStatus::NotCharging | BatteryStatus::Unknown => {
                if m.capacity >= 80 {
                    ("󰁹", parse_hex_color(&config.colors.battery_good))
                } else if m.capacity >= 60 {
                    ("󰂀", parse_hex_color(&config.colors.battery_good))
                } else if m.capacity >= 40 {
                    ("󰁾", parse_hex_color(&config.colors.battery_warning))
                } else if m.capacity >= 15 {
                    ("󰁼", parse_hex_color(&config.colors.battery_warning))
                } else {
                    ("󰂃", parse_hex_color(&config.colors.battery_critical))
                }
            }
            BatteryStatus::ACOnly => ("󰚥", parse_hex_color(&config.colors.battery_good)),
        };

        let time_str = if let Some(mins) = m.time_remaining_mins {
            format!(" ({}h{}m)", mins / 60, mins % 60)
        } else {
            String::new()
        };

        let text = format!("{} {}%{}", icon, m.capacity, time_str);
        let bg = parse_hex_color(&config.colors.block_background);

        let mut block = ModuleBlock::new("battery", &text, fg, bg);
        block.icon = icon.to_string();
        block
    }
}
