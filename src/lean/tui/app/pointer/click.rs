use super::header::token_at;
use super::*;

impl App {
    pub(super) fn mouse_click(&mut self, col: u16, row: u16) {
        if self.click_plan_view(col, row) || self.dialog_open() {
            return;
        }
        if self.click_phase_action(col, row) {
            return;
        }
        let area = self.terminal_area;
        let [body, _, bar] = Self::main_areas(area);
        if self.click_help(col, row, bar) || self.click_graph(col, row) {
            return;
        }
        if self.on_header(col, row, bar) {
            self.click_header(col, bar);
        } else if body.contains((col, row).into()) && area.width >= SIDE_BY_SIDE_BREAKPOINT {
            self.click_panel(col, area);
        }
    }

    fn click_plan_view(&mut self, col: u16, row: u16) -> bool {
        let Some(view) = &mut self.plan_view else {
            return false;
        };
        let action = view.mouse_click(col, row, &self.view);
        self.handle_plan_action(action);
        true
    }

    fn dialog_open(&self) -> bool {
        self.diff.is_some()
            || self.details.is_some()
            || self.picker.is_some()
            || self.editor.is_some()
            || self.help.is_some()
    }

    fn click_phase_action(&mut self, col: u16, row: u16) -> bool {
        if !self
            .phase_action_area
            .is_some_and(|area| area.contains((col, row).into()))
        {
            return false;
        }
        if let Some(command) = self.phase_action_command {
            self.run_workflow(command);
        }
        true
    }

    fn click_help(&mut self, col: u16, row: u16, bar: Rect) -> bool {
        if row == bar.y + (Self::bar_height(self.terminal_area) - 1) / 2
            && col >= bar.right().saturating_sub(3)
            && bar.contains((col, row).into())
        {
            self.show_help(self.help_here());
            true
        } else {
            false
        }
    }

    fn click_graph(&mut self, col: u16, row: u16) -> bool {
        let Some(inner) = self
            .graph_area
            .filter(|area| area.contains((col, row).into()))
        else {
            return false;
        };
        let graph = Graph::new_with_viewport(
            &self.view,
            &self.graph_scope,
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
        }
        self.focus = Focus::Graph;
        true
    }

    /// Any row of the bar: its buttons fill all of them.
    fn on_header(&self, col: u16, row: u16, bar: Rect) -> bool {
        bar.contains((col, row).into())
    }

    fn click_header(&mut self, col: u16, bar: Rect) {
        let header = self.header_line(bar.width.saturating_sub(4));
        let text = header
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        let controls = self.view.controls();
        if let Some(token) = token_at(&text, usize::from(col.saturating_sub(bar.x))) {
            self.activate_header_token(&token, controls);
        }
    }
}
