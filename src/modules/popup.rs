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

        let pad_x = 8i32;
        let pad_top = 24i32; // space for header
        let pad_bot = 8i32;

        let inner_w = (self.width as i32 - pad_x * 2).max(10) as f32;
        let inner_h = (self.height as i32 - pad_top - pad_bot).max(10) as f32;

        let mon_w_f = mon_w.max(640) as f32;
        let mon_h_f = mon_h.max(480) as f32;

        let scale_x = inner_w / mon_w_f;
        let scale_y = inner_h / mon_h_f;

        for c in clients {
            if c.workspace.id != ws_id {
                continue;
            }

            // Normalise coordinate within the monitor bounds
            let rx = (c.at.0 % mon_w as i32).max(0) as f32;
            let ry = (c.at.1 % mon_h as i32).max(0) as f32;

            let x = pad_x + (rx * scale_x) as i32;
            let y = pad_top + (ry * scale_y) as i32;
            let w = ((c.size.0 as f32 * scale_x).round() as u32).max(24).min(self.width - pad_x as u32 * 2);
            let h = ((c.size.1 as f32 * scale_y).round() as u32).max(18).min(self.height - pad_top as u32 - pad_bot as u32);

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

        // Draw popup background with border
        SoftwareRenderer::draw_rounded_rect(
            buffer,
            self.width,
            self.height,
            0,
            0,
            self.width,
            self.height,
            6,
            bg_color,
            Some(border_color),
            2,
        );

        // Draw header title
        let title = format!("Workspace {} Preview", self.workspace_id);
        renderer.draw_text(
            buffer,
            self.width,
            self.height,
            &title,
            12,
            16,
            11.0,
            header_fg,
        );

        // Draw miniature window wireframes
        if self.thumbnails.is_empty() {
            renderer.draw_text(
                buffer,
                self.width,
                self.height,
                "No open windows",
                self.width as i32 / 2 - 40,
                self.height as i32 / 2 + 5,
                11.0,
                thumb_border,
            );
        } else {
            for thumb in &self.thumbnails {
                SoftwareRenderer::draw_rounded_rect(
                    buffer,
                    self.width,
                    self.height,
                    thumb.x,
                    thumb.y,
                    thumb.width,
                    thumb.height,
                    3,
                    thumb_bg,
                    Some(thumb_border),
                    1,
                );

                let label = if !thumb.class.is_empty() {
                    &thumb.class
                } else if !thumb.title.is_empty() {
                    &thumb.title
                } else {
                    "window"
                };

                let max_chars = (thumb.width / 7).max(1) as usize;
                let short_label: String = label.chars().take(max_chars).collect();

                renderer.draw_text(
                    buffer,
                    self.width,
                    self.height,
                    &short_label,
                    thumb.x + 4,
                    thumb.y + thumb.height as i32 / 2 + 4,
                    9.0,
                    text_fg,
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
        let mic_status = if self.source_muted { "MUTED" } else { "ACTIVE" };
        let mic_status_color = if self.source_muted { red_color } else { green_color };
        renderer.draw_text(buffer, self.width, self.height, mic_status, 258, 172, 9.5, mic_status_color);

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

#[derive(Debug, Clone)]
pub struct WifiNetworkItem {
    pub in_use: bool,
    pub ssid: String,
    pub signal: u32,
    pub security: String,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WifiPopoverAction {
    None,
    ToggleRadio,
    Rescan,
    Connect(String),
    Disconnect(String),
    OpenNmtui,
    Close,
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
        }
    }

    pub fn handle_click(&self, click_x: i32, click_y: i32) -> WifiPopoverAction {
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
            if click_x >= 16 && click_x <= 324 && click_y >= row_y && click_y <= row_y + 34 {
                if net.in_use {
                    return WifiPopoverAction::Disconnect(net.ssid.clone());
                } else {
                    return WifiPopoverAction::Connect(net.ssid.clone());
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

        // Rescan button
        SoftwareRenderer::draw_rounded_rect(
            buffer,
            self.width,
            self.height,
            236,
            8,
            60,
            22,
            4,
            block_bg,
            Some(border_subtle),
            1,
        );
        renderer.draw_text(buffer, self.width, self.height, "󰑐 Scan", 244, 22, 9.0, text_color);

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

        // Divider
        SoftwareRenderer::draw_rect(buffer, self.width, self.height, 16, 72, 308, 1, border_subtle);

        // Available Networks subtitle
        renderer.draw_text(buffer, self.width, self.height, "Available Networks", 16, 86, 9.5, muted_text);

        // Networks list
        if self.networks.is_empty() {
            let msg = if !self.wifi_enabled {
                "Wi-Fi is turned off"
            } else {
                "No Wi-Fi networks found"
            };
            renderer.draw_text(buffer, self.width, self.height, msg, 16, 130, 10.5, muted_text);
        } else {
            for (i, net) in self.networks.iter().take(5).enumerate() {
                let row_y = 94 + (i as i32) * 38;
                let (row_bg, row_border, row_border_w) = if net.in_use {
                    (parse_hex_color("#1e2030"), parse_hex_color(&config.colors.active_workspace_bg), 1)
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

                // Security label
                let sec_text = if net.security.is_empty() || net.security == "--" {
                    "Open Network".to_string()
                } else {
                    format!("󰌾 {}", net.security)
                };
                renderer.draw_text(buffer, self.width, self.height, &sec_text, 44, row_y + 29, 8.0, muted_text);

                // Action badge
                if net.in_use {
                    SoftwareRenderer::draw_rounded_rect(
                        buffer,
                        self.width,
                        self.height,
                        226,
                        row_y + 6,
                        90,
                        22,
                        4,
                        accent_color,
                        None,
                        0,
                    );
                    renderer.draw_text(buffer, self.width, self.height, "󰄬 Connected", 232, row_y + 20, 8.5, parse_hex_color("#11111b"));
                } else {
                    SoftwareRenderer::draw_rounded_rect(
                        buffer,
                        self.width,
                        self.height,
                        248,
                        row_y + 6,
                        68,
                        22,
                        4,
                        block_bg,
                        Some(border_subtle),
                        1,
                    );
                    renderer.draw_text(buffer, self.width, self.height, "Connect", 260, row_y + 20, 8.5, text_color);
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
}

impl BarPopover {
    pub fn new() -> Self {
        Self {
            kind: PopoverKind::None,
            visible: false,
            anchor_x: 0,
            width: 260,
            height: 160,
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
            PopoverKind::Audio(audio) => audio.render(renderer, buffer, config),
            PopoverKind::Wifi(wifi) => wifi.render(renderer, buffer, config),
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
        let net = WifiNetworkItem {
            in_use: true,
            ssid: "HomeWiFi".into(),
            signal: 85,
            security: "WPA2".into(),
        };
        let pop = WifiPopover::new(340, 350, true, "HomeWiFi".into(), 85, vec![net]);
        // Close button click
        assert_eq!(pop.handle_click(310, 15), WifiPopoverAction::Close);
        // Rescan button
        assert_eq!(pop.handle_click(250, 15), WifiPopoverAction::Rescan);
        // Radio toggle
        assert_eq!(pop.handle_click(250, 50), WifiPopoverAction::ToggleRadio);
        // Connected item click -> Disconnect
        assert_eq!(pop.handle_click(50, 110), WifiPopoverAction::Disconnect("HomeWiFi".into()));
        // Advanced nmtui button
        assert_eq!(pop.handle_click(50, 320), WifiPopoverAction::OpenNmtui);
    }
}
