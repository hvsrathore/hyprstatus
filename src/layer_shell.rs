use crate::config::{parse_hex_color, Config};
use crate::ipc::HyprlandIpc;
use crate::modules::audio::AudioModule;
use crate::modules::battery::BatteryModule;
use crate::modules::clock::ClockModule;
use crate::modules::network::NetworkModule;
use crate::modules::perf::PerfModule;
use crate::modules::popup::{
    AudioPopover, AudioPopoverAction, BarPopover, PopoverKind, WifiPopover, WifiPopoverAction,
    WorkspacePopup,
};
use crate::modules::screenshot::ScreenshotModule;
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
    // Popover state for this output (workspace preview, audio quick menu, wifi quick menu)
    pub popover: BarPopover,
    pub popup_surface: Option<wl_surface::WlSurface>,
    pub popup_layer_surface: Option<ZwlrLayerSurfaceV1>,
    pub popup_buffer: Option<ShmBuffer>,
    pub popup_configured: bool,

    // Backdrop for click-outside dismissal
    pub backdrop_surface: Option<wl_surface::WlSurface>,
    pub backdrop_layer_surface: Option<ZwlrLayerSurfaceV1>,
    pub backdrop_buffer: Option<ShmBuffer>,
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
    pub mod_screenshot: ScreenshotModule,

    // Pointer tracking
    pub pointer_surface: Option<wl_surface::WlSurface>,
    pub pointer_x: f64,
    pub pointer_y: f64,
    pub pointer_pressed: bool,
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
        mod_screenshot: ScreenshotModule,
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
            mod_screenshot,
            pointer_surface: None,
            pointer_x: 0.0,
            pointer_y: 0.0,
            pointer_pressed: false,
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
                popover: BarPopover::new(),
                popup_surface: None,
                popup_layer_surface: None,
                popup_buffer: None,
                popup_configured: false,
                backdrop_surface: None,
                backdrop_layer_surface: None,
                backdrop_buffer: None,
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
            "screenshot" => Some(self.mod_screenshot.render(&self.config, output_name)),
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
        // 0. If click is on any backdrop surface, dismiss all popovers immediately
        let mut clicked_backdrop = false;
        for bar in self.bars.values() {
            if bar.backdrop_surface.as_ref() == self.pointer_surface.as_ref() {
                clicked_backdrop = true;
                break;
            }
        }
        if clicked_backdrop {
            self.hide_all_popovers();
            return;
        }

        let mut hide_reg_id = None;
        let mut rerender_popover = None;
        let mut rerender_bar = None;

        // 1. If click is on a popup surface, delegate directly to the active popover menu
        for (&reg_id, bar) in self.bars.iter_mut() {
            if bar.popover.visible && bar.popup_surface.as_ref() == self.pointer_surface.as_ref() {
                match &mut bar.popover.kind {
                    PopoverKind::Workspace(ws) => {
                        if ws.handle_click(x, y) {
                            hide_reg_id = Some(reg_id);
                        }
                    }
                    PopoverKind::Audio(audio) => {
                        let action = audio.handle_click(x, y);
                        match action {
                            AudioPopoverAction::Close => {
                                hide_reg_id = Some(reg_id);
                            }
                            AudioPopoverAction::SetVolume(vol) => {
                                audio.volume = vol;
                                audio.sink_muted = false;
                                let mod_audio = self.mod_audio.clone();
                                tokio::spawn(async move {
                                    mod_audio.set_volume(vol).await;
                                });
                                rerender_popover = Some(reg_id);
                                rerender_bar = Some(reg_id);
                            }
                            AudioPopoverAction::VolumeDelta(delta) => {
                                let new_vol = if delta < 0 {
                                    audio.volume.saturating_sub((-delta) as u32)
                                } else {
                                    (audio.volume + delta as u32).min(150)
                                };
                                audio.volume = new_vol;
                                audio.sink_muted = false;
                                let mod_audio = self.mod_audio.clone();
                                tokio::spawn(async move {
                                    mod_audio.set_volume(new_vol).await;
                                });
                                rerender_popover = Some(reg_id);
                                rerender_bar = Some(reg_id);
                            }
                            AudioPopoverAction::ToggleMute => {
                                audio.sink_muted = !audio.sink_muted;
                                let mod_audio = self.mod_audio.clone();
                                tokio::spawn(async move {
                                    mod_audio.toggle_mute().await;
                                });
                                rerender_popover = Some(reg_id);
                                rerender_bar = Some(reg_id);
                            }
                            AudioPopoverAction::ToggleMicMute => {
                                audio.source_muted = !audio.source_muted;
                                let mod_audio = self.mod_audio.clone();
                                tokio::spawn(async move {
                                    mod_audio.toggle_mic_mute().await;
                                });
                                rerender_popover = Some(reg_id);
                                rerender_bar = Some(reg_id);
                            }
                            AudioPopoverAction::None => {}
                        }
                    }
                    PopoverKind::Wifi(wifi) => {
                        let action = wifi.handle_click(x, y);
                        match action {
                            WifiPopoverAction::Close => {
                                hide_reg_id = Some(reg_id);
                            }
                            WifiPopoverAction::ToggleRadio => {
                                wifi.wifi_enabled = !wifi.wifi_enabled;
                                wifi.status_message = Some(if wifi.wifi_enabled {
                                    "Turning Wi-Fi on...".into()
                                } else {
                                    "Wi-Fi turned off".into()
                                });
                                let mod_net = self.mod_network.clone();
                                tokio::spawn(async move {
                                    NetworkModule::toggle_wifi_radio().await;
                                    tokio::time::sleep(tokio::time::Duration::from_millis(600)).await;
                                    mod_net.refresh_networks().await;
                                });
                                rerender_popover = Some(reg_id);
                            }
                            WifiPopoverAction::Rescan => {
                                wifi.is_scanning = true;
                                wifi.status_message = Some("Scanning for networks...".into());
                                let mod_net = self.mod_network.clone();
                                tokio::spawn(async move {
                                    mod_net.trigger_rescan().await;
                                });
                                rerender_popover = Some(reg_id);
                            }
                            WifiPopoverAction::Connect(ssid) => {
                                let s = ssid.clone();
                                let mod_net = self.mod_network.clone();
                                let is_saved = NetworkModule::is_connection_saved(&s);
                                let is_open = wifi.networks.iter().find(|n| n.ssid == s).map_or(false, |n| n.security.is_empty() || n.security == "--");

                                if is_saved || is_open {
                                    wifi.status_message = Some(format!("Connecting to {}...", s));
                                    tokio::spawn(async move {
                                        let _ = NetworkModule::connect_wifi(&s).await;
                                        tokio::time::sleep(tokio::time::Duration::from_millis(1200)).await;
                                        mod_net.refresh_networks().await;
                                    });
                                } else {
                                    wifi.status_message = Some(format!("Prompting password for {}...", s));
                                    tokio::spawn(async move {
                                        if let Some(password) = NetworkModule::prompt_wifi_password(&s).await {
                                            if !password.is_empty() {
                                                let _ = NetworkModule::connect_wifi_with_password(&s, &password).await;
                                                tokio::time::sleep(tokio::time::Duration::from_millis(1200)).await;
                                                mod_net.refresh_networks().await;
                                            }
                                        }
                                    });
                                }
                                rerender_popover = Some(reg_id);
                            }
                            WifiPopoverAction::Disconnect(ssid) => {
                                wifi.status_message = Some(format!("Disconnecting from {}...", ssid));
                                let s = ssid.clone();
                                let mod_net = self.mod_network.clone();
                                tokio::spawn(async move {
                                    NetworkModule::disconnect_wifi(&s).await;
                                    tokio::time::sleep(tokio::time::Duration::from_millis(800)).await;
                                    mod_net.refresh_networks().await;
                                });
                                rerender_popover = Some(reg_id);
                            }
                            WifiPopoverAction::OpenNmtui => {
                                hide_reg_id = Some(reg_id);
                                tokio::spawn(async move {
                                    let terminals = ["kitty", "alacritty", "foot", "wezterm", "gnome-terminal"];
                                    for term in terminals {
                                        if let Ok(mut child) = tokio::process::Command::new(term)
                                            .arg("-e")
                                            .arg("nmtui")
                                            .spawn()
                                        {
                                            let _ = child.wait().await;
                                            break;
                                        }
                                    }
                                });
                            }
                            WifiPopoverAction::None => {}
                        }
                    }
                    PopoverKind::None => {}
                }
                break;
            }
        }

        if let Some(id) = hide_reg_id {
            self.hide_popover(id);
            return;
        }
        if let Some(id) = rerender_popover {
            self.render_popup(qh, id);
        }
        if let Some(id) = rerender_bar {
            self.render_bar(qh, id);
            return;
        }
        if rerender_popover.is_some() {
            return;
        }

        // 2. Click is on the status bar itself
        let mut clicked: Option<(u32, ClickAction, i32, u32)> = None;
        let mut on_bar = false;
        for (&reg_id, bar) in &self.bars {
            if Some(&bar.surface) == self.pointer_surface.as_ref() {
                on_bar = true;
                for cr in &bar.click_regions {
                    if x >= cr.x && x < cr.x + cr.width as i32 && y >= cr.y && y < cr.y + cr.height as i32 {
                        clicked = Some((reg_id, cr.action.clone(), cr.x, cr.width));
                        break;
                    }
                }
                break;
            }
        }

        // Clicking elsewhere (empty bar space or off-surface): dismiss popovers
        if !on_bar || clicked.is_none() {
            if self.has_visible_popover() {
                self.hide_all_popovers();
            }
            return;
        }

        if let Some((active_reg_id, action, cr_x, cr_width)) = clicked {
            match action {
                ClickAction::SwitchWorkspace(id) => {
                    let ws_id = id;
                    self.hide_all_popovers();
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
                            tokio::spawn(async move {
                                let _ = HyprlandIpc::switch_workspace(ws_id).await;
                            });
                        }
                    }
                }
                ClickAction::ToggleAudioMute => {
                    let anchor_x = cr_x + (cr_width as i32 / 2);
                    let mod_audio = self.mod_audio.clone();
                    let step = self.config.modules.audio.step_percent;
                    match button {
                        MouseButton::Right => {
                            let cur = mod_audio.state.source_muted.load(std::sync::atomic::Ordering::Relaxed);
                            mod_audio.state.source_muted.store(!cur, std::sync::atomic::Ordering::Relaxed);
                            let reg_ids: Vec<u32> = self.bars.keys().copied().collect();
                            for id in reg_ids {
                                self.render_bar(qh, id);
                            }
                            tokio::spawn(async move {
                                mod_audio.toggle_mic_mute().await;
                            });
                        }
                        MouseButton::ScrollUp => {
                            let cur = mod_audio.state.volume.load(std::sync::atomic::Ordering::Relaxed);
                            mod_audio.state.volume.store((cur + step).min(150), std::sync::atomic::Ordering::Relaxed);
                            let reg_ids: Vec<u32> = self.bars.keys().copied().collect();
                            for id in reg_ids {
                                self.render_bar(qh, id);
                            }
                            tokio::spawn(async move {
                                mod_audio.volume_up(step).await;
                            });
                        }
                        MouseButton::ScrollDown => {
                            let cur = mod_audio.state.volume.load(std::sync::atomic::Ordering::Relaxed);
                            mod_audio.state.volume.store(cur.saturating_sub(step), std::sync::atomic::Ordering::Relaxed);
                            let reg_ids: Vec<u32> = self.bars.keys().copied().collect();
                            for id in reg_ids {
                                self.render_bar(qh, id);
                            }
                            tokio::spawn(async move {
                                mod_audio.volume_down(step).await;
                            });
                        }
                        _ => {
                            self.toggle_audio_popover(qh, active_reg_id, anchor_x);
                        }
                    }
                }
                ClickAction::ToggleMicMute => {
                    self.hide_all_popovers();
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
                }
                ClickAction::NetworkClick => {
                    let anchor_x = cr_x + (cr_width as i32 / 2);
                    match button {
                        MouseButton::Right => {
                            self.hide_all_popovers();
                            let cmd = self.config.modules.network.click_command.clone();
                            tokio::spawn(async move {
                                let _ = tokio::process::Command::new("alacritty")
                                    .arg("-e")
                                    .arg(if cmd.is_empty() { "nmtui" } else { &cmd })
                                    .spawn();
                            });
                        }
                        _ => {
                            self.toggle_wifi_popover(qh, active_reg_id, anchor_x);
                        }
                    }
                }
                ClickAction::PerfClick => {
                    self.hide_all_popovers();
                    let cmd = self.config.modules.perf.click_command.clone();
                    tokio::spawn(async move {
                        let _ = tokio::process::Command::new("alacritty")
                            .arg("-e")
                            .arg(&cmd)
                            .spawn();
                    });
                }
                ClickAction::ClockClick => {
                    self.hide_all_popovers();
                    tokio::spawn(async move {
                        let _ = tokio::process::Command::new("notify-send")
                            .args(["-a", "hyprstatus", "Calendar", "Calendar notification"])
                            .spawn();
                    });
                }
                ClickAction::ScreenshotClick(out_name) => {
                    // Close any dropdown popovers so they don't block the screen,
                    // but the status bar itself remains completely visible on top.
                    self.hide_all_popovers();
                    self.mod_screenshot.trigger_flash();

                    // Re-render bars immediately to display visual flash feedback
                    let reg_ids: Vec<u32> = self.bars.keys().copied().collect();
                    for id in reg_ids {
                        self.render_bar(qh, id);
                    }

                    let config = self.config.clone();
                    let all_monitors = match button {
                        MouseButton::Right => false, // Right-click captures just this output
                        _ => config.modules.screenshot.all_monitors, // Left-click captures entire desktop screen
                    };

                    tokio::spawn(async move {
                        // Brief pause to allow compositor to flush any closed popovers
                        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                        match ScreenshotModule::capture(&config, Some(&out_name), all_monitors).await {
                            Ok(path) => {
                                println!("[screenshot] Screen captured successfully to {}", path.display());
                            }
                            Err(e) => {
                                eprintln!("[screenshot] Error capturing screenshot: {}", e);
                            }
                        }
                    });
                }
                ClickAction::TrayActivate(service) => {
                    self.hide_all_popovers();
                    let mod_tray = self.mod_tray.clone();
                    tokio::spawn(async move {
                        if button == MouseButton::Right {
                            mod_tray.context_menu_item(&service, x, y).await;
                        } else {
                            mod_tray.activate_item(&service, x, y).await;
                        }
                    });
                }
                ClickAction::TrayContextMenu(service) => {
                    self.hide_all_popovers();
                    let mod_tray = self.mod_tray.clone();
                    tokio::spawn(async move {
                        mod_tray.context_menu_item(&service, x, y).await;
                    });
                }
                _ => {}
            }
        }
    }

    pub fn handle_pointer_motion(&mut self, qh: &QueueHandle<Self>, x: f64, y: f64) {
        self.pointer_x = x;
        self.pointer_y = y;

        // If pointer is pressed inside audio popover, handle dragging volume slider smoothly
        if self.pointer_pressed {
            let mut drag_update = None;
            for (&reg_id, bar) in self.bars.iter_mut() {
                if bar.popover.visible && bar.popup_surface.as_ref() == self.pointer_surface.as_ref() {
                    if let PopoverKind::Audio(audio) = &mut bar.popover.kind {
                        if y >= 58.0 && y <= 86.0 {
                            let ratio = ((x - 16.0) / 288.0).clamp(0.0, 1.0);
                            let new_vol = (ratio * 100.0).round() as u32;
                            if audio.volume != new_vol {
                                audio.volume = new_vol;
                                audio.sink_muted = false;
                                drag_update = Some((reg_id, new_vol));
                            }
                        }
                    }
                    break;
                }
            }

            if let Some((reg_id, new_vol)) = drag_update {
                let mod_audio = self.mod_audio.clone();
                tokio::spawn(async move {
                    mod_audio.set_volume(new_vol).await;
                });
                self.render_popup(qh, reg_id);
                self.render_bar(qh, reg_id);
                return;
            }
        }

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

    pub fn show_popover_surface(&mut self, qh: &QueueHandle<Self>, reg_id: u32, anchor_x: i32) {
        let is_interactive = if let Some(bar) = self.bars.get(&reg_id) {
            matches!(bar.popover.kind, PopoverKind::Audio(_) | PopoverKind::Wifi(_))
        } else {
            false
        };

        if is_interactive {
            let bar_height = self.bars.get(&reg_id).map_or(30, |b| b.height as i32);
            let bar_pos_bottom = self.config.bar.position == "bottom";
            if let (Some(compositor), Some(layer_shell)) = (&self.compositor, &self.layer_shell) {
                let needed: Vec<(u32, wl_output::WlOutput)> = self
                    .bars
                    .iter()
                    .filter(|(_, b)| b.backdrop_surface.is_none())
                    .map(|(&id, b)| (id, b.output.clone()))
                    .collect();

                for (id, output) in needed {
                    let b_surf = compositor.create_surface(qh, ());
                    let b_layer = layer_shell.get_layer_surface(
                        &b_surf,
                        Some(&output),
                        zwlr_layer_shell_v1::Layer::Top,
                        "hyprstatus-backdrop".to_string(),
                        qh,
                        id + 2000,
                    );
                    b_layer.set_anchor(
                        zwlr_layer_surface_v1::Anchor::Top
                        | zwlr_layer_surface_v1::Anchor::Bottom
                        | zwlr_layer_surface_v1::Anchor::Left
                        | zwlr_layer_surface_v1::Anchor::Right,
                    );
                    if bar_pos_bottom {
                        b_layer.set_margin(0, 0, bar_height, 0);
                    } else {
                        b_layer.set_margin(bar_height, 0, 0, 0);
                    }
                    b_layer.set_exclusive_zone(-1);
                    b_surf.commit();

                    if let Some(b) = self.bars.get_mut(&id) {
                        b.backdrop_surface = Some(b_surf);
                        b.backdrop_layer_surface = Some(b_layer);
                    }
                }
            }
        }

        let bar = match self.bars.get_mut(&reg_id) {
            Some(b) => b,
            None => return,
        };

        bar.popover.anchor_x = anchor_x;
        bar.popover.visible = true;

        let pop_w = bar.popover.width;
        let pop_h = bar.popover.height;

        let mut margin_left = anchor_x - (pop_w as i32 / 2);
        if margin_left + pop_w as i32 > bar.width as i32 - 10 {
            margin_left = (bar.width as i32 - pop_w as i32 - 10).max(10);
        }
        if margin_left < 10 {
            margin_left = 10;
        }

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
            layer_surface.set_size(pop_w, pop_h);
            layer_surface.set_margin(margin_top, 0, margin_bottom, margin_left);
            layer_surface.set_exclusive_zone(-1);

            surface.commit();

            bar.popup_surface = Some(surface);
            bar.popup_layer_surface = Some(layer_surface);
            bar.popup_configured = false;
        } else {
            if let Some(ls) = &bar.popup_layer_surface {
                ls.set_size(pop_w, pop_h);
                ls.set_margin(margin_top, 0, margin_bottom, margin_left);
            }
            if let Some(s) = &bar.popup_surface {
                s.commit();
            }
            if bar.popup_configured {
                self.render_popup(qh, reg_id);
            }
        }
    }

    pub fn show_workspace_popup(&mut self, qh: &QueueHandle<Self>, reg_id: u32, ws_id: i32, anchor_x: i32) {
        if let Some(bar) = self.bars.get(&reg_id) {
            if bar.popover.visible && matches!(bar.popover.kind, PopoverKind::Audio(_) | PopoverKind::Wifi(_)) {
                return;
            }
        }

        let clients = self.mod_workspace.get_clients_for_workspace(ws_id);
        let bar = match self.bars.get_mut(&reg_id) {
            Some(b) => b,
            None => return,
        };

        let mut ws = WorkspacePopup::new(self.config.modules.popup.width, self.config.modules.popup.height);
        ws.update_clients(ws_id, anchor_x, &clients, bar.width, bar.height);

        bar.popover.kind = PopoverKind::Workspace(ws);
        bar.popover.width = self.config.modules.popup.width;
        bar.popover.height = self.config.modules.popup.height;

        self.show_popover_surface(qh, reg_id, anchor_x);
    }

    pub fn toggle_audio_popover(&mut self, qh: &QueueHandle<Self>, reg_id: u32, anchor_x: i32) {
        let already_open = if let Some(bar) = self.bars.get(&reg_id) {
            bar.popover.visible && matches!(bar.popover.kind, PopoverKind::Audio(_))
        } else {
            false
        };
        self.hide_all_popovers();
        if already_open {
            return;
        }

        let vol = self.mod_audio.state.volume.load(std::sync::atomic::Ordering::Relaxed);
        let sink_muted = self.mod_audio.state.sink_muted.load(std::sync::atomic::Ordering::Relaxed);
        let source_muted = self.mod_audio.state.source_muted.load(std::sync::atomic::Ordering::Relaxed);

        let audio_pop = AudioPopover::new(
            320,
            240,
            vol,
            sink_muted,
            source_muted,
            "Default Audio Output (PipeWire)".into(),
        );

        if let Some(bar) = self.bars.get_mut(&reg_id) {
            bar.popover.kind = PopoverKind::Audio(audio_pop);
            bar.popover.width = 320;
            bar.popover.height = 240;
            bar.popover.anim_frame = 0;
            bar.popover.visible = true;
        }

        self.show_popover_surface(qh, reg_id, anchor_x);
    }

    pub fn toggle_wifi_popover(&mut self, qh: &QueueHandle<Self>, reg_id: u32, anchor_x: i32) {
        let already_open = if let Some(bar) = self.bars.get(&reg_id) {
            bar.popover.visible && matches!(bar.popover.kind, PopoverKind::Wifi(_))
        } else {
            false
        };
        self.hide_all_popovers();
        if already_open {
            return;
        }

        let enabled = self.mod_network.get_wifi_enabled();
        let mut networks = self.mod_network.get_cached_networks();
        if networks.is_empty() && enabled {
            networks = NetworkModule::query_wifi_networks_sync();
        }
        let net_state = self.mod_network.get_state();

        let wifi_pop = WifiPopover::new(
            340,
            350,
            enabled,
            net_state.ssid.clone(),
            net_state.signal_percent,
            networks,
        );

        if let Some(bar) = self.bars.get_mut(&reg_id) {
            bar.popover.kind = PopoverKind::Wifi(wifi_pop);
            bar.popover.width = 340;
            bar.popover.height = 350;
            bar.popover.anim_frame = 0;
            bar.popover.visible = true;
        }

        self.show_popover_surface(qh, reg_id, anchor_x);
    }

    pub fn hide_popover(&mut self, reg_id: u32) {
        if let Some(bar) = self.bars.get_mut(&reg_id) {
            if bar.popover.visible || bar.popup_surface.is_some() || bar.backdrop_surface.is_some() {
                bar.popover.visible = false;
                bar.popover.kind = PopoverKind::None;
                bar.popup_configured = false;
                if let Some(ls) = bar.popup_layer_surface.take() {
                    ls.destroy();
                }
                if let Some(s) = bar.popup_surface.take() {
                    s.destroy();
                }
                bar.popup_buffer = None;

                if let Some(bls) = bar.backdrop_layer_surface.take() {
                    bls.destroy();
                }
                if let Some(bs) = bar.backdrop_surface.take() {
                    bs.destroy();
                }
                bar.backdrop_buffer = None;
            }
        }
        self.hovered_ws_pill = None;
    }

    pub fn hide_all_popovers(&mut self) {
        let reg_ids: Vec<u32> = self.bars.keys().copied().collect();
        for id in reg_ids {
            self.hide_popover(id);
        }
    }

    pub fn hide_workspace_popup(&mut self) {
        let mut to_hide = Vec::new();
        for (reg_id, bar) in &self.bars {
            if bar.popover.visible && matches!(bar.popover.kind, PopoverKind::Workspace(_)) {
                to_hide.push(*reg_id);
            }
        }
        for id in to_hide {
            self.hide_popover(id);
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

        if !bar.popup_configured || !bar.popover.visible || bar.popup_surface.is_none() {
            return;
        }

        let pw = bar.popover.width;
        let ph = bar.popover.height;

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
        bar.popover.render(&mut self.renderer, &mut pbuf.mmap[..], &self.config);

        if let Some(s) = &bar.popup_surface {
            s.attach(Some(&pbuf.buffer), 0, 0);
            s.damage_buffer(0, 0, pw as i32, ph as i32);
            s.commit();
        }
    }

    pub fn has_visible_popover(&self) -> bool {
        self.bars.values().any(|b| b.popover.visible)
            || self.mod_screenshot.is_flashing()
            || self.mod_screenshot.needs_reset()
    }

    pub fn anim_tick(&mut self, qh: &QueueHandle<Self>) {
        if self.mod_screenshot.is_flashing() || self.mod_screenshot.take_reset() {
            let reg_ids: Vec<u32> = self.bars.keys().copied().collect();
            for id in reg_ids {
                self.render_bar(qh, id);
            }
        }

        let mut needs_redraw = Vec::new();

        for (&reg_id, bar) in self.bars.iter_mut() {
            if !bar.popover.visible {
                continue;
            }
            bar.popover.anim_frame = bar.popover.anim_frame.wrapping_add(1);

            match &mut bar.popover.kind {
                PopoverKind::Audio(audio) => {
                    let vol = self.mod_audio.state.volume.load(std::sync::atomic::Ordering::Relaxed);
                    let sink_muted = self.mod_audio.state.sink_muted.load(std::sync::atomic::Ordering::Relaxed);
                    let source_muted = self.mod_audio.state.source_muted.load(std::sync::atomic::Ordering::Relaxed);

                    if !self.pointer_pressed {
                        audio.volume = vol;
                    }
                    audio.sink_muted = sink_muted;
                    audio.source_muted = source_muted;
                    needs_redraw.push(reg_id);
                }
                PopoverKind::Wifi(wifi) => {
                    let enabled = self.mod_network.get_wifi_enabled();
                    wifi.wifi_enabled = enabled;

                    let net_state = self.mod_network.get_state();
                    wifi.active_ssid = net_state.ssid.clone();
                    wifi.active_signal = net_state.signal_percent;

                    let cached = self.mod_network.get_cached_networks();
                    if !cached.is_empty() {
                        wifi.networks = cached;
                    }

                    // Reset scanning state after a brief delay (~20 frames = ~1.4s)
                    if wifi.is_scanning && bar.popover.anim_frame % 20 == 0 {
                        wifi.is_scanning = false;
                        if wifi.status_message.as_deref() == Some("Scanning for networks...") {
                            wifi.status_message = None;
                        }
                    }

                    if let Some(ref msg) = wifi.status_message {
                        if msg.starts_with("Connecting to ") {
                            let target = msg.trim_start_matches("Connecting to ").trim_end_matches("...");
                            if !wifi.active_ssid.is_empty() && wifi.active_ssid == target {
                                wifi.status_message = Some(format!("Connected to {}", target));
                            }
                        }
                    }

                    needs_redraw.push(reg_id);
                }
                PopoverKind::Workspace(_) => {}
                PopoverKind::None => {}
            }
        }

        for reg_id in needs_redraw {
            self.render_popup(qh, reg_id);
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
                    if let Some(bls) = bar.backdrop_layer_surface.take() {
                        bls.destroy();
                    }
                    if let Some(bs) = bar.backdrop_surface.take() {
                        bs.destroy();
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

                if id >= 2000 {
                    // Backdrop configure
                    let bar_id = id - 2000;
                    if let Some(bar) = state.bars.get_mut(&bar_id) {
                        if let Some(shm) = &state.shm {
                            let bw = width.max(1);
                            let bh = height.max(1);
                            if bar.backdrop_buffer.as_ref().map_or(true, |b| b.width != bw || b.height != bh) {
                                match ShmBuffer::new(shm, qh, bw, bh) {
                                    Ok(b) => bar.backdrop_buffer = Some(b),
                                    Err(e) => {
                                        eprintln!("[layer_shell] Failed to allocate shm buffer for backdrop: {}", e);
                                    }
                                }
                            }
                            if let (Some(s), Some(buf)) = (&bar.backdrop_surface, &bar.backdrop_buffer) {
                                s.attach(Some(&buf.buffer), 0, 0);
                                s.damage_buffer(0, 0, bw as i32, bh as i32);
                                s.commit();
                            }
                        }
                    }
                } else if id >= 1000 {
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
                if id >= 2000 {
                    let bar_id = id - 2000;
                    if let Some(bar) = state.bars.get_mut(&bar_id) {
                        if let Some(bls) = bar.backdrop_layer_surface.take() {
                            bls.destroy();
                        }
                        if let Some(bs) = bar.backdrop_surface.take() {
                            bs.destroy();
                        }
                        bar.backdrop_buffer = None;
                    }
                } else if id >= 1000 {
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
                        bar.popover.visible = false;
                    }
                } else {
                    state.bars.remove(&id);
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
                state.pointer_pressed = false;
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
                let pressed = btn_state == wayland_client::WEnum::Value(wayland_client::protocol::wl_pointer::ButtonState::Pressed);
                state.pointer_pressed = pressed;
                if pressed {
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
