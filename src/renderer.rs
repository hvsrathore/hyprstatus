#![allow(dead_code)]
use std::collections::HashMap;
use std::fs;
use std::path::Path;

pub struct SoftwareRenderer {
    pub font: fontdue::Font,
    glyph_cache: HashMap<(char, u32), (fontdue::Metrics, Vec<u8>)>,
}

#[inline(always)]
fn blend_pixel(dst: &mut [u8], src_argb: u32) {
    let src_a = ((src_argb >> 24) & 0xFF) as u32;
    if src_a == 0 {
        return;
    }
    let src_r = ((src_argb >> 16) & 0xFF) as u32;
    let src_g = ((src_argb >> 8) & 0xFF) as u32;
    let src_b = (src_argb & 0xFF) as u32;

    if src_a >= 255 {
        dst[0] = src_b as u8;
        dst[1] = src_g as u8;
        dst[2] = src_r as u8;
        dst[3] = 255;
        return;
    }

    let inv_a = 255 - src_a;
    let dst_b = dst[0] as u32;
    let dst_g = dst[1] as u32;
    let dst_r = dst[2] as u32;
    let dst_a = dst[3] as u32;

    let out_b = (src_b * src_a + dst_b * inv_a) / 255;
    let out_g = (src_g * src_a + dst_g * inv_a) / 255;
    let out_r = (src_r * src_a + dst_r * inv_a) / 255;
    let out_a = src_a + (dst_a * inv_a) / 255;

    dst[0] = out_b as u8;
    dst[1] = out_g as u8;
    dst[2] = out_r as u8;
    dst[3] = out_a.min(255) as u8;
}

impl SoftwareRenderer {
    pub fn new(font_path: &str) -> Self {
        let paths_to_try = [
            font_path,
            "/usr/share/fonts/TTF/JetBrainsMonoNerdFontPropo-Bold.ttf",
            "/usr/share/fonts/TTF/JetBrainsMonoNLNerdFontPropo-Regular.ttf",
            "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
            "/usr/share/fonts/noto/NotoSans-Regular.ttf",
        ];

        let mut font_bytes = None;
        for p in paths_to_try {
            if Path::new(p).exists() {
                if let Ok(b) = fs::read(p) {
                    font_bytes = Some(b);
                    break;
                }
            }
        }

        let font = match font_bytes {
            Some(bytes) => fontdue::Font::from_bytes(bytes, fontdue::FontSettings::default())
                .unwrap_or_else(|e| panic!("Failed to parse font: {}", e)),
            None => panic!("Could not find any usable TrueType font on system"),
        };

        Self {
            font,
            glyph_cache: HashMap::with_capacity(256),
        }
    }

    pub fn clear(buffer: &mut [u8], color: u32) {
        let b = (color & 0xFF) as u8;
        let g = ((color >> 8) & 0xFF) as u8;
        let r = ((color >> 16) & 0xFF) as u8;
        let a = ((color >> 24) & 0xFF) as u8;

        for chunk in buffer.chunks_exact_mut(4) {
            chunk[0] = b;
            chunk[1] = g;
            chunk[2] = r;
            chunk[3] = a;
        }
    }

    pub fn draw_rect(buffer: &mut [u8], buf_w: u32, buf_h: u32, x: i32, y: i32, w: u32, h: u32, color: u32) {
        let x0 = x.max(0) as u32;
        let y0 = y.max(0) as u32;
        let x1 = ((x + w as i32).max(0) as u32).min(buf_w);
        let y1 = ((y + h as i32).max(0) as u32).min(buf_h);

        for cy in y0..y1 {
            let row_offset = (cy * buf_w * 4) as usize;
            for cx in x0..x1 {
                let offset = row_offset + (cx * 4) as usize;
                blend_pixel(&mut buffer[offset..offset + 4], color);
            }
        }
    }

