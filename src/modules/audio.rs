#![allow(dead_code)]
use crate::config::{parse_hex_color, Config};
use crate::modules::{ClickAction, ClickRegion, ModuleBlock};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use tokio::process::Command;

pub struct AudioState {
    pub volume: AtomicU32,
    pub sink_muted: AtomicBool,
    pub source_muted: AtomicBool,
    pub source_active: AtomicBool,
}

impl Default for AudioState {
    fn default() -> Self {
        Self {
            volume: AtomicU32::new(100),
            sink_muted: AtomicBool::new(false),
            source_muted: AtomicBool::new(false),
            source_active: AtomicBool::new(false),
        }
    }
}

#[derive(Clone)]
pub struct AudioModule {
    state: Arc<AudioState>,
}

impl AudioModule {
    pub fn new() -> Self {
        let state = Arc::new(AudioState::default());
        let module = Self { state: state.clone() };

        // Initialize PipeWire monitoring thread
        module.spawn_pipewire_monitor(state.clone());

        // Spawn periodic poller to guarantee synchronization with pactl/pw-dump
        module.spawn_periodic_sync(state);

        module
    }

    fn spawn_pipewire_monitor(&self, _state: Arc<AudioState>) {
        std::thread::spawn(move || {
            pipewire::init();
            let mainloop = match pipewire::main_loop::MainLoopRc::new(None) {
                Ok(ml) => ml,
                Err(e) => {
                    eprintln!("[audio] Failed to create PipeWire mainloop: {}", e);
                    return;
                }
            };

            let context = match pipewire::context::ContextRc::new(&mainloop, None) {
                Ok(ctx) => ctx,
                Err(e) => {
                    eprintln!("[audio] Failed to create PipeWire context: {}", e);
                    return;
                }
            };

            let core = match context.connect_rc(None) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("[audio] Failed to connect to PipeWire core: {}", e);
                    return;
                }
            };

            let registry = match core.get_registry() {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("[audio] Failed to get PipeWire registry: {}", e);
                    return;
                }
            };

            let _listener = registry
                .add_listener_local()
                .global(move |global| {
                    if global.type_ == pipewire::types::ObjectType::Node
                        || global.type_ == pipewire::types::ObjectType::Metadata
                    {
                        // Signal change
                    }
                })
                .register();

            println!("[audio] PipeWire native client initialized successfully");
            mainloop.run();
        });
    }

    fn spawn_periodic_sync(&self, state: Arc<AudioState>) {
        tokio::spawn(async move {
            loop {
                Self::query_pactl_audio(&state).await;
                tokio::time::sleep(tokio::time::Duration::from_millis(800)).await;
            }
        });
    }

    pub async fn query_pactl_audio(state: &Arc<AudioState>) {
        // Query default sink volume and mute
        if let Ok(out) = Command::new("pactl")
            .args(["get-sink-volume", "@DEFAULT_SINK@"])
            .output()
            .await
        {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                // Look for "/ 100% /"
                if let Some(pct_idx) = text.find('%') {
                    let pre = &text[..pct_idx];
                    if let Some(slash_idx) = pre.rfind('/') {
                        let num_str = pre[slash_idx + 1..].trim();
                        if let Ok(vol) = num_str.parse::<u32>() {
                            state.volume.store(vol, Ordering::Relaxed);
                        }
                    }
                }
            }
        }

        // Query sink mute
        if let Ok(out) = Command::new("pactl")
            .args(["get-sink-mute", "@DEFAULT_SINK@"])
            .output()
            .await
        {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                state.sink_muted.store(text.contains("yes"), Ordering::Relaxed);
            }
        }

        // Query source mute
        if let Ok(out) = Command::new("pactl")
            .args(["get-source-mute", "@DEFAULT_SOURCE@"])
            .output()
            .await
        {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                state.source_muted.store(text.contains("yes"), Ordering::Relaxed);
            }
        }
    }

    pub async fn volume_up(&self, step: u32) {
        let cur = self.state.volume.load(Ordering::Relaxed);
        let new_vol = (cur + step).min(150);
        self.state.volume.store(new_vol, Ordering::Relaxed);
        let _ = Command::new("pactl")
            .args(["set-sink-volume", "@DEFAULT_SINK@", &format!("+{}%", step)])
            .spawn();
    }

    pub async fn volume_down(&self, step: u32) {
        let cur = self.state.volume.load(Ordering::Relaxed);
        let new_vol = cur.saturating_sub(step);
        self.state.volume.store(new_vol, Ordering::Relaxed);
        let _ = Command::new("pactl")
            .args(["set-sink-volume", "@DEFAULT_SINK@", &format!("-{}%", step)])
            .spawn();
    }

    pub async fn toggle_mute(&self) {
        let cur = self.state.sink_muted.load(Ordering::Relaxed);
        self.state.sink_muted.store(!cur, Ordering::Relaxed);
        let _ = Command::new("pactl")
            .args(["set-sink-mute", "@DEFAULT_SINK@", "toggle"])
            .spawn();
    }

    pub async fn toggle_mic_mute(&self) {
        let cur = self.state.source_muted.load(Ordering::Relaxed);
        self.state.source_muted.store(!cur, Ordering::Relaxed);
        let _ = Command::new("pactl")
            .args(["set-source-mute", "@DEFAULT_SOURCE@", "toggle"])
            .spawn();
    }

    pub fn render(&self, config: &Config) -> ModuleBlock {
        let vol = self.state.volume.load(Ordering::Relaxed);
        let muted = self.state.sink_muted.load(Ordering::Relaxed);
        let mic_muted = self.state.source_muted.load(Ordering::Relaxed);

        let icon = if muted {
            "󰝟" // Muted icon
        } else if vol >= 70 {
            "󰕾" // High volume
        } else if vol >= 30 {
            "󰖀" // Medium volume
        } else {
            "󰕿" // Low volume
        };

        let mic_icon = if !mic_muted { " 󰍬" } else { "" };

        let text = if muted {
            format!("{} Muted{}", icon, mic_icon)
        } else {
            format!("{} {}%{}", icon, vol, mic_icon)
        };

        let fg = parse_hex_color(&config.colors.audio);
        let bg = parse_hex_color(&config.colors.block_background);

        let mut block = ModuleBlock::new("audio", &text, fg, bg);
        block.icon = icon.to_string();

        // Register click regions
        block.click_regions.push(ClickRegion {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
            action: ClickAction::ToggleAudioMute,
        });

        block
    }
}
