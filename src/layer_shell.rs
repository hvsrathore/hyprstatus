use crate::config::{parse_hex_color, Config};
use crate::ipc::HyprlandIpc;
use crate::modules::audio::AudioModule;
use crate::modules::battery::BatteryModule;
use crate::modules::clock::ClockModule;
use crate::modules::network::NetworkModule;
use crate::modules::perf::PerfModule;
use crate::modules::popup::WorkspacePopup;
use crate::modules::tray::TrayModule;
use crate::modules::window_title::WindowTitleModule;
use crate::modules::workspace::WorkspaceModule;
use crate::modules::{ClickAction, ClickRegion, ModuleBlock, MouseButton};
use crate::renderer::SoftwareRenderer;
use std::collections::HashMap;
use std::os::fd::AsFd;
use std::sync::Arc;
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_output, wl_pointer, wl_registry, wl_seat, wl_shm, wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1::{self, ZwlrLayerShellV1},
    zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1},
};

pub struct ShmBuffer {
    pub buffer: wl_buffer::WlBuffer,
    pub mmap: memmap2::MmapMut,
    pub width: u32,
    pub height: u32,
}

impl ShmBuffer {
    pub fn new(
        shm: &wl_shm::WlShm,
        qh: &QueueHandle<BarState>,
        width: u32,
        height: u32,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let stride = width * 4;
        let size = stride * height;

        let fd = rustix::fs::memfd_create(
            "hyprstatus-shm",
            rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
        )?;
        rustix::fs::ftruncate(&fd, size as u64)?;

        let mmap = unsafe { memmap2::MmapMut::map_mut(&fd)? };
        let pool = shm.create_pool(fd.as_fd(), size as i32, qh, ());
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride as i32,
            wl_shm::Format::Argb8888,
            qh,
            (),
        );
        pool.destroy();

        Ok(Self {
            buffer,
            mmap,
            width,
            height,
        })
    }
}

pub struct BarOutput {
    pub name: String,
    pub output: wl_output::WlOutput,
    pub width: u32,
    pub height: u32,
    pub scale: f32,
    pub surface: wl_surface::WlSurface,
    pub layer_surface: ZwlrLayerSurfaceV1,
    pub configured: bool,
    pub buffer: Option<ShmBuffer>,
    pub click_regions: Vec<ClickRegion>,
    // Popup state for this output
    pub popup: WorkspacePopup,
    pub popup_surface: Option<wl_surface::WlSurface>,
    pub popup_layer_surface: Option<ZwlrLayerSurfaceV1>,
    pub popup_buffer: Option<ShmBuffer>,
    pub popup_configured: bool,
}

pub struct BarState {
    pub config: Arc<Config>,
    pub compositor: Option<wl_compositor::WlCompositor>,
    pub shm: Option<wl_shm::WlShm>,
    pub layer_shell: Option<ZwlrLayerShellV1>,
    pub seat: Option<wl_seat::WlSeat>,
    pub pointer: Option<wl_pointer::WlPointer>,
    pub outputs: HashMap<u32, (wl_output::WlOutput, Option<String>)>, // registry_id -> (output, name)
    pub bars: HashMap<u32, BarOutput>, // registry_id -> BarOutput
    pub renderer: SoftwareRenderer,

    // Modules
    pub mod_workspace: WorkspaceModule,
    pub mod_window_title: WindowTitleModule,
    pub mod_audio: AudioModule,
    pub mod_network: NetworkModule,
    pub mod_battery: BatteryModule,
    pub mod_perf: PerfModule,
    pub mod_clock: ClockModule,
    pub mod_tray: TrayModule,

    // Pointer tracking
    pub pointer_surface: Option<wl_surface::WlSurface>,
    pub pointer_x: f64,
    pub pointer_y: f64,
    pub hovered_ws_pill: Option<(u32, i32, i32, u32)>, // (output_id, ws_id, x, w)
}

impl BarState {
    pub fn new(
        config: Arc<Config>,
        mod_workspace: WorkspaceModule,
        mod_window_title: WindowTitleModule,
        mod_audio: AudioModule,
        mod_network: NetworkModule,
        mod_battery: BatteryModule,
        mod_perf: PerfModule,
        mod_clock: ClockModule,
        mod_tray: TrayModule,
    ) -> Self {
        let renderer = SoftwareRenderer::new(&config.bar.font_path);
        Self {
            config,
            compositor: None,
            shm: None,
            layer_shell: None,
            seat: None,
            pointer: None,
            outputs: HashMap::new(),
            bars: HashMap::new(),
            renderer,
            mod_workspace,
            mod_window_title,
            mod_audio,
            mod_network,
            mod_battery,
            mod_perf,
            mod_clock,
            mod_tray,
            pointer_surface: None,
            pointer_x: 0.0,
            pointer_y: 0.0,
            hovered_ws_pill: None,
        }
    }

