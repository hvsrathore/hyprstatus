#![allow(dead_code)]
use serde::{Deserialize, Serialize};
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::{broadcast, RwLock};

#[derive(Debug, Clone)]
pub enum HyprEvent {
    WorkspaceChanged { id: i32, name: String },
    ActiveWindowChanged { class: String, title: String },
    FocusedMonitorChanged { monitor: String, workspace_id: i32 },
    WindowOpened { address: String, workspace: String, class: String, title: String },
    WindowClosed { address: String },
    WindowMoved { address: String, workspace_id: i32 },
    UrgentWindow { address: String },
    LayoutConfigChanged,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HyprWorkspace {
    pub id: i32,
    pub name: String,
    pub monitor: String,
    pub windows: u32,
    pub hasfullscreen: bool,
    pub lastwindow: String,
    pub lastwindowtitle: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct HyprClient {
    pub address: String,
    pub mapped: bool,
    pub hidden: bool,
    pub at: (i32, i32),
    pub size: (i32, i32),
    pub workspace: HyprClientWorkspace,
    pub floating: bool,
    pub monitor: i32,
    pub class: String,
    pub title: String,
    pub initialClass: Option<String>,
    pub initialTitle: Option<String>,
    pub pid: i32,
    pub xwayland: bool,
    pub pinned: bool,
    pub fullscreen: bool,
    pub fullscreenClient: Option<i32>,
    pub grouped: Option<Vec<String>>,
    pub tags: Option<Vec<String>>,
    pub swallowing: Option<String>,
    pub focusHistoryID: Option<i32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HyprClientWorkspace {
    pub id: i32,
    pub name: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[allow(non_snake_case)]
pub struct HyprMonitor {
    pub id: i32,
    pub name: String,
    pub description: String,
    pub make: String,
    pub model: String,
    pub width: i32,
    pub height: i32,
    pub refreshRate: f32,
    pub x: i32,
    pub y: i32,
    pub activeWorkspace: HyprWorkspaceShort,
    pub specialWorkspace: HyprWorkspaceShort,
    pub focused: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HyprWorkspaceShort {
    pub id: i32,
    pub name: String,
}

#[derive(Debug, Clone, Default)]
pub struct HyprState {
    pub active_workspace: i32,
    pub active_window_class: String,
    pub active_window_title: String,
    pub workspaces: Vec<HyprWorkspace>,
    pub monitors: Vec<HyprMonitor>,
    pub clients: Vec<HyprClient>,
    pub urgent_workspaces: Vec<i32>,
}

pub struct HyprlandIpc {
    event_tx: broadcast::Sender<HyprEvent>,
    pub state: Arc<RwLock<HyprState>>,
}

impl HyprlandIpc {
    pub fn new() -> (Self, broadcast::Receiver<HyprEvent>) {
        let (event_tx, rx) = broadcast::channel(100);
        let state = Arc::new(RwLock::new(HyprState::default()));
        (Self { event_tx, state }, rx)
    }

    pub fn get_socket_path() -> Option<PathBuf> {
        let xdg_runtime_dir = env::var("XDG_RUNTIME_DIR").ok()?;
        let his = env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
        let path = PathBuf::from(xdg_runtime_dir).join("hypr").join(his).join(".socket.sock");
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }

    pub fn get_socket2_path() -> Option<PathBuf> {
        let xdg_runtime_dir = env::var("XDG_RUNTIME_DIR").ok()?;
        let his = env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
        let path = PathBuf::from(xdg_runtime_dir).join("hypr").join(his).join(".socket2.sock");
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }

    pub async fn send_command(cmd: &str) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let path = Self::get_socket_path().ok_or("Hyprland socket not found")?;
        let mut stream = UnixStream::connect(path).await?;
        stream.write_all(cmd.as_bytes()).await?;
        stream.shutdown().await?;
        let mut response = String::new();
        stream.read_to_string(&mut response).await?;
        Ok(response)
    }

    pub async fn dispatch(cmd: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let full_cmd = format!("dispatch {}", cmd);
        let _ = Self::send_command(&full_cmd).await?;
        Ok(())
    }

    pub async fn focus_window(address: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr = if address.starts_with("0x") {
            address.to_string()
        } else {
            format!("0x{}", address)
        };
        Self::dispatch(&format!("focuswindow address:{}", addr)).await
    }

    pub async fn switch_workspace(id: i32) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        Self::dispatch(&format!("workspace {}", id)).await
    }

    pub async fn query_workspaces() -> Vec<HyprWorkspace> {
        if let Ok(res) = Self::send_command("j/workspaces").await {
            if let Ok(ws) = serde_json::from_str::<Vec<HyprWorkspace>>(&res) {
                return ws;
            }
        }
        Vec::new()
    }

    pub async fn query_clients() -> Vec<HyprClient> {
        if let Ok(res) = Self::send_command("j/clients").await {
            if let Ok(clients) = serde_json::from_str::<Vec<HyprClient>>(&res) {
                return clients;
            }
        }
        Vec::new()
    }

    pub async fn query_monitors() -> Vec<HyprMonitor> {
        if let Ok(res) = Self::send_command("j/monitors").await {
            if let Ok(monitors) = serde_json::from_str::<Vec<HyprMonitor>>(&res) {
                return monitors;
            }
        }
        Vec::new()
    }

    pub async fn refresh_state(state: &Arc<RwLock<HyprState>>) {
        let ws = Self::query_workspaces().await;
        let clients = Self::query_clients().await;
        let monitors = Self::query_monitors().await;

        let mut st = state.write().await;
        st.workspaces = ws;
        st.clients = clients;
        st.monitors = monitors.clone();

        for m in monitors {
            if m.focused {
                st.active_workspace = m.activeWorkspace.id;
                break;
            }
        }
    }

    pub fn start_listener(&self) {
        let event_tx = self.event_tx.clone();
        let state = self.state.clone();

        tokio::spawn(async move {
            let socket2_path = match Self::get_socket2_path() {
                Some(p) => p,
                None => {
                    eprintln!("[ipc] Hyprland .socket2.sock not found");
                    return;
                }
            };

            // Initial state population
            Self::refresh_state(&state).await;

            loop {
                match UnixStream::connect(&socket2_path).await {
                    Ok(stream) => {
                        println!("[ipc] Connected to Hyprland event socket: {}", socket2_path.display());
                        let reader = BufReader::new(stream);
                        let mut lines = reader.lines();

                        while let Ok(Some(line)) = lines.next_line().await {
                            if let Some((event_name, data)) = line.split_once(">>") {
                                match event_name {
                                    "workspace" => {
                                        let id = data.parse::<i32>().unwrap_or(1);
                                        {
                                            let mut st = state.write().await;
                                            st.active_workspace = id;
                                        }
                                        let _ = event_tx.send(HyprEvent::WorkspaceChanged {
                                            id,
                                            name: data.to_string(),
                                        });
                                    }
                                    "workspacev2" => {
                                        if let Some((id_str, name)) = data.split_once(',') {
                                            let id = id_str.parse::<i32>().unwrap_or(1);
                                            {
                                                let mut st = state.write().await;
                                                st.active_workspace = id;
                                            }
                                            let _ = event_tx.send(HyprEvent::WorkspaceChanged {
                                                id,
                                                name: name.to_string(),
                                            });
                                        }
                                    }
                                    "focusedmon" => {
                                        if let Some((mon, ws_str)) = data.split_once(',') {
                                            let ws_id = ws_str.parse::<i32>().unwrap_or(1);
                                            {
                                                let mut st = state.write().await;
                                                st.active_workspace = ws_id;
                                            }
                                            let _ = event_tx.send(HyprEvent::FocusedMonitorChanged {
                                                monitor: mon.to_string(),
                                                workspace_id: ws_id,
                                            });
                                        }
                                    }
                                    "activewindow" => {
                                        let (class, title) = if let Some((c, t)) = data.split_once(',') {
                                            (c.to_string(), t.to_string())
                                        } else {
                                            (String::new(), String::new())
                                        };
                                        {
                                            let mut st = state.write().await;
                                            st.active_window_class = class.clone();
                                            st.active_window_title = title.clone();
                                        }
                                        let _ = event_tx.send(HyprEvent::ActiveWindowChanged { class, title });
                                    }
                                    "openwindow" | "closewindow" | "movewindow" => {
                                        Self::refresh_state(&state).await;
                                        let _ = event_tx.send(HyprEvent::LayoutConfigChanged);
                                    }
                                    "urgent" => {
                                        let _ = event_tx.send(HyprEvent::UrgentWindow {
                                            address: data.to_string(),
                                        });
                                    }
                                    "configreloaded" => {
                                        let _ = event_tx.send(HyprEvent::LayoutConfigChanged);
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("[ipc] Connection error to Hyprland event socket: {}, retrying in 2s...", e);
                        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
                    }
                }
            }
        });
    }
}
