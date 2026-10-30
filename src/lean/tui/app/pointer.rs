use super::super::*;
mod click;
mod header;

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
            self.scroll_graph(down, false);
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

    /// Whether the pointer is over the graph while no overlay covers it.
    fn over_graph(&self, column: u16, row: u16) -> bool {
        self.help.is_none()
            && self.plan_view.is_none()
            && self.diff.is_none()
            && self.details.is_none()
            && self
                .graph_area
                .is_some_and(|area| area.contains(ratatui::layout::Position::new(column, row)))
    }

    fn scroll_graph(&mut self, down: bool, sideways: bool) {
        let graph = Graph::new_with_viewport(
            &self.view,
            &self.graph_scope,
            &self.graph_collapsed,
            self.graph_grouping,
            self.graph_viewport,
        );
        self.pan_graph(&graph, down, if sideways { 6 } else { 3 }, sideways);
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
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                if self.over_graph(mouse.column, mouse.row) =>
            {
                let sideways = mouse.modifiers.contains(KeyModifiers::SHIFT);
                self.scroll_graph(mouse.kind == MouseEventKind::ScrollDown, sideways);
            }
            MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight
                if self.over_graph(mouse.column, mouse.row) =>
            {
                self.scroll_graph(mouse.kind == MouseEventKind::ScrollRight, true);
            }
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
            self.click_graph_filters(mouse);
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
            || self.graph_filters.is_some()
            || self.interview_visible()
    }

    fn click_graph_filters(&mut self, mouse: event::MouseEvent) {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let providers = graph::providers(&self.view);
        let Some(menu) = self.graph_filters.as_mut() else {
            return;
        };
        let outcome = menu.click(
            mouse.column,
            mouse.row,
            &providers,
            &mut self.relations,
            &mut self.graph_scope,
        );
        if outcome == graph_filters::Outcome::Close {
            self.graph_filters = None;
        }
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
}
