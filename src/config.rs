use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::watch;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub bar: BarConfig,
    #[serde(default)]
    pub colors: ColorTheme,
    #[serde(default)]
    pub layout: LayoutConfig,
    #[serde(default)]
    pub modules: ModulesConfig,
    #[serde(default)]
    pub output: HashMap<String, OutputConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BarConfig {
    #[serde(default = "default_height")]
    pub height: u32,
    #[serde(default = "default_position")]
    pub position: String, // "top" or "bottom"
    #[serde(default = "default_font_name")]
    pub font_name: String,
    #[serde(default = "default_font_path")]
    pub font_path: String,
    #[serde(default = "default_font_size")]
    pub font_size: f32,
    #[serde(default = "default_spacing")]
    pub spacing: u32,
    #[serde(default = "default_padding_horizontal")]
    pub padding_horizontal: u32,
    #[serde(default = "default_padding_vertical")]
    pub padding_vertical: u32,
    #[serde(default = "default_margin_top")]
    pub margin_top: i32,
    #[serde(default = "default_margin_bottom")]
    pub margin_bottom: i32,
    #[serde(default = "default_margin_left")]
    pub margin_left: i32,
    #[serde(default = "default_margin_right")]
    pub margin_right: i32,
    #[serde(default = "default_separator")]
    pub separator: String,
}

impl Default for BarConfig {
    fn default() -> Self {
        Self {
            height: default_height(),
            position: default_position(),
            font_name: default_font_name(),
            font_path: default_font_path(),
            font_size: default_font_size(),
            spacing: default_spacing(),
            padding_horizontal: default_padding_horizontal(),
            padding_vertical: default_padding_vertical(),
            margin_top: default_margin_top(),
            margin_bottom: default_margin_bottom(),
            margin_left: default_margin_left(),
            margin_right: default_margin_right(),
            separator: default_separator(),
        }
    }
}

