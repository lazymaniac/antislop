use super::*;

impl App {
    // The pointer adapter has already dispatched modal-owned events.
    pub(super) fn mouse_click(&mut self, col: u16, row: u16) {
        if self.click_plan_view(col, row) || self.content_covers_graph() {
            return;
        }
        if self.click_phase_action(col, row) {
            return;
        }
        let _ = self.click_now(col, row) || self.click_graph(col, row);
    }

    fn click_plan_view(&mut self, col: u16, row: u16) -> bool {
        let Some(view) = &mut self.plan_view else {
            return false;
        };
        let action = view.mouse_click(col, row, &self.view);
        self.handle_plan_action(action);
        true
    }

    fn content_covers_graph(&self) -> bool {
        self.diff.is_some() || self.details.is_some()
    }

    fn click_phase_action(&mut self, col: u16, row: u16) -> bool {
        if !self
            .phase_action_area
            .is_some_and(|area| area.contains((col, row).into()))
        {
            return false;
        }
        if let Some(command) = self.phase_action_command {
            if command == Command::TaskActions
                && let Some(stopped) = self
                    .view
                    .tasks
                    .iter()
                    .find(|t| matches!(t.status.as_str(), "failed" | "needs_user"))
                    .map(|t| t.id.clone())
            {
                self.selected = format!("worker:{stopped}");
                self.open_unblock(Some(stopped));
            } else {
                self.run_workflow(command);
            }
        }
        true
    }

    fn click_graph(&mut self, col: u16, row: u16) -> bool {
        let Some(inner) = self
            .graph_area
            .filter(|area| area.contains((col, row).into()))
        else {
            return false;
        };
        let scope = self.graph_scope_with_exception();
        let graph = Graph::new_with_viewport(
            &self.view,
            &scope,
            &self.graph_collapsed,
            self.graph_grouping,
            self.graph_viewport,
        );
        let x = col.saturating_sub(inner.x).saturating_add(self.offset.0);
        let y = row.saturating_sub(inner.y).saturating_add(self.offset.1);
        if let Some(node) = graph.nodes.iter().find(|node| {
            node.visible
                && x >= node.x
                && x < node.x.saturating_add(node.width)
                && y >= node.y
                && y < node.y.saturating_add(node.height)
        }) {
            self.select_node(node.key.clone());
            if self.graph_link_source.is_some() {
                self.toggle_flow_link();
            }
        }
        self.set_focus(Focus::Graph);
        true
    }

    fn click_now(&mut self, col: u16, row: u16) -> bool {
        let Some(area) = self
            .now_area
            .filter(|area| area.contains((col, row).into()))
        else {
            return false;
        };
        if area.height > 1 && (row <= area.y || row >= area.bottom().saturating_sub(1)) {
            return true;
        }
        let line = if area.height == 1 {
            0
        } else {
            row.saturating_sub(area.y).saturating_sub(1) as usize
        };
        let Some((agent, visible)) = self.now_click_targets.get(line).and_then(Clone::clone) else {
            return true;
        };
        let selected = graph::card_key(&self.view, &agent);
        self.selected = selected.clone();
        if !visible {
            self.graph_visibility_exception = Some(selected);
        }
        self.graph_revealed = None;
        self.set_focus(Focus::Graph);
        true
    }
}