    pub fn init_bar_for_output(&mut self, qh: &QueueHandle<Self>, registry_id: u32) {
        let (output, name_opt) = match self.outputs.get(&registry_id) {
            Some(o) => (o.0.clone(), o.1.clone().unwrap_or_else(|| format!("output-{}", registry_id))),
            None => return,
        };

        // Check if output is disabled in config
        if let Some(out_cfg) = self.config.output.get(&name_opt) {
            if !out_cfg.enabled {
                println!("[layer_shell] Output {} disabled by config, skipping bar creation", name_opt);
                return;
            }
        }

        let compositor = match &self.compositor {
            Some(c) => c,
            None => return,
        };
        let layer_shell = match &self.layer_shell {
            Some(ls) => ls,
            None => return,
        };

        let surface = compositor.create_surface(qh, ());
        let layer_surface = layer_shell.get_layer_surface(
            &surface,
            Some(&output),
            zwlr_layer_shell_v1::Layer::Top,
            "hyprstatus".to_string(),
            qh,
            registry_id,
        );

        let height = self.config.output.get(&name_opt)
            .and_then(|o| o.height)
            .unwrap_or(self.config.bar.height);

        let anchor = if self.config.bar.position == "bottom" {
            zwlr_layer_surface_v1::Anchor::Bottom
                | zwlr_layer_surface_v1::Anchor::Left
                | zwlr_layer_surface_v1::Anchor::Right
        } else {
            zwlr_layer_surface_v1::Anchor::Top
                | zwlr_layer_surface_v1::Anchor::Left
                | zwlr_layer_surface_v1::Anchor::Right
        };

        layer_surface.set_anchor(anchor);
        layer_surface.set_size(0, height);
        layer_surface.set_exclusive_zone(height as i32);
        layer_surface.set_margin(
            self.config.bar.margin_top,
            self.config.bar.margin_right,
            self.config.bar.margin_bottom,
            self.config.bar.margin_left,
        );

        surface.commit();

        let popup_w = self.config.modules.popup.width;
        let popup_h = self.config.modules.popup.height;

        println!("[layer_shell] Created panel surface for output {} ({})", name_opt, registry_id);

        self.bars.insert(
            registry_id,
            BarOutput {
                name: name_opt,
                output,
                width: 1920,
                height,
                scale: 1.0,
                surface,
                layer_surface,
                configured: false,
                buffer: None,
                click_regions: Vec::new(),
                popup: WorkspacePopup::new(popup_w, popup_h),
                popup_surface: None,
                popup_layer_surface: None,
                popup_buffer: None,
                popup_configured: false,
            },
        );
    }

    fn get_module_block(&self, name: &str, output_name: &str) -> Option<ModuleBlock> {
        match name {
            "workspaces" => Some(self.mod_workspace.render(&self.config, output_name)),
            "window_title" => Some(self.mod_window_title.render(&self.config)),
            "audio" => Some(self.mod_audio.render(&self.config)),
            "network" => Some(self.mod_network.render(&self.config)),
            "battery" => Some(self.mod_battery.render(&self.config)),
            "perf" => Some(self.mod_perf.render(&self.config)),
            "clock" => Some(self.mod_clock.render(&self.config)),
            "tray" => Some(self.mod_tray.render(&self.config)),
            _ => None,
        }
    }

