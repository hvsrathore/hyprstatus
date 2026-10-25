#![allow(dead_code)]
pub mod audio;
pub mod battery;
pub mod clock;
pub mod network;
pub mod perf;
pub mod popup;
pub mod screenshot;
pub mod tray;
pub mod window_title;
pub mod workspace;

use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    ScrollUp,
    ScrollDown,
}

#[derive(Debug, Clone)]
pub struct ClickEvent {
    pub button: MouseButton,
    pub x: i32,
    pub y: i32,
    pub output_name: String,
}

#[derive(Debug, Clone)]
pub struct ClickRegion {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub action: ClickAction,
}

#[derive(Debug, Clone)]
pub enum ClickAction {
    SwitchWorkspace(i32),
    HoverWorkspace(i32, i32, i32), // ws_id, pill_x, pill_w
    ToggleAudioMute,
    ToggleMicMute,
    AudioVolumeUp,
    AudioVolumeDown,
    NetworkClick,
    PerfClick,
    ClockClick,
    ScreenshotClick(String),
    TrayActivate(String),
    TrayContextMenu(String),
    Command(String),
}

#[derive(Debug, Clone)]
pub struct PillItem {
    pub id: i32,
    pub label: String,
    pub fg: u32,
    pub bg: u32,
    pub border: Option<u32>,
    pub is_active: bool,
    pub is_urgent: bool,
}

#[derive(Debug, Clone)]
pub struct ModuleBlock {
    pub name: String,
    pub icon: String,
    pub text: String,
    pub fg: u32,
    pub bg: u32,
    pub border: Option<u32>,
    pub sparkline: Option<String>,
    pub tray_pixmap: Option<(u32, u32, Arc<Vec<u8>>)>, // (w, h, argb)
    pub pills: Vec<PillItem>,
    pub click_regions: Vec<ClickRegion>,
}

impl ModuleBlock {
    pub fn new(name: &str, text: &str, fg: u32, bg: u32) -> Self {
        Self {
            name: name.to_string(),
            icon: String::new(),
            text: text.to_string(),
            fg,
            bg,
            border: None,
            sparkline: None,
            tray_pixmap: None,
            pills: Vec::new(),
            click_regions: Vec::new(),
        }
    }
}
