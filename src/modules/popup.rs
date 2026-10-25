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
        if !self.visible {
            return;
        }

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

                // Draw miniature label (app class)
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