    pub fn render_bar(&mut self, qh: &QueueHandle<Self>, registry_id: u32) {
        let (output_name, width, height, _scale, is_configured) = match self.bars.get(&registry_id) {
            Some(b) => (b.name.clone(), b.width, b.height, b.scale, b.configured),
            None => return,
        };

        if !is_configured || width == 0 || height == 0 {
            return;
        }

        // 1. Fetch layout modules for this monitor
        let out_cfg = self.config.output.get(&output_name);
        let left_mods = out_cfg.and_then(|o| o.left.as_ref()).unwrap_or(&self.config.layout.left).clone();
        let center_mods = out_cfg.and_then(|o| o.center.as_ref()).unwrap_or(&self.config.layout.center).clone();
        let right_mods = out_cfg.and_then(|o| o.right.as_ref()).unwrap_or(&self.config.layout.right).clone();

        let mut left_blocks = Vec::new();
        for m in &left_mods {
            if let Some(b) = self.get_module_block(m, &output_name) {
                left_blocks.push(b);
            }
        }

        let mut center_blocks = Vec::new();
        for m in &center_mods {
            if let Some(b) = self.get_module_block(m, &output_name) {
                center_blocks.push(b);
            }
        }

        let mut right_blocks = Vec::new();
        for m in &right_mods {
            if let Some(b) = self.get_module_block(m, &output_name) {
                right_blocks.push(b);
            }
        }

        let shm = match &self.shm {
            Some(s) => s,
            None => return,
        };

        // Ensure buffer exists and matches dimensions
        let bar = match self.bars.get_mut(&registry_id) {
            Some(b) => b,
            None => return,
        };

        if bar.buffer.as_ref().map_or(true, |b| b.width != width || b.height != height) {
            match ShmBuffer::new(shm, qh, width, height) {
                Ok(b) => bar.buffer = Some(b),
                Err(e) => {
                    eprintln!("[layer_shell] Failed to allocate shm buffer for bar: {}", e);
                    return;
                }
            }
        }

        let buffer_entry = bar.buffer.as_mut().unwrap();
        let pixel_buf = &mut buffer_entry.mmap[..];

        // Clear background
        let bg_color = parse_hex_color(&self.config.colors.background);
        SoftwareRenderer::clear(pixel_buf, bg_color);

        let pad_h = self.config.bar.padding_horizontal as i32;
        let font_size = self.config.bar.font_size;
        let baseline_y = (height as i32 / 2) + (font_size as i32 / 3);

        let mut click_regions = Vec::new();

        // 3. Render Left Blocks
        let mut cur_x = pad_h;
        for block in &left_blocks {
            if block.name == "workspaces" && !block.pills.is_empty() {
                for pill in &block.pills {
                    let (tw, _th) = self.renderer.measure_text(&pill.label, font_size);
                    let pill_w = tw + 18;
                    let pill_h = height.saturating_sub(8);
                    let pill_y = 4;

                    SoftwareRenderer::draw_rounded_rect(
                        pixel_buf,
                        width,
                        height,
                        cur_x,
                        pill_y as i32,
                        pill_w,
                        pill_h,
                        4,
                        pill.bg,
                        pill.border,
                        1,
                    );

                    self.renderer.draw_text(
                        pixel_buf,
                        width,
                        height,
                        &pill.label,
                        cur_x + 9,
                        baseline_y,
                        font_size,
                        pill.fg,
                    );

                    click_regions.push(ClickRegion {
                        x: cur_x,
                        y: 0,
                        width: pill_w,
                        height,
                        action: ClickAction::SwitchWorkspace(pill.id),
                    });

                    // Add hover region for thumbnail popup
                    click_regions.push(ClickRegion {
                        x: cur_x,
                        y: 0,
                        width: pill_w,
                        height,
                        action: ClickAction::HoverWorkspace(pill.id, cur_x, pill_w as i32),
                    });

                    cur_x += pill_w as i32 + 4;
                }
                cur_x += 8;
            } else if !block.text.is_empty() {
                let (tw, _) = self.renderer.measure_text(&block.text, font_size);
                let block_x = cur_x;

                self.renderer.draw_text(
                    pixel_buf,
                    width,
                    height,
                    &block.text,
                    cur_x,
                    baseline_y,
                    font_size,
                    block.fg,
                );

                for cr in &block.click_regions {
                    click_regions.push(ClickRegion {
                        x: block_x,
                        y: 0,
                        width: tw,
                        height,
                        action: cr.action.clone(),
                    });
                }

                cur_x += tw as i32 + 16;
            }
        }

        // 4. Render Right Blocks (Right to Left)
        let mut rx = width as i32 - pad_h;
        for block in right_blocks.iter().rev() {
            if block.name == "tray" {
                if let Some((pw, ph, pixmap)) = &block.tray_pixmap {
                    let icon_size = 20u32;
                    let icon_y = (height.saturating_sub(icon_size) / 2) as i32;
                    let icon_x = rx - icon_size as i32;

                    SoftwareRenderer::draw_pixmap(
                        pixel_buf,
                        width,
                        height,
                        pixmap,
                        *pw,
                        *ph,
                        icon_x,
                        icon_y,
                        icon_size,
                        icon_size,
                    );

                    for cr in &block.click_regions {
                        click_regions.push(ClickRegion {
                            x: icon_x,
                            y: 0,
                            width: icon_size,
                            height,
                            action: cr.action.clone(),
                        });
                    }

                    rx -= icon_size as i32 + 10;
                }
            } else if !block.text.is_empty() {
                let (tw, _) = self.renderer.measure_text(&block.text, font_size);
                let block_w = tw + 16;
                let block_x = rx - block_w as i32;
                let pill_h = height.saturating_sub(6);
                let pill_y = 3;

                SoftwareRenderer::draw_rounded_rect(
                    pixel_buf,
                    width,
                    height,
                    block_x,
                    pill_y as i32,
                    block_w,
                    pill_h,
                    4,
                    block.bg,
                    None,
                    0,
                );

                self.renderer.draw_text(
                    pixel_buf,
                    width,
                    height,
                    &block.text,
                    block_x + 8,
                    baseline_y,
                    font_size,
                    block.fg,
                );

                for cr in &block.click_regions {
                    click_regions.push(ClickRegion {
                        x: block_x,
                        y: 0,
                        width: block_w,
                        height,
                        action: cr.action.clone(),
                    });
                }

                rx -= block_w as i32 + 6;
            }
        }

        // 5. Render Center Blocks
        if !center_blocks.is_empty() {
            let mut total_center_w = 0;
            let mut measured_center = Vec::new();

            for block in &center_blocks {
                let (tw, _) = self.renderer.measure_text(&block.text, font_size);
                let bw = tw + 16;
                measured_center.push((block, bw));
                total_center_w += bw + 6;
            }

            let mut cx = (width as i32 - total_center_w as i32) / 2;
            for (block, bw) in measured_center {
                let pill_h = height.saturating_sub(6);
                let pill_y = 3;

                SoftwareRenderer::draw_rounded_rect(
                    pixel_buf,
                    width,
                    height,
                    cx,
                    pill_y as i32,
                    bw,
                    pill_h,
                    4,
                    block.bg,
                    None,
                    0,
                );

                self.renderer.draw_text(
                    pixel_buf,
                    width,
                    height,
                    &block.text,
                    cx + 8,
                    baseline_y,
                    font_size,
                    block.fg,
                );

                for cr in &block.click_regions {
                    click_regions.push(ClickRegion {
                        x: cx,
                        y: 0,
                        width: bw,
                        height,
                        action: cr.action.clone(),
                    });
                }

                cx += bw as i32 + 6;
            }
        }

        // Update bar click regions and commit buffer
        let bar = self.bars.get_mut(&registry_id).unwrap();
        bar.click_regions = click_regions;

        let buf = &bar.buffer.as_ref().unwrap().buffer;
        bar.surface.attach(Some(buf), 0, 0);
        bar.surface.damage_buffer(0, 0, width as i32, height as i32);
        bar.surface.commit();
    }

