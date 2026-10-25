mod config;
mod ipc;
mod layer_shell;
mod modules;
mod renderer;

use config::{load_or_create_config, watch_config_file};
use ipc::{HyprEvent, HyprlandIpc};
use layer_shell::BarState;
use modules::audio::AudioModule;
use modules::battery::BatteryModule;
use modules::clock::ClockModule;
use modules::network::NetworkModule;
use modules::perf::PerfModule;
use modules::screenshot::ScreenshotModule;
use modules::tray::TrayModule;
use modules::window_title::WindowTitleModule;
use modules::workspace::WorkspaceModule;
use std::os::unix::io::{AsFd, AsRawFd, RawFd};
use std::sync::Arc;
use tokio::io::unix::AsyncFd;
use tokio::sync::watch;
use wayland_client::Connection;

struct WaylandFd(Connection);

impl AsRawFd for WaylandFd {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_fd().as_raw_fd()
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() > 1 && (args[1] == "--wifi-menu" || args[1] == "wifi-menu") {
        NetworkModule::open_wifi_modal().await;
        return Ok(());
    }

    println!("------------------------------------------------------------");
    println!("hyprstatus - Blazing-fast i3-style Status Bar for Hyprland");
    println!("------------------------------------------------------------");

    // 1. Load initial configuration
    let initial_config = Arc::new(load_or_create_config());
    let (config_tx, mut config_rx) = watch::channel(initial_config.clone());

    // 2. Start config hot-reload watcher
    watch_config_file(config_tx);

    // 3. Connect to Hyprland IPC
    let (hypr_ipc, mut hypr_rx) = HyprlandIpc::new();
    hypr_ipc.start_listener();

    // 4. Initialize Modular Status Widgets
    let mod_workspace = WorkspaceModule::new(hypr_ipc.state.clone());
    let mod_window_title = WindowTitleModule::new(hypr_ipc.state.clone());
    let mod_audio = AudioModule::new();
    let mod_network = NetworkModule::new();
    let mod_battery = BatteryModule::new();
    let mod_perf = PerfModule::new();
    let mod_clock = ClockModule::new();
    let mod_tray = TrayModule::new();
    let mod_screenshot = ScreenshotModule::new();

    // 5. Connect to Wayland compositor
    let conn = match Connection::connect_to_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[wayland] Failed to connect to Wayland display: {}", e);
            eprintln!("[wayland] Ensure WAYLAND_DISPLAY is set and Hyprland is running.");
            return Err(e.into());
        }
    };

    println!("[wayland] Connected to Wayland compositor");

    let mut event_queue = conn.new_event_queue();
    let qh = event_queue.handle();

    let mut state = BarState::new(
        initial_config,
        mod_workspace,
        mod_window_title,
        mod_audio,
        mod_network,
        mod_battery,
        mod_perf,
        mod_clock,
        mod_tray,
        mod_screenshot,
    );

    // Get display registry and discover globals
    let display = conn.display();
    let _registry = display.get_registry(&qh, ());

    // Initial roundtrips to bind globals and register monitors
    event_queue.roundtrip(&mut state)?;
    event_queue.roundtrip(&mut state)?;
    event_queue.roundtrip(&mut state)?;

    println!("[wayland] Initialized {} active monitor bar(s)", state.bars.len());

    let async_fd = unsafe {
        AsyncFd::register_with_interest(WaylandFd(conn.clone()), tokio::io::Interest::READABLE)?
    };
    let mut interval = tokio::time::interval(tokio::time::Duration::from_millis(1000));
    let mut anim_interval = tokio::time::interval(tokio::time::Duration::from_millis(70));
    anim_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        // 1. Dispatch any pending Wayland messages already in queue
        event_queue.dispatch_pending(&mut state)?;
        conn.flush()?;

        // 2. Prepare read guard
        let read_guard = event_queue.prepare_read();

        tokio::select! {
            _ = anim_interval.tick(), if state.has_visible_popover() => {
                std::mem::drop(read_guard);
                state.anim_tick(&qh);
                conn.flush()?;
            }
            res = async_fd.readable() => {
                let mut ready_guard = res?;
                ready_guard.clear_ready();
                if let Some(guard) = read_guard {
                    if let Err(e) = guard.read() {
                        eprintln!("[wayland] Socket read error: {}", e);
                    }
                }
                event_queue.dispatch_pending(&mut state)?;
            }
            _ = interval.tick() => {
                std::mem::drop(read_guard);
                let reg_ids: Vec<u32> = state.bars.keys().copied().collect();
                for id in reg_ids {
                    state.render_bar(&qh, id);
                }
            }
            Ok(()) = config_rx.changed() => {
                std::mem::drop(read_guard);
                let new_cfg = config_rx.borrow().clone();
                println!("[main] Applying hot-reloaded configuration to status bars");
                state.config = new_cfg.clone();
                state.renderer = renderer::SoftwareRenderer::new(&new_cfg.bar.font_path);
                let reg_ids: Vec<u32> = state.bars.keys().copied().collect();
                for id in reg_ids {
                    state.render_bar(&qh, id);
                }
            }
            Ok(event) = hypr_rx.recv() => {
                std::mem::drop(read_guard);
                if state.has_visible_popover() {
                    state.hide_all_popovers();
                }
                match event {
                    HyprEvent::WorkspaceChanged { .. }
                    | HyprEvent::ActiveWindowChanged { .. }
                    | HyprEvent::FocusedMonitorChanged { .. }
                    | HyprEvent::LayoutConfigChanged => {
                        let reg_ids: Vec<u32> = state.bars.keys().copied().collect();
                        for id in reg_ids {
                            state.render_bar(&qh, id);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}