    pub fn draw_rounded_rect(
        buffer: &mut [u8],
        buf_w: u32,
        buf_h: u32,
        x: i32,
        y: i32,
        w: u32,
        h: u32,
        radius: u32,
        fill_color: u32,
        border_color: Option<u32>,
        border_width: u32,
    ) {
        let r = radius.min(w / 2).min(h / 2) as i32;
        let x0 = x.max(0);
        let y0 = y.max(0);
        let x1 = (x + w as i32).min(buf_w as i32);
        let y1 = (y + h as i32).min(buf_h as i32);

        let r_sq = r * r;
        let inner_r = (r - border_width as i32).max(0);
        let inner_r_sq = inner_r * inner_r;

        for cy in y0..y1 {
            let row_offset = (cy as u32 * buf_w * 4) as usize;
            for cx in x0..x1 {
                let offset = row_offset + (cx as u32 * 4) as usize;

                // Check corners
                let mut is_border = false;
                let mut is_inside = true;

                let dx_left = cx - (x + r);
                let dx_right = cx - (x + w as i32 - 1 - r);
                let dy_top = cy - (y + r);
                let dy_bottom = cy - (y + h as i32 - 1 - r);

                let in_left_corner = cx < x + r;
                let in_right_corner = cx > x + w as i32 - 1 - r;
                let in_top_corner = cy < y + r;
                let in_bottom_corner = cy > y + h as i32 - 1 - r;

                if (in_left_corner || in_right_corner) && (in_top_corner || in_bottom_corner) {
                    let c_dx = if in_left_corner { dx_left } else { dx_right };
                    let c_dy = if in_top_corner { dy_top } else { dy_bottom };
                    let dist_sq = c_dx * c_dx + c_dy * c_dy;

                    if dist_sq > r_sq {
                        is_inside = false;
                    } else if border_color.is_some() && dist_sq > inner_r_sq {
                        is_border = true;
                    }
                } else if border_color.is_some() && border_width > 0 {
                    if cx < x + border_width as i32
                        || cx >= x + w as i32 - border_width as i32
                        || cy < y + border_width as i32
                        || cy >= y + h as i32 - border_width as i32
                    {
                        is_border = true;
                    }
                }

                if is_inside {
                    if is_border {
                        if let Some(bc) = border_color {
                            blend_pixel(&mut buffer[offset..offset + 4], bc);
                        }
                    } else {
                        blend_pixel(&mut buffer[offset..offset + 4], fill_color);
                    }
                }
            }
        }
    }

    pub fn measure_text(&mut self, text: &str, size: f32) -> (u32, u32) {
        let size_u32 = (size * 10.0) as u32;
        let mut width = 0.0f32;
        let mut max_height = size;

        for ch in text.chars() {
            let (metrics, _) = self.glyph_cache.entry((ch, size_u32)).or_insert_with(|| {
                self.font.rasterize(ch, size)
            });
            width += metrics.advance_width;
            max_height = max_height.max(metrics.height as f32);
        }

        (width.ceil() as u32, max_height.ceil() as u32)
    }

    pub fn draw_text(
        &mut self,
        buffer: &mut [u8],
        buf_w: u32,
        buf_h: u32,
        text: &str,
        start_x: i32,
        baseline_y: i32,
        size: f32,
        color: u32,
    ) -> u32 {
        let size_u32 = (size * 10.0) as u32;
        let mut cur_x = start_x as f32;

        let src_r = ((color >> 16) & 0xFF) as u32;
        let src_g = ((color >> 8) & 0xFF) as u32;
        let src_b = (color & 0xFF) as u32;
        let src_a = ((color >> 24) & 0xFF) as u32;

        for ch in text.chars() {
            let (metrics, bitmap) = self.glyph_cache.entry((ch, size_u32)).or_insert_with(|| {
                self.font.rasterize(ch, size)
            });

            let glyph_x = cur_x.round() as i32 + metrics.xmin;
            let glyph_y = baseline_y - metrics.height as i32 - metrics.ymin;

            let gw = metrics.width;
            let gh = metrics.height;

            for gy in 0..gh {
                let py = glyph_y + gy as i32;
                if py < 0 || py >= buf_h as i32 {
                    continue;
                }
                let row_offset = (py as u32 * buf_w * 4) as usize;

                for gx in 0..gw {
                    let px = glyph_x + gx as i32;
                    if px < 0 || px >= buf_w as i32 {
                        continue;
                    }

                    let cov = bitmap[gy * gw + gx] as u32;
                    if cov == 0 {
                        continue;
                    }

                    let alpha = (src_a * cov) / 255;
                    let pixel_argb = (alpha << 24) | (src_r << 16) | (src_g << 8) | src_b;

                    let offset = row_offset + (px as u32 * 4) as usize;
                    blend_pixel(&mut buffer[offset..offset + 4], pixel_argb);
                }
            }

            cur_x += metrics.advance_width;
        }

        (cur_x.round() as i32 - start_x).max(0) as u32
    }

