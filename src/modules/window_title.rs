use crate::config::{parse_hex_color, Config};
use crate::ipc::HyprState;
use crate::modules::ModuleBlock;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct WindowTitleModule {
    state: Arc<RwLock<HyprState>>,
}

impl WindowTitleModule {
    pub fn new(state: Arc<RwLock<HyprState>>) -> Self {
        Self { state }
    }

    pub fn render(&self, config: &Config) -> ModuleBlock {
        let (class, title) = {
            if let Ok(st) = self.state.try_read() {
                (st.active_window_class.clone(), st.active_window_title.clone())
            } else {
                (String::new(), String::new())
            }
        };

        let max_len = 45;
        let truncated_title = if title.chars().count() > max_len {
            let s: String = title.chars().take(max_len).collect();
            format!("{}...", s)
        } else {
            title
        };

        let text = if !class.is_empty() && !truncated_title.is_empty() {
            format!("󰣆 {} - {}", class, truncated_title)
        } else if !truncated_title.is_empty() {
            format!("󰣆 {}", truncated_title)
        } else {
            "󰣆 Hyprland".to_string()
        };

        let fg = parse_hex_color(&config.colors.foreground);
        let bg = parse_hex_color(&config.colors.background);

        let mut block = ModuleBlock::new("window_title", &text, fg, bg);
        block.icon = "󰣆".to_string();
        block
    }
}
