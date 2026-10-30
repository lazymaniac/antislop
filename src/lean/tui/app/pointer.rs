use super::super::*;
mod header;
use header::token_at;

impl App {
    pub(in crate::lean::tui) fn bar_height(area: Rect) -> u16 {
        if area.width >= SIDE_BY_SIDE_BREAKPOINT && area.height >= 30 {
            3
        } else {
            2
        }
    }

    pub(in crate::lean::tui) fn main_areas(area: Rect) -> [Rect; 3] {
        Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(u16::from(area.height >= 6)),
            Constraint::Length(Self::bar_height(area)),
        ])
        .areas(area)
    }

    pub(in crate::lean::tui) fn mouse_scroll(&mut self, down: bool) {
        if self.scroll_help(down) {
            return;
        }
        if let Some(view) = &mut self.plan_view {
            view.key(
                event::KeyEvent::new(
                    if down { KeyCode::Down } else { KeyCode::Up },
                    KeyModifiers::NONE,
                ),
                &self.view,
            );
        } else if let Some(diff) = &mut self.diff {
            diff.key(if down { KeyCode::Down } else { KeyCode::Up });
            return;
        }
        if self.scroll_details(down) {
            return;
        }
        if self.focus == Focus::Graph {
            self.scroll_graph(down);
        } else {
            self.scroll_chat(down);
        }
    }

    fn scroll_help(&mut self, down: bool) -> bool {
        if self.help.is_none() {
            return false;
        }
        self.help_scroll = if down {
            (self.help_scroll + 3).min(self.help_max_scroll())
        } else {
            self.help_scroll.saturating_sub(3)
        };
        true
    }

    fn scroll_details(&mut self, down: bool) -> bool {
        let Some(details) = &mut self.details else {
            return false;
        };
        details.pending_g = false;
        if !down {
            details.follow = false;
        }
        details.scroll = if down {
            details.scroll.saturating_add(3)
        } else {
            details.scroll.saturating_sub(3)
        };
        true
    }

    fn scroll_graph(&mut self, down: bool) {
        let graph = Graph::new_with_viewport(
            &self.view,
            self.graph_filter.as_deref(),
            &self.graph_collapsed,
            self.graph_grouping,
            self.graph_viewport,
        );
        self.scroll_graph_node(&graph, down, 3);
    }

    fn scroll_chat(&mut self, down: bool) {
        self.scroll = if down {
            self.scroll.saturating_sub(3)
        } else {
            self.scroll + 3
        };
    }

    pub fn mouse(&mut self, mouse: event::MouseEvent) -> bool {
        if self.handle_native_mouse(mouse) {
            return true;
        }
        if self.plan_history.is_some() {
            return true;
        }
        self.sync_interview();
        if self.handle_modal_mouse(mouse) {
            return true;
        }
        self.handle_content_mouse(mouse);
        true
    }

    fn handle_content_mouse(&mut self, mouse: event::MouseEvent) {
        self.update_selection_context();
        if !self.selection_blocked() {
            self.text_selection.mouse(mouse);
        }
        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                self.mouse_scroll(mouse.kind == MouseEventKind::ScrollDown);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                self.mouse_click(mouse.column, mouse.row);
            }
            _ => {}
        }
    }

    fn handle_native_mouse(&mut self, mut mouse: event::MouseEvent) -> bool {
        if !self.native_focused() {
            return false;
        }
        if let Some(details) = &self.details {
            let area = details.native_areas[details.active_panel];
            if area.contains((mouse.column, mouse.row).into()) {
                mouse.column -= area.x;
                mouse.row -= area.y;
                self.native_send(crate::lean::terminal::Input::Mouse { event: mouse });
            }
        }
        true
    }

    fn handle_modal_mouse(&mut self, mouse: event::MouseEvent) -> bool {
        if let Some(view) = self.ledger_view.as_mut().filter(|view| view.visible) {
            let action = view.mouse(mouse);
            self.ledger_action(action);
            return true;
        }
        self.update_selection_context();
        if self.modal_blocks_mouse() {
            self.click_unblock(mouse);
            return true;
        }
        false
    }

    fn update_selection_context(&mut self) {
        if self.selection_blocked() {
            self.text_selection = Default::default();
        }
        self.text_selection
            .invalidate_context(&self.selection_context());
    }

    fn modal_blocks_mouse(&self) -> bool {
        self.config_editor
            .as_ref()
            .is_some_and(|editor| editor.visible)
            || self.trust_prompt.is_some()
            || self.project_trust.is_some()
            || self.unblock.is_some()
            || self.interview_visible()
    }

    fn click_unblock(&mut self, mouse: event::MouseEvent) {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let outcome = self
            .unblock
            .as_mut()
            .map(|modal| modal.click(mouse.column, mouse.row));
        if let Some(unblock::Outcome::Choose(choice)) = outcome
            && let Some(state) = self.unblock.take()
        {
            self.unblock_choice(state, choice);
        }
    }

    fn mouse_click(&mut self, col: u16, row: u16) {
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
            self.command(command);
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
            self.graph_filter.as_deref(),
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

    fn on_header(&self, col: u16, row: u16, bar: Rect) -> bool {
        row == bar.y + (Self::bar_height(self.terminal_area) - 1) / 2
            && bar.contains((col, row).into())
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
