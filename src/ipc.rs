#![allow(dead_code)]
use serde::{Deserialize, Serialize};
use std::env;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::sync::broadcast;

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

fn deserialize_bool_or_int<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Visitor;
    struct BoolOrIntVisitor;
    impl<'de> Visitor<'de> for BoolOrIntVisitor {
        type Value = bool;
        fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
            formatter.write_str("a boolean or an integer")
        }
        fn visit_bool<E>(self, v: bool) -> Result<bool, E> {
            Ok(v)
        }
        fn visit_i64<E>(self, v: i64) -> Result<bool, E> {
            Ok(v != 0)
        }
        fn visit_u64<E>(self, v: u64) -> Result<bool, E> {
            Ok(v != 0)
        }
    }
    deserializer.deserialize_any(BoolOrIntVisitor)
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
    #[serde(default, deserialize_with = "deserialize_bool_or_int")]
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
        let fut = async {
            let mut stream = UnixStream::connect(path).await?;
            stream.write_all(cmd.as_bytes()).await?;
            stream.shutdown().await?;
            let mut response = String::new();
            stream.read_to_string(&mut response).await?;
            Ok::<String, Box<dyn std::error::Error + Send + Sync>>(response)
        };
        match tokio::time::timeout(tokio::time::Duration::from_millis(500), fut).await {
            Ok(res) => res,
            Err(_) => Err("Hyprland socket command timed out".into()),
        }
    }

    pub async fn dispatch(cmd: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // If it's a legacy workspace or focuswindow command, try Hyprland 0.56+ Lua dispatch first
        if let Some(target) = cmd.strip_prefix("workspace ") {
            let lua_cmd = if let Ok(id) = target.trim().parse::<i32>() {
                format!("dispatch hl.dsp.focus({{workspace = {}}})", id)
            } else {
                format!("dispatch hl.dsp.focus({{workspace = \"{}\"}})", target.trim())
            };
            if let Ok(res) = Self::send_command(&lua_cmd).await {
                if res.trim() == "ok" {
                    return Ok(());
                }
            }
        } else if let Some(target) = cmd.strip_prefix("focuswindow ") {
            let addr = target.trim();
            let lua_cmd = format!("dispatch hl.dsp.focus({{window = '{}'}})", addr);
            if let Ok(res) = Self::send_command(&lua_cmd).await {
                if res.trim() == "ok" {
                    return Ok(());
                }
            }
        }

        let full_cmd = format!("dispatch {}", cmd);
        let res = Self::send_command(&full_cmd).await?;
        if res.starts_with("error:") {
            eprintln!("[ipc] Dispatch returned error: {}", res);
        }
        Ok(())
    }

    pub async fn focus_window(address: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let addr = if address.starts_with("0x") {
            address.to_string()
        } else {
            format!("0x{}", address)
        };
        // Hyprland 0.56+ Lua dispatch syntax
        let lua_cmd = format!("dispatch hl.dsp.focus({{window = 'address:{}'}})", addr);
        if let Ok(res) = Self::send_command(&lua_cmd).await {
            if res.trim() == "ok" {
                return Ok(());
            }
        }
        // Fallback for pre-0.56 Hyprland
        Self::dispatch(&format!("focuswindow address:{}", addr)).await
    }

    pub async fn switch_workspace(id: i32) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        // Hyprland 0.56+ Lua dispatch syntax
        let lua_cmd = format!("dispatch hl.dsp.focus({{workspace = {}}})", id);
        if let Ok(res) = Self::send_command(&lua_cmd).await {
            if res.trim() == "ok" {
                return Ok(());
            }
        }
        // Fallback for pre-0.56 Hyprland
        Self::dispatch(&format!("workspace {}", id)).await
    }

    pub async fn cycle_workspace(next: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let rel = if next { "m+1" } else { "m-1" };
        let lua_cmd = format!("dispatch hl.dsp.focus({{workspace = \"{}\"}})", rel);
        if let Ok(res) = Self::send_command(&lua_cmd).await {
            if res.trim() == "ok" {
                return Ok(());
            }
        }
        // Fallback for pre-0.56 Hyprland
        Self::dispatch(&format!("workspace {}", rel)).await
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
        let (ws, clients, monitors) = tokio::join!(
            Self::query_workspaces(),
            Self::query_clients(),
            Self::query_monitors()
        );

        let mut st = match state.write() {
            Ok(s) => s,
            Err(e) => e.into_inner(),
        };
        st.workspaces = ws;
        st.clients = clients;

        // Preserve in-memory active workspace in case a newer workspace switch event
        // arrived while queries were in flight
        let current_active_ws = st.active_workspace;
        let mut updated_monitors = monitors;
        let single_mon = updated_monitors.len() == 1;
        if current_active_ws > 0 {
            for m in &mut updated_monitors {
                if m.focused || single_mon {
                    m.activeWorkspace.id = current_active_ws;
                    m.activeWorkspace.name = current_active_ws.to_string();
                }
            }
        } else {
            for m in &updated_monitors {
                if m.focused {
                    st.active_workspace = m.activeWorkspace.id;
                    break;
                }
            }
        }
        st.monitors = updated_monitors;
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

            // Background debounced state refresh worker to avoid blocking the event loop
            let (refresh_tx, mut refresh_rx) = tokio::sync::mpsc::channel::<()>(32);
            let bg_state = state.clone();
            let bg_event_tx = event_tx.clone();

            tokio::spawn(async move {
                while refresh_rx.recv().await.is_some() {
                    // Debounce rapid window/layout events (e.g. app launch)
                    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
                    while refresh_rx.try_recv().is_ok() {}

                    Self::refresh_state(&bg_state).await;
                    let _ = bg_event_tx.send(HyprEvent::LayoutConfigChanged);
                }
            });

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
                                        let changed = {
                                            let mut st = match state.write() {
                                                Ok(s) => s,
                                                Err(e) => e.into_inner(),
                                            };
                                            st.urgent_workspaces.retain(|&w| w != id);
                                            let mut any_changed = st.active_workspace != id;
                                            st.active_workspace = id;
                                            let single_mon = st.monitors.len() == 1;
                                            for m in &mut st.monitors {
                                                if m.focused || single_mon {
                                                    if m.activeWorkspace.id != id {
                                                        any_changed = true;
                                                        m.activeWorkspace.id = id;
                                                        m.activeWorkspace.name = data.to_string();
                                                    }
                                                }
                                            }
                                            any_changed
                                        };
                                        if changed {
                                            let _ = event_tx.send(HyprEvent::WorkspaceChanged {
                                                id,
                                                name: data.to_string(),
                                            });
                                        }
                                    }
                                    "workspacev2" => {
                                        if let Some((id_str, name)) = data.split_once(',') {
                                            let id = id_str.parse::<i32>().unwrap_or(1);
                                            let changed = {
                                                let mut st = match state.write() {
                                                    Ok(s) => s,
                                                    Err(e) => e.into_inner(),
                                                };
                                                st.urgent_workspaces.retain(|&w| w != id);
                                                let mut any_changed = st.active_workspace != id;
                                                st.active_workspace = id;
                                                let single_mon = st.monitors.len() == 1;
                                                for m in &mut st.monitors {
                                                    if m.focused || single_mon {
                                                        if m.activeWorkspace.id != id || m.activeWorkspace.name != name {
                                                            any_changed = true;
                                                            m.activeWorkspace.id = id;
                                                            m.activeWorkspace.name = name.to_string();
                                                        }
                                                    }
                                                }
                                                any_changed
                                            };
                                            if changed {
                                                let _ = event_tx.send(HyprEvent::WorkspaceChanged {
                                                    id,
                                                    name: name.to_string(),
                                                });
                                            }
                                        }
                                    }
                                    "focusedmon" => {
                                        if let Some((mon, ws_str)) = data.split_once(',') {
                                            let ws_id = ws_str.parse::<i32>().unwrap_or(1);
                                            let changed = {
                                                let mut st = match state.write() {
                                                    Ok(s) => s,
                                                    Err(e) => e.into_inner(),
                                                };
                                                let mut any_changed = st.active_workspace != ws_id;
                                                st.active_workspace = ws_id;
                                                for m in &mut st.monitors {
                                                    let was_focused = m.focused;
                                                    m.focused = m.name == mon;
                                                    if was_focused != m.focused {
                                                        any_changed = true;
                                                    }
                                                    if m.name == mon {
                                                        if m.activeWorkspace.id != ws_id {
                                                            any_changed = true;
                                                            m.activeWorkspace.id = ws_id;
                                                            m.activeWorkspace.name = ws_str.to_string();
                                                        }
                                                    }
                                                }
                                                any_changed
                                            };
                                            if changed {
                                                let _ = event_tx.send(HyprEvent::FocusedMonitorChanged {
                                                    monitor: mon.to_string(),
                                                    workspace_id: ws_id,
                                                });
                                            }
                                        }
                                    }
                                    "createworkspace" | "createworkspacev2"
                                    | "destroyworkspace" | "destroyworkspacev2"
                                    | "moveworkspace" | "moveworkspacev2"
                                    | "activespecial" => {
                                        let _ = refresh_tx.try_send(());
                                    }
                                    "activewindow" => {
                                        let (class, title) = if let Some((c, t)) = data.split_once(',') {
                                            (c.to_string(), t.to_string())
                                        } else {
                                            (String::new(), String::new())
                                        };
                                        {
                                            let mut st = match state.write() {
                                                Ok(s) => s,
                                                Err(e) => e.into_inner(),
                                            };
                                            st.active_window_class = class.clone();
                                            st.active_window_title = title.clone();
                                        }
                                        let _ = event_tx.send(HyprEvent::ActiveWindowChanged { class, title });
                                    }
                                    "openwindow" | "closewindow" | "movewindow" | "movewindowv2"
                                    | "windowtitle" | "windowtitlev2" | "changefloatingmode"
                                    | "fullscreen" | "pin" => {
                                        let _ = refresh_tx.try_send(());
                                    }
                                    "urgent" => {
                                        let addr = data.trim();
                                        {
                                            let mut st = match state.write() {
                                                Ok(s) => s,
                                                Err(e) => e.into_inner(),
                                            };
                                            let ws_id = st.clients.iter()
                                                .find(|c| c.address.trim_start_matches("0x").eq_ignore_ascii_case(addr.trim_start_matches("0x")))
                                                .map(|c| c.workspace.id);
                                            if let Some(w) = ws_id {
                                                if !st.urgent_workspaces.contains(&w) {
                                                    st.urgent_workspaces.push(w);
                                                }
                                            }
                                        }
                                        let _ = event_tx.send(HyprEvent::UrgentWindow {
                                            address: addr.to_string(),
                                        });
                                    }
                                    "configreloaded" => {
                                        let _ = refresh_tx.try_send(());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_query_clients() {
        let clients = HyprlandIpc::query_clients().await;
        println!("SUCCESS parsed {} clients: {:?}", clients.len(), clients);
        assert!(!clients.is_empty());
    }

    #[tokio::test]
    async fn test_switch_workspace() {
        // Query active workspace
        let res = HyprlandIpc::switch_workspace(3).await;
        assert!(res.is_ok(), "switch_workspace to 3 failed: {:?}", res);
    }
}