fn default_height() -> u32 { 30 }
fn default_position() -> String { "top".to_string() }
fn default_font_name() -> String { "JetBrainsMono Nerd Font".to_string() }
fn default_font_path() -> String {
    // Check known font paths
    let paths = [
        "/usr/share/fonts/TTF/JetBrainsMonoNerdFontPropo-Bold.ttf",
        "/usr/share/fonts/TTF/JetBrainsMonoNLNerdFontPropo-Regular.ttf",
        "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
        "/usr/share/fonts/dejavu/DejaVuSansMono.ttf",
    ];
    for p in paths {
        if Path::new(p).exists() {
            return p.to_string();
        }
    }
    "/usr/share/fonts/TTF/JetBrainsMonoNerdFontPropo-Bold.ttf".to_string()
}
fn default_font_size() -> f32 { 13.0 }
fn default_spacing() -> u32 { 6 }
fn default_padding_horizontal() -> u32 { 10 }
fn default_padding_vertical() -> u32 { 3 }
fn default_margin_top() -> i32 { 0 }
fn default_margin_bottom() -> i32 { 0 }
fn default_margin_left() -> i32 { 0 }
fn default_margin_right() -> i32 { 0 }
fn default_separator() -> String { " | ".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorTheme {
    #[serde(default = "default_bg")]
    pub background: String,
    #[serde(default = "default_fg")]
    pub foreground: String,
    #[serde(default = "default_block_bg")]
    pub block_background: String,
    #[serde(default = "default_border")]
    pub border: String,
    #[serde(default = "default_ws_active_bg")]
    pub active_workspace_bg: String,
    #[serde(default = "default_ws_active_fg")]
    pub active_workspace_fg: String,
    #[serde(default = "default_ws_inactive_bg")]
    pub inactive_workspace_bg: String,
    #[serde(default = "default_ws_inactive_fg")]
    pub inactive_workspace_fg: String,
    #[serde(default = "default_ws_urgent_bg")]
    pub urgent_workspace_bg: String,
    #[serde(default = "default_ws_urgent_fg")]
    pub urgent_workspace_fg: String,
    #[serde(default = "default_separator_color")]
    pub separator: String,
    #[serde(default = "default_cpu_color")]
    pub cpu: String,
    #[serde(default = "default_mem_color")]
    pub memory: String,
    #[serde(default = "default_audio_color")]
    pub audio: String,
    #[serde(default = "default_network_color")]
    pub network: String,
    #[serde(default = "default_battery_good")]
    pub battery_good: String,
    #[serde(default = "default_battery_warning")]
    pub battery_warning: String,
    #[serde(default = "default_battery_critical")]
    pub battery_critical: String,
}

impl Default for ColorTheme {
    fn default() -> Self {
        Self {
            background: default_bg(),
            foreground: default_fg(),
            block_background: default_block_bg(),
            border: default_border(),
            active_workspace_bg: default_ws_active_bg(),
            active_workspace_fg: default_ws_active_fg(),
            inactive_workspace_bg: default_ws_inactive_bg(),
            inactive_workspace_fg: default_ws_inactive_fg(),
            urgent_workspace_bg: default_ws_urgent_bg(),
            urgent_workspace_fg: default_ws_urgent_fg(),
            separator: default_separator_color(),
            cpu: default_cpu_color(),
            memory: default_mem_color(),
            audio: default_audio_color(),
            network: default_network_color(),
            battery_good: default_battery_good(),
            battery_warning: default_battery_warning(),
            battery_critical: default_battery_critical(),
        }
    }
}

fn default_bg() -> String { "#1e1e2e".to_string() } // Catppuccin Mocha base
fn default_fg() -> String { "#cdd6f4".to_string() } // Text
fn default_block_bg() -> String { "#313244".to_string() } // Surface0
fn default_border() -> String { "#45475a".to_string() } // Surface1
fn default_ws_active_bg() -> String { "#89b4fa".to_string() } // Blue
fn default_ws_active_fg() -> String { "#11111b".to_string() } // Crust
fn default_ws_inactive_bg() -> String { "#313244".to_string() }
fn default_ws_inactive_fg() -> String { "#a6adc8".to_string() }
fn default_ws_urgent_bg() -> String { "#f38ba8".to_string() } // Red
fn default_ws_urgent_fg() -> String { "#11111b".to_string() }
fn default_separator_color() -> String { "#585b70".to_string() }
fn default_cpu_color() -> String { "#f9e2af".to_string() } // Yellow
fn default_mem_color() -> String { "#a6e3a1".to_string() } // Green
fn default_audio_color() -> String { "#cba6f7".to_string() } // Mauve
fn default_network_color() -> String { "#74c7ec".to_string() } // Sapphire
fn default_battery_good() -> String { "#a6e3a1".to_string() }
fn default_battery_warning() -> String { "#fab387".to_string() } // Peach
fn default_battery_critical() -> String { "#f38ba8".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutConfig {
    #[serde(default = "default_left_modules")]
    pub left: Vec<String>,
    #[serde(default = "default_center_modules")]
    pub center: Vec<String>,
    #[serde(default = "default_right_modules")]
    pub right: Vec<String>,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            left: default_left_modules(),
            center: default_center_modules(),
            right: default_right_modules(),
        }
    }
}

fn default_left_modules() -> Vec<String> {
    vec!["workspaces".to_string(), "window_title".to_string()]
}

fn default_center_modules() -> Vec<String> {
    vec!["perf".to_string()]
}

fn default_right_modules() -> Vec<String> {
    vec![
        "tray".to_string(),
        "network".to_string(),
        "audio".to_string(),
        "battery".to_string(),
        "clock".to_string(),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutputConfig {
    #[serde(default = "default_output_enabled")]
    pub enabled: bool,
    pub scale: Option<f32>,
    pub height: Option<u32>,
    pub left: Option<Vec<String>>,
    pub center: Option<Vec<String>>,
    pub right: Option<Vec<String>>,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            scale: None,
            height: None,
            left: None,
            center: None,
            right: None,
        }
    }
}

