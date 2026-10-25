use crate::config::{parse_hex_color, Config};
use crate::ipc::{HyprClient, HyprlandIpc};
use crate::renderer::SoftwareRenderer;

#[derive(Debug, Clone)]
pub struct ThumbnailRect {
    pub address: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub title: String,
    pub class: String,
}

#[derive(Debug, Clone)]
pub struct WorkspacePopup {
    pub width: u32,
    pub height: u32,
    pub visible: bool,
    pub workspace_id: i32,
    pub anchor_x: i32,
    pub thumbnails: Vec<ThumbnailRect>,
}

impl WorkspacePopup {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            visible: false,
            workspace_id: 1,
            anchor_x: 0,
            thumbnails: Vec::new(),
        }
    }

    pub fn update_clients(
        &mut self,
        ws_id: i32,
        anchor_x: i32,
        clients: &[HyprClient],
        mon_w: u32,
        mon_h: u32,
    ) {
        self.workspace_id = ws_id;
        self.anchor_x = anchor_x;
        self.visible = true;
        self.thumbnails.clear();

        let filtered: Vec<&HyprClient> = clients.iter().filter(|c| c.workspace.id == ws_id).collect();
        if filtered.is_empty() {
            return;
        }

        let min_x = filtered.iter().map(|c| c.at.0).min().unwrap_or(0);
        let min_y = filtered.iter().map(|c| c.at.1).min().unwrap_or(0);

        let pad_x = 12i32;
        let pad_top = 30i32;
        let pad_bot = 40i32;

        let inner_w = (self.width as i32 - pad_x * 2).max(10) as f32;
        let inner_h = (self.height as i32 - pad_top - pad_bot).max(10) as f32;

        let mon_w_f = mon_w.max(640) as f32;
        let mon_h_f = mon_h.max(480) as f32;

        let scale_x = inner_w / mon_w_f;
        let scale_y = inner_h / mon_h_f;

        for c in &filtered {
            let rx = (c.at.0 - min_x).max(0) as f32;
            let ry = (c.at.1 - min_y).max(0) as f32;

            let x = pad_x + (rx * scale_x) as i32;
            let y = pad_top + (ry * scale_y) as i32;
            let w = ((c.size.0 as f32 * scale_x).round() as u32).max(30).min(self.width - pad_x as u32 * 2);
            let h = ((c.size.1 as f32 * scale_y).round() as u32).max(22).min(inner_h as u32);

            self.thumbnails.push(ThumbnailRect {
                address: c.address.clone(),
                x,
                y,
                width: w,
                height: h,
                title: c.title.clone(),
                class: c.class.clone(),
            });
        }
    }

    pub fn handle_click(&self, click_x: i32, click_y: i32) -> bool {
        for thumb in &self.thumbnails {
            if click_x >= thumb.x
                && click_x < thumb.x + thumb.width as i32
                && click_y >= thumb.y
                && click_y < thumb.y + thumb.height as i32
            {
                let addr = thumb.address.clone();
                tokio::spawn(async move {
                    let _ = HyprlandIpc::focus_window(&addr).await;
                });
                return true;
            }
        }
        false
    }

    pub fn render(
        &self,
        renderer: &mut SoftwareRenderer,
        buffer: &mut [u8],
        config: &Config,
    ) {
        let bg_color = parse_hex_color("#181825"); // Base background
        let border_color = parse_hex_color(&config.colors.active_workspace_bg);
        let header_fg = parse_hex_color(&config.colors.foreground);
        let thumb_bg = parse_hex_color(&config.colors.block_background);
        let thumb_border = parse_hex_color(&config.colors.border);
        let text_fg = parse_hex_color(&config.colors.foreground);
        let accent_color = parse_hex_color(&config.colors.active_workspace_bg);
        let muted_text = parse_hex_color("#a6adc8");

        // Draw popup background with border
        SoftwareRenderer::draw_rounded_rect(
            buffer,
            self.width,
            self.height,
            0,
            0,
            self.width,
            self.height,
            8,
            bg_color,
            Some(border_color),
            2,
        );

        // Header: "󰍹 Workspace {id}"
        let title = format!("󰍹 Workspace {}", self.workspace_id);
        renderer.draw_text(
            buffer,
            self.width,
            self.height,
            &title,
            12,
            20,
            11.5,
            accent_color,
        );

        // Badge on right of header: window count
        let count_str = if self.thumbnails.is_empty() {
            "Empty".to_string()
        } else if self.thumbnails.len() == 1 {
            "1 Window".to_string()
        } else {
            format!("{} Windows", self.thumbnails.len())
        };
        renderer.draw_text(
            buffer,
            self.width,
            self.height,
            &count_str,
            (self.width - 70) as i32,
            20,
            9.0,
            muted_text,
        );

        if self.thumbnails.is_empty() {
            renderer.draw_text(
                buffer,
                self.width,
                self.height,
                "No open windows",
                self.width as i32 / 2 - 45,
                self.height as i32 / 2 + 5,
                11.0,
                thumb_border,
            );
        } else {
            // Draw miniature window wireframes
            for thumb in &self.thumbnails {
                SoftwareRenderer::draw_rounded_rect(
                    buffer,
                    self.width,
                    self.height,
                    thumb.x,
                    thumb.y,
                    thumb.width,
                    thumb.height,
                    4,
                    thumb_bg,
                    Some(accent_color),
                    1,
                );

                let icon = match thumb.class.to_lowercase().as_str() {
                    c if c.contains("chrome") || c.contains("chromium") || c.contains("browser") => "󰊯",
                    c if c.contains("firefox") => "󰈹",
                    c if c.contains("kitty") || c.contains("terminal") || c.contains("alacritty") => "󰞷",
                    c if c.contains("code") || c.contains("nvim") || c.contains("vim") => "󰨞",
                    c if c.contains("thunar") || c.contains("nautilus") || c.contains("file") => "󰘑",
                    _ => "󰖲",
                };

                let label = format!("{} {}", icon, if !thumb.class.is_empty() { &thumb.class } else { "app" });
                let max_chars = (thumb.width / 7).max(1) as usize;
                let short_label: String = label.chars().take(max_chars).collect();

                renderer.draw_text(
                    buffer,
                    self.width,
                    self.height,
                    &short_label,
                    thumb.x + 6,
                    thumb.y + thumb.height as i32 / 2 + 4,
                    9.0,
                    text_fg,
                );
            }

            // Bottom title banner: shows window title clearly
            let banner_y = (self.height as i32 - 32).max(28);
            SoftwareRenderer::draw_rounded_rect(
                buffer,
                self.width,
                self.height,
                12,
                banner_y,
                self.width - 24,
                24,
                4,
                parse_hex_color("#1e1e2e"),
                Some(thumb_border),
                1,
            );

            if let Some(first) = self.thumbnails.first() {
                let display_title = if !first.title.is_empty() {
                    &first.title
                } else if !first.class.is_empty() {
                    &first.class
                } else {
                    "Untitled Window"
                };
                let max_chars = ((self.width - 40) / 7) as usize;
                let short_title: String = display_title.chars().take(max_chars).collect();
                renderer.draw_text(
                    buffer,
                    self.width,
                    self.height,
                    &short_title,
                    20,
                    banner_y + 16,
                    8.5,
                    header_fg,
                );
            }
        }
    }
}

