use crate::config::{parse_hex_color, Config};
use crate::ipc::{HyprClient, HyprState};
use crate::modules::{ModuleBlock, PillItem};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

pub struct WorkspaceModule {
    state: Arc<RwLock<HyprState>>,
}

impl WorkspaceModule {
    pub fn new(state: Arc<RwLock<HyprState>>) -> Self {
        Self { state }
    }

    pub fn get_clients_for_workspace(&self, ws_id: i32) -> Vec<HyprClient> {
        if let Ok(st) = self.state.try_read() {
            st.clients.iter().filter(|c| c.workspace.id == ws_id).cloned().collect()
        } else {
            Vec::new()
        }
    }

    pub fn render(&self, config: &Config, output_name: &str) -> ModuleBlock {
        let (_active_ws, urgent_set, all_workspaces, monitor_ws) = {
            if let Ok(st) = self.state.try_read() {
                let mon_ws = st.monitors.iter()
                    .find(|m| m.name == output_name)
                    .map(|m| m.activeWorkspace.id)
                    .unwrap_or(st.active_workspace);

                let mut ws_ids = HashSet::new();
                for w in &st.workspaces {
                    ws_ids.insert(w.id);
                }
                for c in &st.clients {
                    ws_ids.insert(c.workspace.id);
                }
                ws_ids.insert(mon_ws);
                // Always show at least workspaces 1..=5 if few exist
                for i in 1..=5 {
                    ws_ids.insert(i);
                }

                let mut sorted_ids: Vec<i32> = ws_ids.into_iter().filter(|id| *id > 0).collect();
                sorted_ids.sort_unstable();

                let urg: HashSet<i32> = st.urgent_workspaces.iter().copied().collect();

                (st.active_workspace, urg, sorted_ids, mon_ws)
            } else {
                (1, HashSet::new(), vec![1, 2, 3, 4, 5], 1)
            }
        };

        let mut pills = Vec::new();

        let active_bg = parse_hex_color(&config.colors.active_workspace_bg);
        let active_fg = parse_hex_color(&config.colors.active_workspace_fg);
        let inactive_bg = parse_hex_color(&config.colors.inactive_workspace_bg);
        let inactive_fg = parse_hex_color(&config.colors.inactive_workspace_fg);
        let urgent_bg = parse_hex_color(&config.colors.urgent_workspace_bg);
        let urgent_fg = parse_hex_color(&config.colors.urgent_workspace_fg);

        for id in all_workspaces {
            let is_active = id == monitor_ws;
            let is_urgent = urgent_set.contains(&id);

            let (fg, bg) = if is_urgent {
                (urgent_fg, urgent_bg)
            } else if is_active {
                (active_fg, active_bg)
            } else {
                (inactive_fg, inactive_bg)
            };

            let label = format!("{}", id);

            pills.push(PillItem {
                id,
                label,
                fg,
                bg,
                border: if is_active { Some(parse_hex_color(&config.colors.border)) } else { None },
                is_active,
                is_urgent,
            });
        }

        let mut block = ModuleBlock::new("workspaces", "", inactive_fg, inactive_bg);
        block.pills = pills;
        block
    }
}
