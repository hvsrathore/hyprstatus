use crate::config::{parse_hex_color, Config};
use crate::ipc::{HyprClient, HyprState};
use crate::modules::{ModuleBlock, PillItem};
use std::collections::HashSet;
use std::sync::{Arc, RwLock};

pub struct WorkspaceModule {
    state: Arc<RwLock<HyprState>>,
    last_render_data: RwLock<Option<(HashSet<i32>, Vec<i32>, i32)>>,
}

impl WorkspaceModule {
    pub fn new(state: Arc<RwLock<HyprState>>) -> Self {
        Self {
            state,
            last_render_data: RwLock::new(None),
        }
    }

    pub fn get_clients_for_workspace(&self, ws_id: i32) -> Vec<HyprClient> {
        if let Ok(st) = self.state.read() {
            st.clients.iter().filter(|c| c.workspace.id == ws_id).cloned().collect()
        } else {
            Vec::new()
        }
    }

    pub fn render(&self, config: &Config, output_name: &str) -> ModuleBlock {
        let (urgent_set, all_workspaces, monitor_ws) = {
            if let Ok(st) = self.state.read() {
                let mon_ws = st.monitors.iter()
                    .find(|m| m.name == output_name)
                    .map(|m| m.activeWorkspace.id)
                    .unwrap_or(st.active_workspace);

                let mut ws_ids = HashSet::new();
                // Display only workspaces containing any windows
                for w in &st.workspaces {
                    if w.windows > 0 {
                        ws_ids.insert(w.id);
                    }
                }
                for c in &st.clients {
                    ws_ids.insert(c.workspace.id);
                }
                // Always include the current active workspace so user sees their current location
                if mon_ws > 0 {
                    ws_ids.insert(mon_ws);
                }

                let mut sorted_ids: Vec<i32> = ws_ids.into_iter().filter(|id| *id > 0).collect();
                sorted_ids.sort_unstable();

                let urg: HashSet<i32> = st.urgent_workspaces.iter().copied().collect();

                let data = (urg, sorted_ids, mon_ws);
                if let Ok(mut last) = self.last_render_data.write() {
                    *last = Some(data.clone());
                }
                data
            } else if let Ok(guard) = self.last_render_data.read() {
                guard.clone().unwrap_or_else(|| (HashSet::new(), vec![1], 1))
            } else {
                (HashSet::new(), vec![1], 1)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::{HyprMonitor, HyprWorkspace, HyprWorkspaceShort};

    #[test]
    fn test_only_workspaces_with_windows_and_active_rendered() {
        let state = Arc::new(RwLock::new(HyprState::default()));
        {
            let mut st = state.write().unwrap();
            st.active_workspace = 2;
            st.monitors = vec![HyprMonitor {
                name: "eDP-1".to_string(),
                activeWorkspace: HyprWorkspaceShort { id: 2, name: "2".to_string() },
                focused: true,
                ..Default::default()
            }];
            st.workspaces = vec![
                HyprWorkspace { id: 1, windows: 0, ..Default::default() }, // empty
                HyprWorkspace { id: 2, windows: 2, ..Default::default() }, // active & has windows
                HyprWorkspace { id: 3, windows: 0, ..Default::default() }, // empty
                HyprWorkspace { id: 4, windows: 1, ..Default::default() }, // inactive & has windows
                HyprWorkspace { id: 5, windows: 0, ..Default::default() }, // empty
            ];
        }

        let module = WorkspaceModule::new(state.clone());
        let config = Config::default();
        let block = module.render(&config, "eDP-1");

        let rendered_ids: Vec<i32> = block.pills.iter().map(|p| p.id).collect();
        // Should only show workspace 2 (active) and workspace 4 (contains windows)
        assert_eq!(rendered_ids, vec![2, 4]);
        assert!(block.pills[0].is_active);
        assert!(!block.pills[1].is_active);

        // Switch to empty workspace 7
        {
            let mut st = state.write().unwrap();
            st.active_workspace = 7;
            st.monitors[0].activeWorkspace.id = 7;
        }

        let block2 = module.render(&config, "eDP-1");
        let rendered_ids2: Vec<i32> = block2.pills.iter().map(|p| p.id).collect();
        // Should only show workspace 2 (has windows), 4 (has windows), and 7 (active)
        assert_eq!(rendered_ids2, vec![2, 4, 7]);
        assert!(!block2.pills[0].is_active); // 2 is inactive
        assert!(!block2.pills[1].is_active); // 4 is inactive
        assert!(block2.pills[2].is_active);  // 7 is active
    }
}