// -------------------------------- Audio Popover --------------------------------

#[derive(Debug, Clone)]
pub struct AudioPopover {
    pub width: u32,
    pub height: u32,
    pub volume: u32,
    pub sink_muted: bool,
    pub source_muted: bool,
    pub sink_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioPopoverAction {
    None,
    SetVolume(u32),
    VolumeDelta(i32),
    ToggleMute,
    ToggleMicMute,
    Close,
}

impl AudioPopover {
    pub fn new(
        width: u32,
        height: u32,
        volume: u32,
        sink_muted: bool,
        source_muted: bool,
        sink_name: String,
    ) -> Self {
        Self {
            width,
            height,
            volume,
            sink_muted,
            source_muted,
            sink_name,
        }
    }

    pub fn handle_click(&self, click_x: i32, click_y: i32) -> AudioPopoverAction {
        // Close button [✕] at top-right
        if click_x >= 280 && click_x <= 312 && click_y >= 6 && click_y <= 30 {
            return AudioPopoverAction::Close;
        }

        // Volume slider track: x: 16..304, y: 58..86
        if click_x >= 16 && click_x <= 304 && click_y >= 58 && click_y <= 86 {
            let ratio = ((click_x - 16) as f32 / 288.0).clamp(0.0, 1.0);
            let new_vol = (ratio * 100.0).round() as u32;
            return AudioPopoverAction::SetVolume(new_vol);
        }

        // Step buttons and mute toggle: y: 92..118
        if click_y >= 92 && click_y <= 118 {
            if click_x >= 16 && click_x <= 71 {
                return AudioPopoverAction::VolumeDelta(-5);
            }
            if click_x >= 77 && click_x <= 132 {
                return AudioPopoverAction::VolumeDelta(5);
            }
            if click_x >= 138 && click_x <= 230 {
                return AudioPopoverAction::ToggleMute;
            }
            if click_x >= 236 && click_x <= 304 {
                return AudioPopoverAction::SetVolume(100);
            }
        }

        // Volume preset buttons: y: 124..146
        if click_y >= 124 && click_y <= 146 {
            let levels = [0, 25, 50, 75, 100];
            for (i, &lvl) in levels.iter().enumerate() {
                let bx = 16 + (i as i32) * 59;
                if click_x >= bx && click_x <= bx + 52 {
                    return AudioPopoverAction::SetVolume(lvl);
                }
            }
        }

        // Mic toggle button: y: 182..210
        if click_x >= 16 && click_x <= 304 && click_y >= 182 && click_y <= 210 {
            return AudioPopoverAction::ToggleMicMute;
        }

        AudioPopoverAction::None
    }

