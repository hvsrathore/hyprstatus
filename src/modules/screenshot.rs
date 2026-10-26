use crate::config::{parse_hex_color, Config};
use crate::modules::{ClickAction, ClickRegion, ModuleBlock};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::SystemTime;

#[derive(Clone)]
pub struct ScreenshotModule {
    flash_active: Arc<AtomicBool>,
    flash_needs_reset: Arc<AtomicBool>,
}

impl ScreenshotModule {
    pub fn new() -> Self {
        Self {
            flash_active: Arc::new(AtomicBool::new(false)),
            flash_needs_reset: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn trigger_flash(&self) {
        let flash = self.flash_active.clone();
        let reset = self.flash_needs_reset.clone();
        flash.store(true, Ordering::Relaxed);
        reset.store(true, Ordering::Relaxed);
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(350)).await;
            flash.store(false, Ordering::Relaxed);
        });
    }

    pub fn is_flashing(&self) -> bool {
        self.flash_active.load(Ordering::Relaxed)
    }

    pub fn needs_reset(&self) -> bool {
        self.flash_needs_reset.load(Ordering::Relaxed)
    }

    pub fn take_reset(&self) -> bool {
        if !self.is_flashing() && self.flash_needs_reset.swap(false, Ordering::Relaxed) {
            true
        } else {
            false
        }
    }

    pub fn render(&self, config: &Config, output_name: &str) -> ModuleBlock {
        let is_flashing = self.is_flashing();
        let icon = if config.modules.screenshot.icon.is_empty() {
            ""
        } else {
            &config.modules.screenshot.icon
        };

        let label = &config.modules.screenshot.label;
        let text = if label.is_empty() {
            icon.to_string()
        } else {
            format!("{} {}", icon, label)
        };

        let fg = if is_flashing {
            parse_hex_color("#11111b") // Catppuccin Crust (contrast on flash)
        } else {
            parse_hex_color(&config.colors.screenshot)
        };

        let bg = if is_flashing {
            parse_hex_color("#f9e2af") // Bright flash highlight
        } else {
            parse_hex_color(&config.colors.block_background)
        };

        let mut block = ModuleBlock::new("screenshot", &text, fg, bg);
        block.icon = icon.to_string();
        block.click_regions.push(ClickRegion {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            action: ClickAction::ScreenshotClick(output_name.to_string()),
        });
        block
    }

    /// Captures the screen (including the status bar), saves to file, copies to clipboard, and sends notification.
    pub async fn capture(
        config: &Config,
        output_name: Option<&str>,
        fullscreen_all: bool,
    ) -> Result<PathBuf, String> {
        // Expand directory path (e.g. ~/Pictures/Screenshots)
        let dir_str = &config.modules.screenshot.save_directory;
        let expanded_dir = if dir_str.starts_with("~/") {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/home/hvsr".to_string());
            PathBuf::from(home).join(&dir_str[2..])
        } else {
            PathBuf::from(dir_str)
        };

        if let Err(e) = fs::create_dir_all(&expanded_dir) {
            return Err(format!("Failed to create directory {}: {}", expanded_dir.display(), e));
        }

        // Generate timestamp-based filename: screenshot_YYYY-MM-DD_HH-MM-SS.png
        let filename = {
            let now = SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            let mut tm: libc::tm = unsafe { std::mem::zeroed() };
            unsafe {
                libc::localtime_r(&now, &mut tm);
            }
            format!(
                "screenshot_{:04}-{:02}-{:02}_{:02}-{:02}-{:02}.png",
                tm.tm_year + 1900,
                tm.tm_mon + 1,
                tm.tm_mday,
                tm.tm_hour,
                tm.tm_min,
                tm.tm_sec
            )
        };
        let filepath = expanded_dir.join(&filename);
        let filepath_str = filepath.to_string_lossy().to_string();

        let custom_cmd = config.modules.screenshot.command.trim();
        let status = if !custom_cmd.is_empty() {
            let expanded_cmd = custom_cmd
                .replace("{file}", &filepath_str)
                .replace("{output}", output_name.unwrap_or(""));
            tokio::process::Command::new("sh")
                .arg("-c")
                .arg(&expanded_cmd)
                .status()
                .await
                .map_err(|e| format!("Failed to run screenshot command '{}': {}", custom_cmd, e))?
        } else {
            // Default: grim captures Wayland outputs via wlr-screencopy
            let mut cmd = tokio::process::Command::new("grim");
            if !fullscreen_all {
                if let Some(out) = output_name {
                    cmd.arg("-o").arg(out);
                }
            }
            cmd.arg(&filepath_str);
            cmd.status().await.map_err(|e| format!("Failed to run grim: {}", e))?
        };

        if !status.success() {
            return Err("Screenshot capture exited with an error".to_string());
        }

        // Copy image to clipboard via wl-copy if enabled
        if config.modules.screenshot.copy_to_clipboard {
            let fp = filepath_str.clone();
            tokio::spawn(async move {
                if let Ok(file_content) = tokio::fs::read(&fp).await {
                    use tokio::io::AsyncWriteExt;
                    if let Ok(mut child) = tokio::process::Command::new("wl-copy")
                        .arg("-t")
                        .arg("image/png")
                        .stdin(std::process::Stdio::piped())
                        .spawn()
                    {
                        if let Some(mut stdin) = child.stdin.take() {
                            let _ = stdin.write_all(&file_content).await;
                        }
                        let _ = child.wait().await;
                    }
                }
            });
        }

        // Desktop notification via notify-send or hyprctl notify if enabled
        if config.modules.screenshot.notify {
            let fp = filepath_str.clone();
            let disp = filepath.display().to_string();
            tokio::spawn(async move {
                let sent = tokio::process::Command::new("notify-send")
                    .args([
                        "-a", "hyprstatus",
                        "-i", &fp,
                        "Screenshot Captured",
                        &format!("Saved to {}\nCopied to clipboard", disp),
                    ])
                    .status()
                    .await;
                if sent.is_err() || !sent.unwrap().success() {
                    let _ = tokio::process::Command::new("hyprctl")
                        .args(["notify", "-1", "4000", "rgb(a6e3a1)", &format!("Screenshot saved to {}", disp)])
                        .spawn();
                }
            });
        }

        Ok(filepath)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_screenshot_module_render() {
        let config = Config::default();
        let module = ScreenshotModule::new();
        let block = module.render(&config, "HDMI-A-1");

        assert_eq!(block.name, "screenshot");
        assert_eq!(block.icon, "");
        assert_eq!(block.text, "");
        assert_eq!(block.click_regions.len(), 1);
        if let ClickAction::ScreenshotClick(out) = &block.click_regions[0].action {
            assert_eq!(out, "HDMI-A-1");
        } else {
            panic!("Expected ClickAction::ScreenshotClick");
        }
    }
}
