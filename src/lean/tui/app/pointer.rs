use super::super::*;
mod click;

impl App {
    pub(in crate::lean::tui) fn bar_height(area: Rect) -> u16 {
        let _ = area;
        1
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
        if self.picker.is_some() || self.editor.is_some() {
            return;
        }
        if self.interview_visible() {
            if let Some(form) = self.interview_forms.get_mut(&self.conversation) {
                form.scroll_by(down, 3);
            }
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
            return;
        } else if let Some(diff) = &mut self.diff {
            diff.key(if down { KeyCode::Down } else { KeyCode::Up });
            return;
        }
        if self.graph_expanded.is_none() && self.scroll_details(down) {
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
        // The expanded card is part of the layout the view pans over.
        let graph = self.graph_for_key();
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
        let before = (
            self.navigation.clone(),
            self.selected.clone(),
            self.details.as_ref().map(|details| details.key.clone()),
        );
        let result = self.mouse_inner(mouse);
        let after = (
            self.navigation.clone(),
            self.selected.clone(),
            self.details.as_ref().map(|details| details.key.clone()),
        );
        if before != after {
            self.navigation_revision = self.navigation_revision.wrapping_add(1);
        }
        result
    }

    fn mouse_inner(&mut self, mouse: event::MouseEvent) -> bool {
        if self.navigation.focus == NavigationFocus::Composer
            || self.details.as_ref().is_some_and(|details| details.editing)
        {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left)
                && let Some((send, cancel)) = self.direction_actions
            {
                if send.contains((mouse.column, mouse.row).into()) {
                    if self.details.as_ref().is_some_and(|details| details.editing) {
                        self.submit_agent_message();
                    } else {
                        self.submit();
                    }
                } else if cancel.contains((mouse.column, mouse.row).into()) {
                    self.key(event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
                }
            }
            return true;
        }
        if mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && (!self.selection_blocked() || self.native_focused())
            && let Some((_, code, modifiers)) = self
                .chrome_keys
                .iter()
                .find(|(area, _, _)| area.contains((mouse.column, mouse.row).into()))
                .copied()
        {
            if self.native_focused() {
                self.navigation
                    .update_presentation(|presentation| presentation.native_input = false);
                self.navigation.focus = NavigationFocus::WorkbenchContent;
                if let Some(details) = &mut self.details {
                    details.native_focus = false;
                }
            }
            self.key(event::KeyEvent::new(code, modifiers));
            return true;
        }
        if self.handle_workbench_mouse(mouse) {
            return true;
        }
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

    fn handle_workbench_mouse(&mut self, mouse: event::MouseEvent) -> bool {
        if self.navigation.screen != Screen::Workbench {
            return false;
        }
        if self.help.is_some()
            || self.picker.is_some()
            || self.editor.is_some()
            || self.interview_visible()
            || self.plan_history.is_some()
            || self.plan_view.is_some()
            || self.modal_blocks_mouse()
        {
            self.workbench_dragging = false;
            return false;
        }
        if self.workbench_dragging {
            match mouse.kind {
                MouseEventKind::Drag(MouseButton::Left) => {
                    let mut presentation = self.navigation.presentation();
                    if super::super::workbench::resize_at(
                        self.terminal_area,
                        &mut presentation,
                        mouse.row,
                    ) {
                        self.navigation.update_presentation(|current| {
                            current.terminal_percent = presentation.terminal_percent;
                        });
                    }
                    return true;
                }
                MouseEventKind::Up(MouseButton::Left) => {
                    self.workbench_dragging = false;
                    return true;
                }
                _ => return true,
            }
        }
        if mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && self
                .workbench_areas
                .separator
                .contains((mouse.column, mouse.row).into())
        {
            self.workbench_dragging = true;
            return true;
        }
        if mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && self
                .workbench_areas
                .terminal
                .contains((mouse.column, mouse.row).into())
        {
            self.navigation.focus = NavigationFocus::WorkbenchTerminal;
            self.navigation.update_presentation(|presentation| {
                presentation.focus = NavigationFocus::WorkbenchTerminal;
            });
            self.fetch_workbench_sessions();
            if let Some(details) = &mut self.details {
                details.native_focus = true;
            }
            return false;
        }
        if mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && self
                .workbench_areas
                .content
                .contains((mouse.column, mouse.row).into())
        {
            self.navigation.focus = NavigationFocus::WorkbenchContent;
            self.navigation.update_presentation(|presentation| {
                presentation.focus = NavigationFocus::WorkbenchContent;
            });
            if let Some(details) = &mut self.details {
                details.native_focus = false;
            }
            return false;
        }
        if mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && self.workbench_areas.list_visible
            && self
                .workbench_areas
                .list
                .contains((mouse.column, mouse.row).into())
        {
            let row = mouse.row.saturating_sub(self.workbench_areas.list.y) as usize
                + super::super::workbench::visible_start(
                    &self.view,
                    &self.navigation.target,
                    self.workbench_areas.list.height,
                );
            if let Some(row) = super::super::workbench::sidebar(&self.view).get(row) {
                self.open_work_card_or_stage(row);
            }
            return true;
        }
        false
    }

    fn handle_content_mouse(&mut self, mouse: event::MouseEvent) {
        if mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && let Some((_, action)) = self
                .inspector_hits
                .iter()
                .find(|(area, _)| area.contains((mouse.column, mouse.row).into()))
                .cloned()
        {
            self.activate_inspector(action);
            return;
        }
        if mouse.kind == MouseEventKind::Down(MouseButton::Left)
            && self
                .now_area
                .is_some_and(|area| area.contains((mouse.column, mouse.row).into()))
        {
            self.mouse_click(mouse.column, mouse.row);
            return;
        }
        self.update_selection_context();
        if !self.selection_blocked() {
            self.text_selection.mouse(mouse);
        }
        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                if self
                    .graph_card_area
                    .is_some_and(|area| area.contains((mouse.column, mouse.row).into())) =>
            {
                // The wheel over an expanded card scrolls what it shows.
                self.scroll_expanded_card(if mouse.kind == MouseEventKind::ScrollDown {
                    -3
                } else {
                    3
                });
            }
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
        self.update_selection_context();
        if self.modal_blocks_mouse() {
            self.click_graph_filters(mouse);
            self.click_unblock(mouse);
            if self
                .config_editor
                .as_ref()
                .is_none_or(|editor| !editor.visible)
                && self.unblock.is_none()
                && self.recovery_consent.is_none()
                && self.graph_filters.is_none()
                && self.flow_editor.is_none()
                && self.decision_inbox.is_none()
                && matches!(
                    mouse.kind,
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollDown
                )
            {
                self.mouse_scroll(mouse.kind == MouseEventKind::ScrollDown);
            }
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
            || self.unblock.is_some()
            || self.recovery_consent.is_some()
            || self.graph_filters.is_some()
            || self.interview_visible()
            || self.decision_inbox.is_some()
    }

    fn click_graph_filters(&mut self, mouse: event::MouseEvent) {
        if mouse.kind != MouseEventKind::Down(MouseButton::Left) {
            return;
        }
        let providers = graph::providers(&self.view);
        let Some(menu) = self.graph_filters.as_mut() else {
            return;
        };
        let previous_scope = self.graph_scope.clone();
        let outcome = menu.click(
            mouse.column,
            mouse.row,
            &providers,
            &mut self.relations,
            &mut self.graph_scope,
        );
        if self.graph_scope != previous_scope {
            self.graph_visibility_exception = None;
        }
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
