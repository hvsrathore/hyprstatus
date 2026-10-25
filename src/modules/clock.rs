use crate::config::{parse_hex_color, Config};
use crate::modules::{ClickAction, ClickRegion, ModuleBlock};
use std::time::SystemTime;

pub struct ClockModule;

impl ClockModule {
    pub fn new() -> Self {
        Self
    }

    pub fn render(&self, config: &Config) -> ModuleBlock {
        // Format time without external heavy chrono crate:
        // Use libc / std time
        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;

        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        unsafe {
            libc::localtime_r(&now, &mut tm);
        }

        let days = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
        let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

        let wday = days.get(tm.tm_wday.clamp(0, 6) as usize).unwrap_or(&"");
        let month = months.get(tm.tm_mon.clamp(0, 11) as usize).unwrap_or(&"");

        let text = format!(
            "󰃭 {} {} {:02}  󰥔 {:02}:{:02}:{:02}",
            wday,
            month,
            tm.tm_mday,
            tm.tm_hour,
            tm.tm_min,
            tm.tm_sec
        );

        let fg = parse_hex_color(&config.colors.foreground);
        let bg = parse_hex_color(&config.colors.block_background);

        let mut block = ModuleBlock::new("clock", &text, fg, bg);
        block.icon = "󰥔".to_string();
        block.click_regions.push(ClickRegion {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            action: ClickAction::ClockClick,
        });

        block
    }
}