    pub fn draw_pixmap(
        buffer: &mut [u8],
        buf_w: u32,
        buf_h: u32,
        pixmap_data: &[u8],
        src_w: u32,
        src_h: u32,
        dst_x: i32,
        dst_y: i32,
        dst_w: u32,
        dst_h: u32,
    ) {
        if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
            return;
        }

        for dy in 0..dst_h {
            let py = dst_y + dy as i32;
            if py < 0 || py >= buf_h as i32 {
                continue;
            }
            let sy = (dy * src_h) / dst_h;
            let row_offset = (py as u32 * buf_w * 4) as usize;

            for dx in 0..dst_w {
                let px = dst_x + dx as i32;
                if px < 0 || px >= buf_w as i32 {
                    continue;
                }
                let sx = (dx * src_w) / dst_w;
                let src_idx = ((sy * src_w + sx) * 4) as usize;

                if src_idx + 4 <= pixmap_data.len() {
                    // SNI pixmap is network byte order ARGB or BGRA
                    let a = pixmap_data[src_idx] as u32;
                    let r = pixmap_data[src_idx + 1] as u32;
                    let g = pixmap_data[src_idx + 2] as u32;
                    let b = pixmap_data[src_idx + 3] as u32;
                    let argb = (a << 24) | (r << 16) | (g << 8) | b;

                    let dst_offset = row_offset + (px as u32 * 4) as usize;
                    blend_pixel(&mut buffer[dst_offset..dst_offset + 4], argb);
                }
            }
        }
    }

    /// Renders rolling time-series values as smooth Unicode Braille patterns (⣀⣤⣾⣿)
    /// Each Braille character holds 2 sequential data points across 4 vertical dots.
    pub fn render_braille_sparkline(values: &[f32], max_val: f32) -> String {
        let mut result = String::with_capacity(values.len() / 2 + 1);
        let max_val = if max_val <= 0.0 { 1.0 } else { max_val };

        // Left column dot values: 0 to 4 dots filled from bottom:
        // 0: none, 1: dot7 (0x40), 2: dot7+dot3 (0x44), 3: dot7+dot3+dot2 (0x46), 4: dot7+dot3+dot2+dot1 (0x47)
        const LEFT_DOTS: [u32; 5] = [0, 0x40, 0x44, 0x46, 0x47];
        // Right column dot values:
        // 0: none, 1: dot8 (0x80), 2: dot8+dot6 (0xA0), 3: dot8+dot6+dot5 (0xB0), 4: dot8+dot6+dot5+dot4 (0xB8)
        const RIGHT_DOTS: [u32; 5] = [0, 0x80, 0xA0, 0xB0, 0xB8];

        let mut i = 0;
        while i < values.len() {
            let v1 = (values[i] / max_val).clamp(0.0, 1.0);
            let lvl1 = (v1 * 4.0).round() as usize;

            let lvl2 = if i + 1 < values.len() {
                let v2 = (values[i + 1] / max_val).clamp(0.0, 1.0);
                (v2 * 4.0).round() as usize
            } else {
                lvl1
            };

            let code = 0x2800 + LEFT_DOTS[lvl1.min(4)] + RIGHT_DOTS[lvl2.min(4)];
            if let Some(ch) = char::from_u32(code) {
                result.push(ch);
            } else {
                result.push(' ');
            }
            i += 2;
        }

        result
    }
}