    pub fn handle_pointer_click(&mut self, qh: &QueueHandle<Self>, button: MouseButton, x: i32, y: i32) {
        // If click is on a popup surface, check popup window thumbnail clicks
        for bar in self.bars.values() {
            if bar.popup.visible && bar.popup_surface.as_ref() == self.pointer_surface.as_ref() {
                if bar.popup.handle_click(x, y) {
                    self.hide_workspace_popup();
                    return;
                }
            }
        }

        // Find which bar contains this coordinate (matching the surface pointer is over)
        for bar in self.bars.values() {
            if Some(&bar.surface) == self.pointer_surface.as_ref() {
                for cr in &bar.click_regions {
                    if x >= cr.x && x < cr.x + cr.width as i32 && y >= cr.y && y < cr.y + cr.height as i32 {
                    match &cr.action {
                        ClickAction::SwitchWorkspace(id) => {
                            let ws_id = *id;
                            match button {
                                MouseButton::ScrollUp => {
                                    tokio::spawn(async move {
                                        let _ = HyprlandIpc::cycle_workspace(false).await;
                                    });
                                }
                                MouseButton::ScrollDown => {
                                    tokio::spawn(async move {
                                        let _ = HyprlandIpc::cycle_workspace(true).await;
                                    });
                                }
                                _ => {
                                    self.hide_workspace_popup();
                                    tokio::spawn(async move {
                                        let _ = HyprlandIpc::switch_workspace(ws_id).await;
                                    });
                                }
                            }
                            return;
                        }
                        ClickAction::ToggleAudioMute => {
                            let mod_audio = self.mod_audio.clone();
                            let step = self.config.modules.audio.step_percent;
                            match button {
                                MouseButton::Right => {
                                    let cur = mod_audio.state.source_muted.load(std::sync::atomic::Ordering::Relaxed);
                                    mod_audio.state.source_muted.store(!cur, std::sync::atomic::Ordering::Relaxed);
                                }
                                MouseButton::ScrollUp => {
                                    let cur = mod_audio.state.volume.load(std::sync::atomic::Ordering::Relaxed);
                                    mod_audio.state.volume.store((cur + step).min(150), std::sync::atomic::Ordering::Relaxed);
                                }
                                MouseButton::ScrollDown => {
                                    let cur = mod_audio.state.volume.load(std::sync::atomic::Ordering::Relaxed);
                                    mod_audio.state.volume.store(cur.saturating_sub(step), std::sync::atomic::Ordering::Relaxed);
                                }
                                _ => {
                                    let cur = mod_audio.state.sink_muted.load(std::sync::atomic::Ordering::Relaxed);
                                    mod_audio.state.sink_muted.store(!cur, std::sync::atomic::Ordering::Relaxed);
                                }
                            }
                            let reg_ids: Vec<u32> = self.bars.keys().copied().collect();
                            for id in reg_ids {
                                self.render_bar(qh, id);
                            }
                            tokio::spawn(async move {
                                match button {
                                    MouseButton::Right => mod_audio.toggle_mic_mute().await,
                                    MouseButton::ScrollUp => mod_audio.volume_up(step).await,
                                    MouseButton::ScrollDown => mod_audio.volume_down(step).await,
                                    _ => mod_audio.toggle_mute().await,
                                }
                            });
                            return;
                        }
                        ClickAction::ToggleMicMute => {
                            let mod_audio = self.mod_audio.clone();
                            let cur = mod_audio.state.source_muted.load(std::sync::atomic::Ordering::Relaxed);
                            mod_audio.state.source_muted.store(!cur, std::sync::atomic::Ordering::Relaxed);
                            let reg_ids: Vec<u32> = self.bars.keys().copied().collect();
                            for id in reg_ids {
                                self.render_bar(qh, id);
                            }
                            tokio::spawn(async move {
                                mod_audio.toggle_mic_mute().await;
                            });
                            return;
                        }
                        ClickAction::AudioVolumeUp => {
                            let mod_audio = self.mod_audio.clone();
                            let step = self.config.modules.audio.step_percent;
                            let cur = mod_audio.state.volume.load(std::sync::atomic::Ordering::Relaxed);
                            mod_audio.state.volume.store((cur + step).min(150), std::sync::atomic::Ordering::Relaxed);
                            let reg_ids: Vec<u32> = self.bars.keys().copied().collect();
                            for id in reg_ids {
                                self.render_bar(qh, id);
                            }
                            tokio::spawn(async move {
                                mod_audio.volume_up(step).await;
                            });
                            return;
                        }
                        ClickAction::AudioVolumeDown => {
                            let mod_audio = self.mod_audio.clone();
                            let step = self.config.modules.audio.step_percent;
                            let cur = mod_audio.state.volume.load(std::sync::atomic::Ordering::Relaxed);
                            mod_audio.state.volume.store(cur.saturating_sub(step), std::sync::atomic::Ordering::Relaxed);
                            let reg_ids: Vec<u32> = self.bars.keys().copied().collect();
                            for id in reg_ids {
                                self.render_bar(qh, id);
                            }
                            tokio::spawn(async move {
                                mod_audio.volume_down(step).await;
                            });
                            return;
                        }
                        ClickAction::NetworkClick => {
                            let cmd = self.config.modules.network.click_command.clone();
                            tokio::spawn(async move {
                                if cmd == "nmtui" || cmd == "hyprstatus --wifi-menu" || cmd == "wifi-menu" || cmd.is_empty() {
                                    NetworkModule::open_wifi_modal().await;
                                } else {
                                    let _ = tokio::process::Command::new("sh")
                                        .arg("-c")
                                        .arg(&cmd)
                                        .spawn();
                                }
                            });
                            return;
                        }
                        ClickAction::PerfClick => {
                            let cmd = self.config.modules.perf.click_command.clone();
                            tokio::spawn(async move {
                                let _ = tokio::process::Command::new("alacritty")
                                    .arg("-e")
                                    .arg(&cmd)
                                    .spawn();
                            });
                            return;
                        }
                        ClickAction::ClockClick => {
                            tokio::spawn(async move {
                                let _ = tokio::process::Command::new("notify-send")
                                    .args(["-a", "hyprstatus", "Calendar", "Calendar notification"])
                                    .spawn();
                            });
                            return;
                        }
                        ClickAction::TrayActivate(service) => {
                            let mod_tray = self.mod_tray.clone();
                            let s = service.clone();
                            tokio::spawn(async move {
                                if button == MouseButton::Right {
                                    mod_tray.context_menu_item(&s, x, y).await;
                                } else {
                                    mod_tray.activate_item(&s, x, y).await;
                                }
                            });
                            return;
                        }
                        ClickAction::TrayContextMenu(service) => {
                            let mod_tray = self.mod_tray.clone();
                            let s = service.clone();
                            tokio::spawn(async move {
                                mod_tray.context_menu_item(&s, x, y).await;
                            });
                            return;
                        }
                        _ => {}
                    }
                    }
                }
            }
        }
    }