    pub fn render(
        &self,
        renderer: &mut SoftwareRenderer,
        buffer: &mut [u8],
        config: &Config,
        anim_frame: u32,
    ) {
        let bg_color = parse_hex_color("#181825");
        let border_color = parse_hex_color(&config.colors.audio);
        let text_color = parse_hex_color(&config.colors.foreground);
        let block_bg = parse_hex_color(&config.colors.block_background);
        let border_subtle = parse_hex_color(&config.colors.border);
        let accent_color = parse_hex_color(&config.colors.audio);
        let green_color = parse_hex_color(&config.colors.battery_good);
        let red_color = parse_hex_color(&config.colors.battery_critical);
        let muted_text = parse_hex_color("#a6adc8");

        // Outer window with rounded corners
        SoftwareRenderer::draw_rounded_rect(
            buffer,
            self.width,
            self.height,
            0,
            0,
            self.width,
            self.height,
            8,
            bg_color,
            Some(border_color),
            2,
        );

        // Header: "󰕾 Sound & Volume"
        renderer.draw_text(
            buffer,
            self.width,
            self.height,
            "󰕾 Sound & Volume",
            16,
            24,
            12.0,
            accent_color,
        );

        // Subtle dancing equalizer visualizer (5 bars) next to the header
        let eq_base_x = 236;
        let eq_base_y = 26;
        let eq_bar_w = 4;
        let eq_gap = 2;
        for i in 0..5 {
            let bar_h = if self.sink_muted {
                2
            } else {
                let t = anim_frame as f32 * 0.22 + i as f32 * 1.25;
                let h_norm = ((t.sin() + (t * 0.7).cos()) * 0.5 + 1.0) * 0.5;
                (3.0 + h_norm * 11.0).round() as u32
            };
            let bx = eq_base_x + i as i32 * (eq_bar_w + eq_gap);
            let by = eq_base_y - bar_h as i32;
            let eq_color = if self.sink_muted {
                parse_hex_color("#585b70")
            } else {
                accent_color
            };
            SoftwareRenderer::draw_rounded_rect(
                buffer,
                self.width,
                self.height,
                bx,
                by,
                eq_bar_w as u32,
                bar_h,
                1,
                eq_color,
                None,
                0,
            );
        }

        // Subtitle: Output Sink
        renderer.draw_text(
            buffer,
            self.width,
            self.height,
            &self.sink_name,
            16,
            38,
            9.0,
            muted_text,
        );

        // Close button [✕] at top-right
        SoftwareRenderer::draw_rounded_rect(
            buffer,
            self.width,
            self.height,
            286,
            10,
            20,
            20,
            4,
            block_bg,
            Some(border_subtle),
            1,
        );
        renderer.draw_text(buffer, self.width, self.height, "✕", 292, 24, 10.0, text_color);

        // Volume label
        let vol_str = if self.sink_muted {
            format!("Master Volume: {}% (MUTED)", self.volume)
        } else {
            format!("Master Volume: {}%", self.volume)
        };
        let vol_color = if self.sink_muted { red_color } else { text_color };
        renderer.draw_text(buffer, self.width, self.height, &vol_str, 16, 56, 10.5, vol_color);

        // Volume slider track: x: 16, y: 64, w: 288, h: 18
        SoftwareRenderer::draw_rounded_rect(
            buffer,
            self.width,
            self.height,
            16,
            64,
            288,
            18,
            5,
            block_bg,
            Some(border_subtle),
            1,
        );

        // Slider fill
        let fill_w = ((self.volume as f32 / 100.0).clamp(0.0, 1.0) * 288.0).round() as u32;
        if fill_w > 0 {
            let fill_color = if self.sink_muted { parse_hex_color("#585b70") } else { accent_color };
            SoftwareRenderer::draw_rounded_rect(
                buffer,
                self.width,
                self.height,
                16,
                64,
                fill_w,
                18,
                5,
                fill_color,
                None,
                0,
            );
        }

        // Tactile slider thumb with glowing accent border
        let thumb_cx = 16 + fill_w as i32;
        let thumb_x = (thumb_cx - 5).clamp(16, 16 + 288 - 10);
        let (thumb_bg, thumb_border) = if self.sink_muted {
            (parse_hex_color("#45475a"), parse_hex_color("#585b70"))
        } else {
            let breathe = (anim_frame as f32 * 0.14).sin() * 0.5 + 0.5;
            let glow = if breathe > 0.5 {
                parse_hex_color("#ffffff")
            } else {
                accent_color
            };
            (accent_color, glow)
        };
        SoftwareRenderer::draw_rounded_rect(
            buffer,
            self.width,
            self.height,
            thumb_x,
            62,
            10,
            22,
            4,
            thumb_bg,
            Some(thumb_border),
            1,
        );

        // Quick Step & Mute buttons (y = 92, h = 26)
        // 1. [-] button
        SoftwareRenderer::draw_rounded_rect(buffer, self.width, self.height, 16, 92, 55, 26, 4, block_bg, Some(border_subtle), 1);
        renderer.draw_text(buffer, self.width, self.height, "󰝤 -5%", 25, 108, 9.5, text_color);

        // 2. [+] button
        SoftwareRenderer::draw_rounded_rect(buffer, self.width, self.height, 77, 92, 55, 26, 4, block_bg, Some(border_subtle), 1);
        renderer.draw_text(buffer, self.width, self.height, "󰐕 +5%", 86, 108, 9.5, text_color);

        // 3. Mute Toggle button
        let (mute_bg, mute_border, mute_fg, mute_text) = if self.sink_muted {
            (red_color, None, parse_hex_color("#11111b"), "󰝟 Unmute")
        } else {
            (block_bg, Some(border_subtle), text_color, "󰕾 Mute")
        };
        SoftwareRenderer::draw_rounded_rect(buffer, self.width, self.height, 138, 92, 92, 26, 4, mute_bg, mute_border, 1);
        renderer.draw_text(buffer, self.width, self.height, mute_text, 156, 108, 9.5, mute_fg);

        // 4. Max 100% button
        SoftwareRenderer::draw_rounded_rect(buffer, self.width, self.height, 236, 92, 68, 26, 4, block_bg, Some(border_subtle), 1);
        renderer.draw_text(buffer, self.width, self.height, "100%", 254, 108, 9.5, text_color);

        // Preset levels row (y = 124, h = 22)
        let presets = ["0%", "25%", "50%", "75%", "100%"];
        for (i, p) in presets.iter().enumerate() {
            let bx = 16 + (i as i32) * 59;
            SoftwareRenderer::draw_rounded_rect(buffer, self.width, self.height, bx, 124, 52, 22, 4, parse_hex_color("#1e1e2e"), Some(border_subtle), 1);
            let tx = bx + 14;
            renderer.draw_text(buffer, self.width, self.height, p, tx, 138, 9.0, muted_text);
        }

        // Divider
        SoftwareRenderer::draw_rect(buffer, self.width, self.height, 16, 156, 288, 1, border_subtle);

        // Microphone input row
        renderer.draw_text(buffer, self.width, self.height, "Microphone Input", 16, 172, 10.0, text_color);
        if self.source_muted {
            renderer.draw_text(buffer, self.width, self.height, "● MUTED", 248, 172, 9.5, red_color);
        } else {
            let pulse = (anim_frame as f32 * 0.16).sin() * 0.5 + 0.5;
            let mic_color = if pulse > 0.5 { green_color } else { parse_hex_color("#94e2d5") };
            let dot = if pulse > 0.6 { "●" } else { "○" };
            renderer.draw_text(buffer, self.width, self.height, &format!("{} LIVE", dot), 250, 172, 9.5, mic_color);
        }

        // Mic toggle button (y = 182, h = 28)
        let (mic_btn_border, mic_btn_fg, mic_btn_text) = if self.source_muted {
            (red_color, red_color, "󰍭 Microphone is Muted (Click to Unmute)")
        } else {
            (green_color, green_color, "󰍬 Microphone is Active (Click to Mute)")
        };
        SoftwareRenderer::draw_rounded_rect(buffer, self.width, self.height, 16, 182, 288, 28, 4, block_bg, Some(mic_btn_border), 1);
        renderer.draw_text(buffer, self.width, self.height, mic_btn_text, 36, 200, 9.5, mic_btn_fg);
    }
}

// -------------------------------- Wi-Fi Popover --------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiNetworkItem {
    pub in_use: bool,
    pub ssid: String,
    pub signal: u32,
    pub security: String,
    pub is_saved: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiPasswordPrompt {
    pub ssid: String,
    pub password: String,
    pub show_password: bool,
    pub error_message: Option<String>,
    pub is_connecting: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiPasswordRequest {
    pub ssid: String,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct WifiPopover {
    pub width: u32,
    pub height: u32,
    pub wifi_enabled: bool,
    pub active_ssid: String,
    pub active_signal: u32,
    pub networks: Vec<WifiNetworkItem>,
    pub status_message: Option<String>,
    pub status_timer: u32,
    pub is_scanning: bool,
    pub password_prompt: Option<WifiPasswordPrompt>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WifiPopoverAction {
    None,
    ToggleRadio,
    Rescan,
    Connect(String),
    Disconnect(String),
    Forget(String),
    OpenNmtui,
    Close,
    CancelPasswordPrompt,
    ToggleShowPassword,
    SubmitPassword { ssid: String, password: String },
    PasswordChanged,
}

impl WifiPopover {
    pub fn new(
        width: u32,
        height: u32,
        wifi_enabled: bool,
        active_ssid: String,
        active_signal: u32,
        networks: Vec<WifiNetworkItem>,
    ) -> Self {
        Self {
            width,
            height,
            wifi_enabled,
            active_ssid,
            active_signal,
            networks,
            status_message: None,
            status_timer: 0,
            is_scanning: false,
            password_prompt: None,
        }
    }

    pub fn handle_key_input(
        &mut self,
        text: &str,
        is_backspace: bool,
        is_enter: bool,
        is_escape: bool,
        is_clear: bool,
    ) -> WifiPopoverAction {
        if let Some(prompt) = &mut self.password_prompt {
            if is_escape {
                self.password_prompt = None;
                return WifiPopoverAction::CancelPasswordPrompt;
            }
            if is_enter {
                if !prompt.is_connecting && !prompt.password.is_empty() {
                    prompt.is_connecting = true;
                    prompt.error_message = None;
                    return WifiPopoverAction::SubmitPassword {
                        ssid: prompt.ssid.clone(),
                        password: prompt.password.clone(),
                    };
                }
                return WifiPopoverAction::None;
            }
            if is_clear {
                prompt.password.clear();
                prompt.error_message = None;
                return WifiPopoverAction::PasswordChanged;
            }
            if is_backspace {
                prompt.password.pop();
                prompt.error_message = None;
                return WifiPopoverAction::PasswordChanged;
            }
            if !text.is_empty() {
                for ch in text.chars() {
                    if !ch.is_control() {
                        prompt.password.push(ch);
                    }
                }
                prompt.error_message = None;
                return WifiPopoverAction::PasswordChanged;
            }
        }
        WifiPopoverAction::None
    }

    pub fn handle_click(&self, click_x: i32, click_y: i32) -> WifiPopoverAction {
        // If password prompt is active, handle clicks inside password prompt view
        if let Some(prompt) = &self.password_prompt {
            // Close button [✕] at top-right
            if click_x >= 300 && click_x <= 332 && click_y >= 4 && click_y <= 32 {
                return WifiPopoverAction::Close;
            }

            // Back button [← Back]: x: 16..90, y: 6..32
            if click_x >= 16 && click_x <= 90 && click_y >= 6 && click_y <= 32 {
                return WifiPopoverAction::CancelPasswordPrompt;
            }

            // Show/Hide password toggle (eye icon): x: 276..320, y: 126..168
            if click_x >= 276 && click_x <= 320 && click_y >= 126 && click_y <= 168 {
                return WifiPopoverAction::ToggleShowPassword;
            }

            // Cancel button: x: 16..162, y: 202..242
            if click_x >= 16 && click_x <= 162 && click_y >= 202 && click_y <= 242 {
                return WifiPopoverAction::CancelPasswordPrompt;
            }

            // Connect button: x: 178..324, y: 202..242
            if click_x >= 178 && click_x <= 324 && click_y >= 202 && click_y <= 242 {
                if !prompt.is_connecting && !prompt.password.is_empty() {
                    return WifiPopoverAction::SubmitPassword {
                        ssid: prompt.ssid.clone(),
                        password: prompt.password.clone(),
                    };
                }
            }

            return WifiPopoverAction::None;
        }

        // Close button [✕] at top-right
        if click_x >= 300 && click_x <= 332 && click_y >= 4 && click_y <= 32 {
            return WifiPopoverAction::Close;
        }

        // Rescan button: x: 230..296, y: 6..30
        if click_x >= 230 && click_x <= 296 && click_y >= 6 && click_y <= 30 {
            return WifiPopoverAction::Rescan;
        }

        // Wi-Fi radio toggle switch: x: 236..328, y: 36..64
        if click_x >= 236 && click_x <= 328 && click_y >= 36 && click_y <= 64 {
            return WifiPopoverAction::ToggleRadio;
        }

        // Network items list (up to 5 items): y: 94 + i * 38
        for (i, net) in self.networks.iter().take(5).enumerate() {
            let row_y = 94 + (i as i32) * 38;
            if click_y >= row_y && click_y <= row_y + 34 {
                // If net is saved, clicking the trash/forget button: x: 288..322
                if net.is_saved && click_x >= 288 && click_x <= 322 {
                    return WifiPopoverAction::Forget(net.ssid.clone());
                }

                if click_x >= 16 && click_x <= 324 {
                    if net.in_use {
                        return WifiPopoverAction::Disconnect(net.ssid.clone());
                    } else {
                        return WifiPopoverAction::Connect(net.ssid.clone());
                    }
                }
            }
        }

        // Bottom nmtui button: y: 304..338
        if click_x >= 16 && click_x <= 324 && click_y >= 304 && click_y <= 338 {
            return WifiPopoverAction::OpenNmtui;
        }

        WifiPopoverAction::None
    }

    pub fn render(
        &self,
        renderer: &mut SoftwareRenderer,
        buffer: &mut [u8],
        config: &Config,
        anim_frame: u32,
    ) {
        if let Some(prompt) = &self.password_prompt {
            self.render_password_prompt(renderer, buffer, config, anim_frame, prompt);
            return;
        }

        let bg_color = parse_hex_color("#181825");
        let border_color = parse_hex_color(&config.colors.network);
        let text_color = parse_hex_color(&config.colors.foreground);
        let block_bg = parse_hex_color(&config.colors.block_background);
        let border_subtle = parse_hex_color(&config.colors.border);
        let accent_color = parse_hex_color(&config.colors.network);
        let green_color = parse_hex_color(&config.colors.battery_good);
        let red_color = parse_hex_color(&config.colors.battery_critical);
        let muted_text = parse_hex_color("#a6adc8");

        // Outer window with rounded corners
        SoftwareRenderer::draw_rounded_rect(
            buffer,
            self.width,
            self.height,
            0,
            0,
            self.width,
            self.height,
            8,
            bg_color,
            Some(border_color),
            2,
        );

        // Header: "󰤨 Wi-Fi Networks"
        renderer.draw_text(
            buffer,
            self.width,
            self.height,
            "󰤨 Wi-Fi Networks",
            16,
            24,
            12.0,
            accent_color,
        );

        // Rescan button with rotating spinner animation when scanning
        let (scan_icon, scan_text, scan_border) = if self.is_scanning {
            let spinner = match (anim_frame / 2) % 4 {
                0 => "󰑐",
                1 => "󰑑",
                2 => "󰑒",
                _ => "󰑓",
            };
            let dots = match (anim_frame / 4) % 3 {
                0 => "Scan.",
                1 => "Scan..",
                _ => "Scan...",
            };
            (spinner, dots, accent_color)
        } else {
            ("󰑐", "Scan", border_subtle)
        };
        SoftwareRenderer::draw_rounded_rect(
            buffer,
            self.width,
            self.height,
            226,
            8,
            70,
            22,
            4,
            block_bg,
            Some(scan_border),
            1,
        );
        renderer.draw_text(
            buffer,
            self.width,
            self.height,
            &format!("{} {}", scan_icon, scan_text),
            230,
            22,
            8.5,
            text_color,
        );

        // Close button [✕]
        SoftwareRenderer::draw_rounded_rect(
            buffer,
            self.width,
            self.height,
            304,
            8,
            20,
            22,
            4,
            block_bg,
            Some(border_subtle),
            1,
        );
        renderer.draw_text(buffer, self.width, self.height, "✕", 310, 22, 10.0, text_color);

        // Wi-Fi Radio Row
        let radio_label = if self.wifi_enabled {
            "Wi-Fi Radio: Enabled"
        } else {
            "Wi-Fi Radio: Disabled"
        };
        let radio_fg = if self.wifi_enabled { green_color } else { red_color };
        renderer.draw_text(buffer, self.width, self.height, radio_label, 16, 52, 10.5, radio_fg);

        // Toggle radio button
        let (toggle_bg, toggle_text) = if self.wifi_enabled {
            (green_color, "󰤨 Turn OFF")
        } else {
            (red_color, "󰤭 Turn ON")
        };
        SoftwareRenderer::draw_rounded_rect(
            buffer,
            self.width,
            self.height,
            236,
            38,
            88,
            24,
            4,
            toggle_bg,
            None,
            0,
        );
        renderer.draw_text(buffer, self.width, self.height, toggle_text, 246, 54, 9.0, parse_hex_color("#11111b"));

        // Status banner if present (e.g. "Connecting to 3a Pro...", "Scanning...")
        let list_start_y = if let Some(ref msg) = self.status_message {
            let is_connecting = msg.contains("Connecting") || msg.contains("Scanning") || msg.contains("Prompting");
            let is_failed = msg.contains("Failed") || msg.contains("Error") || msg.contains("Incorrect") || msg.contains("Wrong");
            let msg_color = if is_failed {
                red_color
            } else if is_connecting {
                accent_color
            } else {
                green_color
            };
            SoftwareRenderer::draw_rounded_rect(
                buffer,
                self.width,
                self.height,
                16,
                68,
                308,
                20,
                4,
                parse_hex_color("#1e1e2e"),
                Some(msg_color),
                1,
            );
            let display_msg = if is_failed {
                format!("󰅖 {}", msg)
            } else if is_connecting {
                let waves = match (anim_frame / 3) % 4 {
                    0 => "󰤟",
                    1 => "󰤢",
                    2 => "󰤥",
                    _ => "󰤨",
                };
                format!("{} {}", waves, msg)
            } else {
                format!("󰄬 {}", msg)
            };
            renderer.draw_text(buffer, self.width, self.height, &display_msg, 22, 82, 8.5, msg_color);
            94
        } else {
            SoftwareRenderer::draw_rect(buffer, self.width, self.height, 16, 72, 308, 1, border_subtle);
            renderer.draw_text(buffer, self.width, self.height, "Available Networks", 16, 86, 9.5, muted_text);
            94
        };

        // Networks list
        if self.networks.is_empty() {
            let msg = if !self.wifi_enabled {
                "Wi-Fi is turned off"
            } else if self.is_scanning {
                "Scanning for nearby Wi-Fi networks..."
            } else {
                "No Wi-Fi networks found"
            };
            renderer.draw_text(buffer, self.width, self.height, msg, 16, 130, 10.5, muted_text);
        } else {
            for (i, net) in self.networks.iter().take(5).enumerate() {
                let row_y = list_start_y + (i as i32) * 38;
                let is_connecting_this = self.status_message.as_ref().map_or(false, |m| m.contains(&net.ssid) && m.contains("Connecting"));

                let (row_bg, row_border, row_border_w) = if net.in_use {
                    let breathe = (anim_frame as f32 * 0.14).sin() * 0.5 + 0.5;
                    let b_color = if breathe > 0.5 {
                        accent_color
                    } else {
                        parse_hex_color("#b4befe")
                    };
                    (parse_hex_color("#1e2030"), b_color, 1)
                } else if is_connecting_this {
                    (parse_hex_color("#1e2030"), accent_color, 1)
                } else {
                    (parse_hex_color("#1e1e2e"), border_subtle, 1)
                };

                SoftwareRenderer::draw_rounded_rect(
                    buffer,
                    self.width,
                    self.height,
                    16,
                    row_y,
                    308,
                    34,
                    5,
                    row_bg,
                    Some(row_border),
                    row_border_w,
                );

                // Signal icon & color
                let (icon, icon_color) = if net.signal >= 75 {
                    ("󰤨", green_color)
                } else if net.signal >= 50 {
                    ("󰤥", accent_color)
                } else if net.signal >= 25 {
                    ("󰤢", parse_hex_color(&config.colors.battery_warning))
                } else {
                    ("󰤟", red_color)
                };
                renderer.draw_text(buffer, self.width, self.height, icon, 24, row_y + 22, 11.0, icon_color);

                // SSID name
                let max_chars = 15;
                let short_ssid: String = net.ssid.chars().take(max_chars).collect();
                renderer.draw_text(buffer, self.width, self.height, &short_ssid, 44, row_y + 17, 10.0, text_color);

                // Security label & Saved indicator
                let sec_text = if net.is_saved {
                    format!("Saved • 󰌾 {}", net.security)
                } else if net.security.is_empty() || net.security == "--" {
                    "Open Network".to_string()
                } else {
                    format!("󰌾 {}", net.security)
                };
                renderer.draw_text(buffer, self.width, self.height, &sec_text, 44, row_y + 29, 8.0, muted_text);

                // Forget button [ 󰆴 ] for saved networks (x: 288..318)
                if net.is_saved {
                    SoftwareRenderer::draw_rounded_rect(
                        buffer,
                        self.width,
                        self.height,
                        288,
                        row_y + 6,
                        30,
                        22,
                        4,
                        block_bg,
                        Some(border_subtle),
                        1,
                    );
                    renderer.draw_text(buffer, self.width, self.height, "󰆴", 298, row_y + 20, 9.5, red_color);
                }

                // Action badge / Connect button
                if net.in_use {
                    let badge_w = if net.is_saved { 70 } else { 90 };
                    let badge_x = if net.is_saved { 214 } else { 226 };
                    SoftwareRenderer::draw_rounded_rect(
                        buffer,
                        self.width,
                        self.height,
                        badge_x,
                        row_y + 6,
                        badge_w,
                        22,
                        4,
                        accent_color,
                        None,
                        0,
                    );
                    let label = if net.is_saved { "󰄬 Active" } else { "󰄬 Connected" };
                    renderer.draw_text(buffer, self.width, self.height, label, badge_x + 6, row_y + 20, 8.5, parse_hex_color("#11111b"));
                } else if is_connecting_this {
                    let dots = match (anim_frame / 3) % 4 {
                        0 => "Wait",
                        1 => "Wait.",
                        2 => "Wait..",
                        _ => "Wait...",
                    };
                    let badge_w = if net.is_saved { 62 } else { 76 };
                    let badge_x = if net.is_saved { 222 } else { 240 };
                    SoftwareRenderer::draw_rounded_rect(
                        buffer,
                        self.width,
                        self.height,
                        badge_x,
                        row_y + 6,
                        badge_w,
                        22,
                        4,
                        parse_hex_color("#313244"),
                        Some(accent_color),
                        1,
                    );
                    renderer.draw_text(buffer, self.width, self.height, dots, badge_x + 12, row_y + 20, 8.5, accent_color);
                } else {
                    let btn_w = if net.is_saved { 62 } else { 68 };
                    let btn_x = if net.is_saved { 222 } else { 248 };
                    SoftwareRenderer::draw_rounded_rect(
                        buffer,
                        self.width,
                        self.height,
                        btn_x,
                        row_y + 6,
                        btn_w,
                        22,
                        4,
                        block_bg,
                        Some(border_subtle),
                        1,
                    );
                    renderer.draw_text(buffer, self.width, self.height, "Connect", btn_x + 10, row_y + 20, 8.5, text_color);
                }
            }
        }

        // Bottom nmtui button
        SoftwareRenderer::draw_rounded_rect(
            buffer,
            self.width,
            self.height,
            16,
            304,
            308,
            30,
            5,
            block_bg,
            Some(border_subtle),
            1,
        );
        renderer.draw_text(
            buffer,
            self.width,
            self.height,
            "󰖩 Network Manager Settings (nmtui)",
            42,
            324,
            9.5,
            text_color,
        );
    }

    fn render_password_prompt(
        &self,
        renderer: &mut SoftwareRenderer,
        buffer: &mut [u8],
        config: &Config,
        anim_frame: u32,
        prompt: &WifiPasswordPrompt,
    ) {
        let bg_color = parse_hex_color("#181825");
        let border_color = parse_hex_color(&config.colors.network);
        let text_color = parse_hex_color(&config.colors.foreground);
        let block_bg = parse_hex_color(&config.colors.block_background);
        let border_subtle = parse_hex_color(&config.colors.border);
        let accent_color = parse_hex_color(&config.colors.network);
        let green_color = parse_hex_color(&config.colors.battery_good);
        let red_color = parse_hex_color(&config.colors.battery_critical);
        let muted_text = parse_hex_color("#a6adc8");

        // Outer window with rounded corners
        SoftwareRenderer::draw_rounded_rect(
            buffer,
            self.width,
            self.height,
            0,
            0,
            self.width,
            self.height,
            8,
            bg_color,
            Some(border_color),
            2,
        );

        // Header: Back button [← Back], Title "Wi-Fi Authentication", Close button [✕]
        SoftwareRenderer::draw_rounded_rect(
            buffer,
            self.width,
            self.height,
            16,
            8,
            68,
            22,
            4,
            block_bg,
            Some(border_subtle),
            1,
        );
        renderer.draw_text(buffer, self.width, self.height, "󰁍 Back", 24, 22, 8.5, text_color);

        renderer.draw_text(
            buffer,
            self.width,
            self.height,
            "Wi-Fi Authentication",
            94,
            24,
            11.0,
            accent_color,
        );

        // Close button [✕]
        SoftwareRenderer::draw_rounded_rect(
            buffer,
            self.width,
            self.height,
            304,
            8,
            20,
            22,
            4,
            block_bg,
            Some(border_subtle),
            1,
        );
        renderer.draw_text(buffer, self.width, self.height, "✕", 310, 22, 10.0, text_color);

        // Network Info Badge / Card
        SoftwareRenderer::draw_rounded_rect(
            buffer,
            self.width,
            self.height,
            16,
            38,
            308,
            46,
            6,
            parse_hex_color("#1e1e2e"),
            Some(border_subtle),
            1,
        );

        let net_item = self.networks.iter().find(|n| n.ssid == prompt.ssid);
        let signal = net_item.map_or(self.active_signal, |n| n.signal);
        let security = net_item.map_or("WPA2/WPA3", |n| {
            if n.security.is_empty() || n.security == "--" {
                "Encrypted"
            } else {
                &n.security
            }
        });

        let (icon, icon_color) = if signal >= 75 {
            ("󰤨", green_color)
        } else if signal >= 50 {
            ("󰤥", accent_color)
        } else if signal >= 25 {
            ("󰤢", parse_hex_color(&config.colors.battery_warning))
        } else {
            ("󰤟", red_color)
        };
        renderer.draw_text(buffer, self.width, self.height, icon, 26, 68, 13.0, icon_color);

        let short_ssid: String = prompt.ssid.chars().take(20).collect();
        renderer.draw_text(buffer, self.width, self.height, &short_ssid, 50, 56, 11.0, text_color);
        renderer.draw_text(
            buffer,
            self.width,
            self.height,
            &format!("󰌾 {}", security),
            50,
            72,
            8.5,
            muted_text,
        );

        // Status / Error banner or instruction
        if prompt.is_connecting {
            SoftwareRenderer::draw_rounded_rect(
                buffer,
                self.width,
                self.height,
                16,
                92,
                308,
                26,
                4,
                parse_hex_color("#1e2030"),
                Some(accent_color),
                1,
            );
            let waves = match (anim_frame / 3) % 4 {
                0 => "󰤟",
                1 => "󰤢",
                2 => "󰤥",
                _ => "󰤨",
            };
            renderer.draw_text(
                buffer,
                self.width,
                self.height,
                &format!("{} Connecting to \"{}\"...", waves, short_ssid),
                24,
                109,
                8.5,
                accent_color,
            );
        } else if let Some(ref err) = prompt.error_message {
            SoftwareRenderer::draw_rounded_rect(
                buffer,
                self.width,
                self.height,
                16,
                92,
                308,
                26,
                4,
                parse_hex_color("#1e1e2e"),
                Some(red_color),
                1,
            );
            let short_err: String = err.chars().take(34).collect();
            renderer.draw_text(
                buffer,
                self.width,
                self.height,
                &format!("󰅖 {}", short_err),
                24,
                109,
                8.5,
                red_color,
            );
        } else {
            renderer.draw_text(
                buffer,
                self.width,
                self.height,
                "Enter network password:",
                18,
                110,
                9.0,
                muted_text,
            );
        }

        // Password Input Box
        SoftwareRenderer::draw_rounded_rect(
            buffer,
            self.width,
            self.height,
            16,
            126,
            308,
            42,
            6,
            parse_hex_color("#11111b"),
            Some(accent_color),
            2,
        );

        renderer.draw_text(buffer, self.width, self.height, "󰌾", 26, 152, 11.5, muted_text);

        let masked_buf;
        let (display_str, text_fg) = if prompt.password.is_empty() {
            ("Enter password...", parse_hex_color("#585b70"))
        } else if prompt.show_password {
            let s: String = prompt.password.chars().rev().take(22).collect::<Vec<_>>().into_iter().rev().collect();
            masked_buf = s;
            (masked_buf.as_str(), text_color)
        } else {
            let bullets = "●".repeat(prompt.password.len().min(22));
            masked_buf = bullets;
            (masked_buf.as_str(), text_color)
        };

        let text_advance = renderer.draw_text(
            buffer,
            self.width,
            self.height,
            display_str,
            46,
            152,
            10.5,
            text_fg,
        );

        // Blinking cursor
        if !prompt.is_connecting && (anim_frame / 6) % 2 == 0 {
            let cursor_x = if prompt.password.is_empty() {
                46
            } else {
                46 + text_advance as i32 + 2
            };
            SoftwareRenderer::draw_rect(buffer, self.width, self.height, cursor_x, 137, 2, 20, accent_color);
        }

        // Show/Hide password toggle eye button (x: 278..316, y: 132..162)
        SoftwareRenderer::draw_rounded_rect(
            buffer,
            self.width,
            self.height,
            278,
            132,
            38,
            30,
            4,
            block_bg,
            Some(border_subtle),
            1,
        );
        let (eye_icon, eye_fg) = if prompt.show_password {
            ("󰈈", accent_color)
        } else {
            ("󰈉", muted_text)
        };
        renderer.draw_text(buffer, self.width, self.height, eye_icon, 290, 151, 11.0, eye_fg);

        // Shortcut hint
        renderer.draw_text(
            buffer,
            self.width,
            self.height,
            "Press Enter to connect, Esc to cancel",
            18,
            188,
            8.5,
            muted_text,
        );

        // Action Buttons Row [Cancel] [Connect] (y: 202..242)
        // Cancel button
        SoftwareRenderer::draw_rounded_rect(
            buffer,
            self.width,
            self.height,
            16,
            202,
            146,
            40,
            6,
            block_bg,
            Some(border_subtle),
            1,
        );
        renderer.draw_text(buffer, self.width, self.height, "Cancel", 68, 226, 10.0, text_color);

        // Connect button
        if prompt.is_connecting {
            SoftwareRenderer::draw_rounded_rect(
                buffer,
                self.width,
                self.height,
                178,
                202,
                146,
                40,
                6,
                parse_hex_color("#313244"),
                Some(accent_color),
                1,
            );
            renderer.draw_text(buffer, self.width, self.height, "Connecting...", 212, 226, 9.5, accent_color);
        } else if prompt.password.is_empty() {
            SoftwareRenderer::draw_rounded_rect(
                buffer,
                self.width,
                self.height,
                178,
                202,
                146,
                40,
                6,
                block_bg,
                Some(border_subtle),
                1,
            );
            renderer.draw_text(buffer, self.width, self.height, "Connect", 228, 226, 10.0, parse_hex_color("#585b70"));
        } else {
            SoftwareRenderer::draw_rounded_rect(
                buffer,
                self.width,
                self.height,
                178,
                202,
                146,
                40,
                6,
                accent_color,
                None,
                0,
            );
            renderer.draw_text(buffer, self.width, self.height, "󰄬 Connect", 220, 226, 10.0, parse_hex_color("#11111b"));
        }

        // Info Footer
        SoftwareRenderer::draw_rect(buffer, self.width, self.height, 16, 264, 308, 1, border_subtle);
        renderer.draw_text(
            buffer,
            self.width,
            self.height,
            "󰌾 Security: WPA / WPA2 / WPA3 Personal",
            20,
            290,
            8.5,
            muted_text,
        );
        renderer.draw_text(
            buffer,
            self.width,
            self.height,
            "󰖩 Credentials will be saved by NetworkManager",
            20,
            312,
            8.5,
            muted_text,
        );
    }
}

// -------------------------------- Unified Bar Popover --------------------------------

#[derive(Debug, Clone)]
pub enum PopoverKind {
    None,
    Workspace(WorkspacePopup),
    Audio(AudioPopover),
    Wifi(WifiPopover),
}

pub struct BarPopover {
    pub kind: PopoverKind,
    pub visible: bool,
    pub anchor_x: i32,
    pub width: u32,
    pub height: u32,
    pub anim_frame: u32,
}

impl BarPopover {
    pub fn new() -> Self {
        Self {
            kind: PopoverKind::None,
            visible: false,
            anchor_x: 0,
            width: 260,
            height: 160,
            anim_frame: 0,
        }
    }

    pub fn render(
        &self,
        renderer: &mut SoftwareRenderer,
        buffer: &mut [u8],
        config: &Config,
    ) {
        if !self.visible {
            return;
        }
        match &self.kind {
            PopoverKind::None => {}
            PopoverKind::Workspace(ws) => ws.render(renderer, buffer, config),
            PopoverKind::Audio(audio) => audio.render(renderer, buffer, config, self.anim_frame),
            PopoverKind::Wifi(wifi) => wifi.render(renderer, buffer, config, self.anim_frame),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_popover_clicks() {
        let pop = AudioPopover::new(320, 240, 50, false, false, "Test Sink".into());
        // Close button click
        assert_eq!(pop.handle_click(290, 15), AudioPopoverAction::Close);
        // Step -5%
        assert_eq!(pop.handle_click(30, 100), AudioPopoverAction::VolumeDelta(-5));
        // Step +5%
        assert_eq!(pop.handle_click(90, 100), AudioPopoverAction::VolumeDelta(5));
        // Mute toggle
        assert_eq!(pop.handle_click(150, 100), AudioPopoverAction::ToggleMute);
        // Preset 75%
        assert_eq!(pop.handle_click(200, 130), AudioPopoverAction::SetVolume(75));
        // Mic mute toggle
        assert_eq!(pop.handle_click(50, 190), AudioPopoverAction::ToggleMicMute);
    }

    #[test]
    fn test_wifi_popover_clicks() {
        let net1 = WifiNetworkItem {
            in_use: true,
            ssid: "HomeWiFi".into(),
            signal: 85,
            security: "WPA2".into(),
            is_saved: true,
        };
        let net2 = WifiNetworkItem {
            in_use: false,
            ssid: "CoffeeShop".into(),
            signal: 60,
            security: "WPA2".into(),
            is_saved: false,
        };
        let pop = WifiPopover::new(340, 350, true, "HomeWiFi".into(), 85, vec![net1, net2]);
        // Close button click
        assert_eq!(pop.handle_click(310, 15), WifiPopoverAction::Close);
        // Rescan button
        assert_eq!(pop.handle_click(250, 15), WifiPopoverAction::Rescan);
        // Radio toggle
        assert_eq!(pop.handle_click(250, 50), WifiPopoverAction::ToggleRadio);
        // Forget button click for saved network row 0 (x: 288..322, y: 94..128)
        assert_eq!(pop.handle_click(300, 110), WifiPopoverAction::Forget("HomeWiFi".into()));
        // Connected item click -> Disconnect
        assert_eq!(pop.handle_click(50, 110), WifiPopoverAction::Disconnect("HomeWiFi".into()));
        // Row 1 is not saved: clicking at x: 300 or x: 50 should trigger Connect, not Forget
        assert_eq!(pop.handle_click(300, 140), WifiPopoverAction::Connect("CoffeeShop".into()));
        assert_eq!(pop.handle_click(50, 140), WifiPopoverAction::Connect("CoffeeShop".into()));
        // Advanced nmtui button
        assert_eq!(pop.handle_click(50, 320), WifiPopoverAction::OpenNmtui);
    }

    #[test]
    fn test_popover_animation_rendering() {
        let mut renderer = SoftwareRenderer::new("");
        let mut buffer = vec![0u8; 340 * 350 * 4];
        let config = Config::default();

        let net = WifiNetworkItem {
            in_use: true,
            ssid: "HomeWiFi".into(),
            signal: 85,
            security: "WPA2".into(),
            is_saved: true,
        };
        let mut pop = WifiPopover::new(340, 350, true, "HomeWiFi".into(), 85, vec![net]);
        pop.is_scanning = true;
        pop.status_message = Some("Connecting to HomeWiFi...".into());

        // Render at frame 0 and frame 15 - ensures no panic with animation math
        pop.render(&mut renderer, &mut buffer, &config, 0);
        pop.render(&mut renderer, &mut buffer, &config, 15);

        // Render with error banner (incorrect password feedback)
        pop.status_message = Some("Incorrect password for HomeWiFi".into());
        pop.render(&mut renderer, &mut buffer, &config, 20);

        let audio_pop = AudioPopover::new(320, 240, 60, false, false, "Test Sink".into());
        let mut audio_buf = vec![0u8; 320 * 240 * 4];
        audio_pop.render(&mut renderer, &mut audio_buf, &config, 0);
        audio_pop.render(&mut renderer, &mut audio_buf, &config, 30);
    }

    #[test]
    fn test_wifi_popover_password_prompt_clicks() {
        let net = WifiNetworkItem {
            in_use: false,
            ssid: "CoffeeShop".into(),
            signal: 70,
            security: "WPA2".into(),
            is_saved: false,
        };
        let mut pop = WifiPopover::new(340, 350, true, "CoffeeShop".into(), 70, vec![net]);
        pop.password_prompt = Some(WifiPasswordPrompt {
            ssid: "CoffeeShop".into(),
            password: "".into(),
            show_password: false,
            error_message: None,
            is_connecting: false,
        });

        // Close button click [✕] at top-right
        assert_eq!(pop.handle_click(310, 15), WifiPopoverAction::Close);

        // Back button click [← Back]
        assert_eq!(pop.handle_click(50, 15), WifiPopoverAction::CancelPasswordPrompt);

        // Cancel button click
        assert_eq!(pop.handle_click(80, 220), WifiPopoverAction::CancelPasswordPrompt);

        // Eye icon toggle show password click
        assert_eq!(pop.handle_click(295, 145), WifiPopoverAction::ToggleShowPassword);

        // Connect button when password is empty: does not submit
        assert_eq!(pop.handle_click(250, 220), WifiPopoverAction::None);

        // Provide password
        pop.password_prompt.as_mut().unwrap().password = "SecretPass123".into();

        // Connect button with password -> SubmitPassword
        assert_eq!(
            pop.handle_click(250, 220),
            WifiPopoverAction::SubmitPassword {
                ssid: "CoffeeShop".into(),
                password: "SecretPass123".into(),
            }
        );
    }

    #[test]
    fn test_wifi_popover_keyboard_input() {
        let mut pop = WifiPopover::new(340, 350, true, "".into(), 0, vec![]);
        pop.password_prompt = Some(WifiPasswordPrompt {
            ssid: "OfficeWiFi".into(),
            password: "".into(),
            show_password: false,
            error_message: Some("Invalid password".into()),
            is_connecting: false,
        });

        // Type characters
        assert_eq!(pop.handle_key_input("abc", false, false, false, false), WifiPopoverAction::PasswordChanged);
        assert_eq!(pop.password_prompt.as_ref().unwrap().password, "abc");
        assert_eq!(pop.password_prompt.as_ref().unwrap().error_message, None);

        // Backspace
        assert_eq!(pop.handle_key_input("", true, false, false, false), WifiPopoverAction::PasswordChanged);
        assert_eq!(pop.password_prompt.as_ref().unwrap().password, "ab");

        // Clear
        assert_eq!(pop.handle_key_input("", false, false, false, true), WifiPopoverAction::PasswordChanged);
        assert_eq!(pop.password_prompt.as_ref().unwrap().password, "");

        // Enter with empty password
        assert_eq!(pop.handle_key_input("", false, true, false, false), WifiPopoverAction::None);

        // Type password and hit enter
        pop.password_prompt.as_mut().unwrap().password = "mypassword".into();
        assert_eq!(
            pop.handle_key_input("", false, true, false, false),
            WifiPopoverAction::SubmitPassword {
                ssid: "OfficeWiFi".into(),
                password: "mypassword".into(),
            }
        );
        assert!(pop.password_prompt.as_ref().unwrap().is_connecting);

        // Escape cancels
        assert_eq!(pop.handle_key_input("", false, false, true, false), WifiPopoverAction::CancelPasswordPrompt);
        assert!(pop.password_prompt.is_none());
    }

    #[test]
    fn test_wifi_popover_password_prompt_rendering() {
        let mut renderer = SoftwareRenderer::new("");
        let mut buffer = vec![0u8; 340 * 350 * 4];
        let config = Config::default();

        let mut pop = WifiPopover::new(340, 350, true, "".into(), 0, vec![]);
        pop.password_prompt = Some(WifiPasswordPrompt {
            ssid: "MyProtectedNetwork".into(),
            password: "supersecretpassword".into(),
            show_password: false,
            error_message: None,
            is_connecting: false,
        });

        // Render masked password at frame 0 and frame 6 (cursor blink test)
        pop.render(&mut renderer, &mut buffer, &config, 0);
        pop.render(&mut renderer, &mut buffer, &config, 6);

        // Render with password visible
        pop.password_prompt.as_mut().unwrap().show_password = true;
        pop.render(&mut renderer, &mut buffer, &config, 10);

        // Render with error message
        pop.password_prompt.as_mut().unwrap().error_message = Some("Incorrect password. Please try again.".into());
        pop.render(&mut renderer, &mut buffer, &config, 15);

        // Render while connecting
        pop.password_prompt.as_mut().unwrap().is_connecting = true;
        pop.render(&mut renderer, &mut buffer, &config, 20);
    }
}