fn default_output_enabled() -> bool { true }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModulesConfig {
    #[serde(default)]
    pub audio: AudioConfig,
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default)]
    pub battery: BatteryConfig,
    #[serde(default)]
    pub perf: PerfConfig,
    #[serde(default)]
    pub clock: ClockConfig,
    #[serde(default)]
    pub popup: PopupConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioConfig {
    #[serde(default = "default_step")]
    pub step_percent: u32,
    #[serde(default = "default_true")]
    pub show_mic: bool,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            step_percent: default_step(),
            show_mic: true,
        }
    }
}

fn default_step() -> u32 { 5 }
fn default_true() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkConfig {
    #[serde(default = "default_interface")]
    pub interface: Option<String>,
    #[serde(default = "default_network_format")]
    pub format: String,
    #[serde(default = "default_click_cmd")]
    pub click_command: String,
}

impl Default for NetworkConfig {
    fn default() -> Self {
        Self {
            interface: None,
            format: "{icon} {ssid} {speed}".to_string(),
            click_command: "nmtui".to_string(),
        }
    }
}

fn default_interface() -> Option<String> { None }
fn default_network_format() -> String { "{icon} {ssid}".to_string() }
fn default_click_cmd() -> String { "alacritty -e nmtui".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatteryConfig {
    #[serde(default = "default_battery_format")]
    pub format: String,
    #[serde(default = "default_low_threshold")]
    pub low_threshold: u32,
    #[serde(default = "default_notify_low")]
    pub notify_on_low: bool,
}

impl Default for BatteryConfig {
    fn default() -> Self {
        Self {
            format: "{icon} {percentage}%".to_string(),
            low_threshold: default_low_threshold(),
            notify_on_low: default_notify_low(),
        }
    }
}

fn default_battery_format() -> String { "{icon} {percentage}%".to_string() }
fn default_low_threshold() -> u32 { 15 }
fn default_notify_low() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerfConfig {
    #[serde(default = "default_interval_ms")]
    pub update_interval_ms: u64,
    #[serde(default = "default_sparkline_len")]
    pub sparkline_length: usize,
    #[serde(default = "default_sys_mon_cmd")]
    pub click_command: String,
}

impl Default for PerfConfig {
    fn default() -> Self {
        Self {
            update_interval_ms: default_interval_ms(),
            sparkline_length: default_sparkline_len(),
            click_command: default_sys_mon_cmd(),
        }
    }
}

fn default_interval_ms() -> u64 { 1000 }
fn default_sparkline_len() -> usize { 8 }
fn default_sys_mon_cmd() -> String { "btop".to_string() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClockConfig {
    #[serde(default = "default_clock_format")]
    pub format: String,
    #[serde(default = "default_clock_interval")]
    pub interval_sec: u64,
}

impl Default for ClockConfig {
    fn default() -> Self {
        Self {
            format: default_clock_format(),
            interval_sec: default_clock_interval(),
        }
    }
}

fn default_clock_format() -> String { "%a %b %d  %H:%M:%S".to_string() }
fn default_clock_interval() -> u64 { 1 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PopupConfig {
    #[serde(default = "default_popup_width")]
    pub width: u32,
    #[serde(default = "default_popup_height")]
    pub height: u32,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for PopupConfig {
    fn default() -> Self {
        Self {
            width: default_popup_width(),
            height: default_popup_height(),
            enabled: true,
        }
    }
}

fn default_popup_width() -> u32 { 260 }
fn default_popup_height() -> u32 { 160 }

impl Default for Config {
    fn default() -> Self {
        Self {
            bar: BarConfig::default(),
            colors: ColorTheme::default(),
            layout: LayoutConfig::default(),
            modules: ModulesConfig::default(),
            output: HashMap::new(),
        }
    }
}

/// Parse hex color "#rrggbb" or "#rrggbbaa" into 0xAARRGGBB u32
pub fn parse_hex_color(hex: &str) -> u32 {
    let s = hex.trim_start_matches('#');
    match s.len() {
        6 => {
            if let Ok(v) = u32::from_str_radix(s, 16) {
                0xFF00_0000 | v
            } else {
                0xFFFF_FFFF
            }
        }
        8 => {
            if let Ok(v) = u32::from_str_radix(s, 16) {
                // In hex #RRGGBBAA -> Wayland ARGB:
                let r = (v >> 24) & 0xFF;
                let g = (v >> 16) & 0xFF;
                let b = (v >> 8) & 0xFF;
                let a = v & 0xFF;
                (a << 24) | (r << 16) | (g << 8) | b
            } else {
                0xFFFF_FFFF
            }
        }
        _ => 0xFFFF_FFFF,
    }
}

pub fn get_config_path() -> PathBuf {
    if let Ok(path) = std::env::var("HYPRSTATUS_CONFIG") {
        return PathBuf::from(path);
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| "/home/hvsr".to_string());
    PathBuf::from(home).join(".config/hyprstatus/config.toml")
}

pub fn load_or_create_config() -> Config {
    let path = get_config_path();
    if let Ok(content) = fs::read_to_string(&path) {
        match toml::from_str::<Config>(&content) {
            Ok(cfg) => {
                println!("[config] Loaded configuration from {}", path.display());
                return cfg;
            }
            Err(e) => {
                eprintln!("[config] Warning: Failed to parse {}: {}, falling back to defaults", path.display(), e);
            }
        }
    } else {
        // Create directory and write default configuration
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let default_cfg = Config::default();
        if let Ok(toml_str) = toml::to_string_pretty(&default_cfg) {
            let header = "# hyprstatus - Blazing-fast i3-style Status Bar for Hyprland\n# Auto-generated default configuration\n\n";
            let full_doc = format!("{}{}", header, toml_str);
            if let Err(e) = fs::write(&path, full_doc) {
                eprintln!("[config] Could not write default config to {}: {}", path.display(), e);
            } else {
                println!("[config] Created default configuration file at {}", path.display());
            }
        }
    }
    Config::default()
}

pub fn watch_config_file(config_tx: watch::Sender<Arc<Config>>) {
    let path = get_config_path();
    tokio::spawn(async move {
        use notify::{RecommendedWatcher, RecursiveMode, Watcher};
        let (tx, mut rx) = tokio::sync::mpsc::channel(10);
        let mut watcher = match RecommendedWatcher::new(
            move |res| {
                if let Ok(event) = res {
                    let _ = tx.blocking_send(event);
                }
            },
            notify::Config::default(),
        ) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("[config] Failed to initialize file watcher: {}", e);
                return;
            }
        };

        let watch_dir = match path.parent() {
            Some(p) if p.exists() => p,
            _ => Path::new("."),
        };

        if let Err(e) = watcher.watch(watch_dir, RecursiveMode::NonRecursive) {
            eprintln!("[config] Failed to watch {}: {}", watch_dir.display(), e);
            return;
        }

        println!("[config] Watching {} for hot-reloads", path.display());

        while let Some(event) = rx.recv().await {
            let matches = event.paths.iter().any(|p| p == &path);
            if matches {
                tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
                // Drain any additional events queued during the debounce delay
                while let Ok(_) = rx.try_recv() {}

                if let Ok(content) = fs::read_to_string(&path) {
                    match toml::from_str::<Config>(&content) {
                        Ok(new_cfg) => {
                            println!("[config] Hot-reloaded configuration successfully!");
                            let _ = config_tx.send(Arc::new(new_cfg));
                        }
                        Err(e) => {
                            eprintln!("[config] Error parsing updated config: {}", e);
                        }
                    }
                }
            }
        }
    });
}