    pub fn handle_pointer_motion(&mut self, qh: &QueueHandle<Self>, x: f64, y: f64) {
        self.pointer_x = x;
        self.pointer_y = y;

        // Check if cursor hovers over any workspace pill on the active bar
        let xi = x as i32;
        let yi = y as i32;

        let mut hovered: Option<(u32, i32, i32, u32)> = None;
        for (reg_id, bar) in &self.bars {
            if Some(&bar.surface) == self.pointer_surface.as_ref() {
                for cr in &bar.click_regions {
                    if let ClickAction::HoverWorkspace(ws_id, px, pw) = &cr.action {
                        if xi >= *px && xi < *px + *pw && yi >= cr.y && yi < cr.y + cr.height as i32 {
                            hovered = Some((*reg_id, *ws_id, *px, *pw as u32));
                            break;
                        }
                    }
                }
                break;
            }
        }

        if hovered != self.hovered_ws_pill {
            self.hovered_ws_pill = hovered;
            if let Some((reg_id, ws_id, px, _)) = hovered {
                if self.config.modules.popup.enabled {
                    self.show_workspace_popup(qh, reg_id, ws_id, px);
                }
            } else {
                self.hide_workspace_popup();
            }
        }
    }

    pub fn show_workspace_popup(&mut self, qh: &QueueHandle<Self>, reg_id: u32, ws_id: i32, anchor_x: i32) {
        let clients = self.mod_workspace.get_clients_for_workspace(ws_id);
        let bar = match self.bars.get_mut(&reg_id) {
            Some(b) => b,
            None => return,
        };

        bar.popup.update_clients(ws_id, anchor_x, &clients, bar.width, bar.height);

        let (anchor, margin_top, margin_bottom) = if self.config.bar.position == "bottom" {
            (
                zwlr_layer_surface_v1::Anchor::Bottom | zwlr_layer_surface_v1::Anchor::Left,
                0,
                bar.height as i32 + 6,
            )
        } else {
            (
                zwlr_layer_surface_v1::Anchor::Top | zwlr_layer_surface_v1::Anchor::Left,
                bar.height as i32 + 6,
                0,
            )
        };

        // If popup surface doesn't exist, create it via layer_shell
        if bar.popup_surface.is_none() {
            let compositor = match &self.compositor {
                Some(c) => c,
                None => return,
            };
            let layer_shell = match &self.layer_shell {
                Some(ls) => ls,
                None => return,
            };

            let surface = compositor.create_surface(qh, ());
            let layer_surface = layer_shell.get_layer_surface(
                &surface,
                Some(&bar.output),
                zwlr_layer_shell_v1::Layer::Overlay,
                "hyprstatus-popup".to_string(),
                qh,
                reg_id + 1000,
            );

            layer_surface.set_anchor(anchor);
            layer_surface.set_size(bar.popup.width, bar.popup.height);
            layer_surface.set_margin(margin_top, 0, margin_bottom, anchor_x);
            layer_surface.set_exclusive_zone(-1);

            // Initial commit without buffer attached (required by layer shell protocol)
            surface.commit();

            bar.popup_surface = Some(surface);
            bar.popup_layer_surface = Some(layer_surface);
            bar.popup_configured = false;
        } else {
            // Popup surface already exists, update position margin
            if let Some(ls) = &bar.popup_layer_surface {
                ls.set_margin(margin_top, 0, margin_bottom, anchor_x);
            }
            if bar.popup_configured {
                self.render_popup(qh, reg_id);
            }
        }
    }

    pub fn hide_workspace_popup(&mut self) {
        for bar in self.bars.values_mut() {
            if bar.popup.visible || bar.popup_surface.is_some() {
                bar.popup.visible = false;
                bar.popup_configured = false;
                if let Some(ls) = bar.popup_layer_surface.take() {
                    ls.destroy();
                }
                if let Some(s) = bar.popup_surface.take() {
                    s.destroy();
                }
                bar.popup_buffer = None;
            }
        }
        self.hovered_ws_pill = None;
    }

    pub fn render_popup(&mut self, qh: &QueueHandle<Self>, reg_id: u32) {
        let shm = match &self.shm {
            Some(s) => s,
            None => return,
        };

        let bar = match self.bars.get_mut(&reg_id) {
            Some(b) => b,
            None => return,
        };

        if !bar.popup_configured || !bar.popup.visible || bar.popup_surface.is_none() {
            return;
        }

        let pw = bar.popup.width;
        let ph = bar.popup.height;

        if bar.popup_buffer.as_ref().map_or(true, |b| b.width != pw || b.height != ph) {
            match ShmBuffer::new(shm, qh, pw, ph) {
                Ok(b) => bar.popup_buffer = Some(b),
                Err(e) => {
                    eprintln!("[layer_shell] Failed to allocate shm buffer for popup: {}", e);
                    return;
                }
            }
        }

        let pbuf = bar.popup_buffer.as_mut().unwrap();
        bar.popup.render(&mut self.renderer, &mut pbuf.mmap[..], &self.config);

        if let Some(s) = &bar.popup_surface {
            s.attach(Some(&pbuf.buffer), 0, 0);
            s.damage_buffer(0, 0, pw as i32, ph as i32);
            s.commit();
        }
    }
}

// -------------------------------- Wayland Dispatch Implementations --------------------------------

impl Dispatch<wl_registry::WlRegistry, ()> for BarState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } => match &interface[..] {
                "wl_compositor" => {
                    state.compositor = Some(registry.bind::<wl_compositor::WlCompositor, _, _>(
                        name,
                        version.min(6),
                        qh,
                        (),
                    ));
                }
                "wl_shm" => {
                    state.shm = Some(registry.bind::<wl_shm::WlShm, _, _>(name, 1, qh, ()));
                }
                "zwlr_layer_shell_v1" => {
                    state.layer_shell = Some(registry.bind::<ZwlrLayerShellV1, _, _>(
                        name,
                        version.min(4),
                        qh,
                        (),
                    ));
                }
                "wl_seat" => {
                    let seat = registry.bind::<wl_seat::WlSeat, _, _>(name, version.min(8), qh, ());
                    state.seat = Some(seat);
                }
                "wl_output" => {
                    let output = registry.bind::<wl_output::WlOutput, _, _>(name, version.min(4), qh, name);
                    state.outputs.insert(name, (output, None));
                    println!("[layer_shell] Detected display output: id={}", name);
                }
                _ => {}
            },
            wl_registry::Event::GlobalRemove { name } => {
                println!("[layer_shell] Display output disconnected: id={}", name);
                if let Some(mut bar) = state.bars.remove(&name) {
                    if let Some(ls) = bar.popup_layer_surface.take() {
                        ls.destroy();
                    }
                    if let Some(ps) = bar.popup_surface.take() {
                        ps.destroy();
                    }
                    bar.layer_surface.destroy();
                    bar.surface.destroy();
                }
                state.outputs.remove(&name);
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for BarState {
    fn event(
        _: &mut Self,
        _: &wl_compositor::WlCompositor,
        _: wl_compositor::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {}
}

impl Dispatch<wl_shm::WlShm, ()> for BarState {
    fn event(
        _: &mut Self,
        _: &wl_shm::WlShm,
        _: wl_shm::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {}
}

impl Dispatch<wl_shm_pool::WlShmPool, ()> for BarState {
    fn event(
        _: &mut Self,
        _: &wl_shm_pool::WlShmPool,
        _: wl_shm_pool::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {}
}
use wayland_client::protocol::wl_shm_pool;

impl Dispatch<wl_buffer::WlBuffer, ()> for BarState {
    fn event(
        _: &mut Self,
        _: &wl_buffer::WlBuffer,
        _: wl_buffer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {}
}

impl Dispatch<wl_surface::WlSurface, ()> for BarState {
    fn event(
        _: &mut Self,
        _: &wl_surface::WlSurface,
        _: wl_surface::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {}
}

impl Dispatch<ZwlrLayerShellV1, ()> for BarState {
    fn event(
        _: &mut Self,
        _: &ZwlrLayerShellV1,
        _: zwlr_layer_shell_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {}
}

impl Dispatch<ZwlrLayerSurfaceV1, u32> for BarState {
    fn event(
        state: &mut Self,
        surface: &ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        data: &u32,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let id = *data;
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                println!("[layer_shell] Configure event received for id={}, width={}, height={}", id, width, height);
                surface.ack_configure(serial);

                if id >= 1000 {
                    // Popup configure
                    let bar_id = id - 1000;
                    if let Some(bar) = state.bars.get_mut(&bar_id) {
                        bar.popup_configured = true;
                    }
                    state.render_popup(qh, bar_id);
                } else if let Some(bar) = state.bars.get_mut(&id) {
                    if width > 0 {
                        bar.width = width;
                    }
                    if height > 0 {
                        bar.height = height;
                    }
                    bar.configured = true;
                    state.render_bar(qh, id);
                }
            }
            zwlr_layer_surface_v1::Event::Closed => {
                if id < 1000 {
                    state.bars.remove(&id);
                } else {
                    let bar_id = id - 1000;
                    if let Some(bar) = state.bars.get_mut(&bar_id) {
                        if let Some(ls) = bar.popup_layer_surface.take() {
                            ls.destroy();
                        }
                        if let Some(s) = bar.popup_surface.take() {
                            s.destroy();
                        }
                        bar.popup_buffer = None;
                        bar.popup_configured = false;
                        bar.popup.visible = false;
                    }
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, u32> for BarState {
    fn event(
        state: &mut Self,
        _output: &wl_output::WlOutput,
        event: wl_output::Event,
        data: &u32,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let registry_id = *data;
        match event {
            wl_output::Event::Name { name } => {
                println!("[layer_shell] Output id={} name='{}'", registry_id, name);
                if let Some(entry) = state.outputs.get_mut(&registry_id) {
                    entry.1 = Some(name);
                }
            }
            wl_output::Event::Mode { width, height: _, .. } => {
                if let Some(bar) = state.bars.get_mut(&registry_id) {
                    bar.width = width as u32;
                }
            }
            wl_output::Event::Scale { factor } => {
                if let Some(bar) = state.bars.get_mut(&registry_id) {
                    bar.scale = factor as f32;
                }
            }
            wl_output::Event::Done => {
                // Monitor announcement complete, instantiate bar if not yet created
                if !state.bars.contains_key(&registry_id) {
                    state.init_bar_for_output(qh, registry_id);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_seat::WlSeat, ()> for BarState {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities { capabilities } = event {
            if let wayland_client::WEnum::Value(caps) = capabilities {
                if caps.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                    let pointer = seat.get_pointer(qh, ());
                    state.pointer = Some(pointer);
                }
            }
        }
    }
}

impl Dispatch<wl_pointer::WlPointer, ()> for BarState {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter {
                surface,
                surface_x,
                surface_y,
                ..
            } => {
                state.pointer_surface = Some(surface);
                state.handle_pointer_motion(qh, surface_x, surface_y);
            }
            wl_pointer::Event::Leave { .. } => {
                state.pointer_surface = None;
                state.hide_workspace_popup();
            }
            wl_pointer::Event::Motion {
                surface_x,
                surface_y,
                ..
            } => {
                state.handle_pointer_motion(qh, surface_x, surface_y);
            }
            wl_pointer::Event::Button { button, state: btn_state, .. } => {
                if btn_state == wayland_client::WEnum::Value(wayland_client::protocol::wl_pointer::ButtonState::Pressed) {
                    let mouse_btn = match button {
                        272 => MouseButton::Left,
                        273 => MouseButton::Right,
                        274 => MouseButton::Middle,
                        _ => MouseButton::Left,
                    };
                    state.handle_pointer_click(qh, mouse_btn, state.pointer_x as i32, state.pointer_y as i32);
                }
            }
            wl_pointer::Event::Axis { value, .. } => {
                let mouse_btn = if value < 0.0 {
                    MouseButton::ScrollUp
                } else {
                    MouseButton::ScrollDown
                };
                state.handle_pointer_click(qh, mouse_btn, state.pointer_x as i32, state.pointer_y as i32);
            }
            _ => {}
        }
    }
}
