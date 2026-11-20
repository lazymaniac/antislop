use super::super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

mod graph_keys;

impl App {
    pub(in crate::lean::tui) fn key_inner(&mut self, key: KeyEvent) -> bool {
        if let Some(result) = self.route_native_or_history(key) {
            return result;
        }
        self.sync_interview();
        self.sync_approval();
        self.sync_rigor_recommendation();
        if self.help.is_some()
            && let Some(result) = self.route_overlays(key)
        {
            return result;
        }
        if self.decision_inbox.is_some()
            && let Some(result) = self.route_decision_inbox(key)
        {
            return result;
        }
        if self.picker.is_some() {
            if key.code == KeyCode::F(1) {
                self.show_help(Help::Picker);
                return true;
            }
            return self.route_screen(key).unwrap_or(true);
        }
        if let Some(result) = self.route_selection_and_config(key) {
            return result;
        }
        if let Some(result) = self.route_exit(key) {
            return result;
        }
        if let Some(result) = self.route_overlays(key) {
            return result;
        }
        if key.code == KeyCode::F(2) {
            self.open_decision_inbox();
            return true;
        }
        // The visible interview form owns input before navigation shortcuts and
        // zoom handling. Its own key handler keeps Ctrl+P/Ctrl+O exceptions.
        if let Some(result) = self.route_interview(key) {
            return result;
        }
        if self.details.as_ref().is_some_and(|details| details.editing)
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && key.code == KeyCode::Char('t')
        {
            return self.route_screen(key).unwrap_or(true);
        }
        if key.code == KeyCode::Esc
            && self
                .details
                .as_ref()
                .is_some_and(|details| details.editing || details.searching)
        {
            self.details_key(key);
            return true;
        }
        if key.code == KeyCode::Esc && (self.editor.is_some() || self.plan_view.is_some()) {
            return self.route_screen(key).unwrap_or(true);
        }
        if self.route_workbench_navigation(key)
            || self.route_navigation_escape(key)
            || self.route_frontend_shortcut(key)
        {
            return true;
        }
        if let Some(result) = self.route_screen(key) {
            return result;
        }
        self.route_base_layout(key)
    }

    fn route_workbench_navigation(&mut self, key: KeyEvent) -> bool {
        if self.navigation.screen != Screen::Workbench
            || self.graph_expanded.is_some()
            || self.interview_visible()
        {
            return false;
        }
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if alt
            && matches!(key.code, KeyCode::Left | KeyCode::Right)
            && self.navigation.presentation().tab == WorkbenchTab::Results
            && let Some(details) = self.details.as_mut()
            && details.visible_result_keys().len() > 1
        {
            details.change_result_document(if key.code == KeyCode::Right { 1 } else { -1 });
            self.schedule_output_document_fetch();
            return true;
        }
        if !control && !alt && key.code == KeyCode::Char('p') {
            let current = match &self.navigation.target {
                WorkbenchTarget::Agent(_) => Some(self.workbench_terminal_owner()),
                WorkbenchTarget::Session => self
                    .details
                    .as_ref()
                    .filter(|details| details.key != "conversation")
                    .map(|details| details.composer_recipient(&details.key)),
            };
            self.navigation.update_presentation(|presentation| {
                let pin =
                    current.filter(|role| presentation.terminal_owner.as_deref() != Some(role));
                presentation.terminal_owner = pin;
                presentation.pinned = presentation.terminal_owner.is_some();
                presentation.native_input = false;
            });
            self.fetch_workbench_sessions();
            return true;
        }
        if alt && matches!(key.code, KeyCode::Up | KeyCode::Down) && self.workbench_areas.split {
            self.navigation.update_presentation(|presentation| {
                let step = if key.code == KeyCode::Up {
                    -5_i16
                } else {
                    5_i16
                };
                presentation.terminal_percent =
                    (i16::from(presentation.terminal_percent) + step).clamp(10, 80) as u8;
            });
            return true;
        }
        if control && key.code == KeyCode::Char('b') {
            self.navigation.update_presentation(|presentation| {
                presentation.list_visible = !presentation.list_visible;
            });
            return true;
        }
        if control && matches!(key.code, KeyCode::Left | KeyCode::Right) {
            return self.workbench_change_tab(if key.code == KeyCode::Right { 1 } else { -1 });
        }
        if control && key.code == KeyCode::Char('w') {
            let focus = self.navigation.focus;
            let round = self
                .details
                .as_ref()
                .and_then(|details| details.selected_round);
            self.navigation.update_presentation(|presentation| {
                presentation.focus = focus;
                presentation.round = round;
            });
            let Some(details) = &mut self.details else {
                return false;
            };
            self.result_fetch.cancel_document_fetch();
            details.switch_role();
            self.navigation.target = WorkbenchTarget::Agent(details.key.clone());
            self.navigation
                .update_presentation(|presentation| presentation.native_input = false);
            let target = self.navigation.target.clone();
            if !self.navigation.presentations.contains_key(&target) {
                let delivery = details
                    .task_id()
                    .and_then(|id| self.view.tasks.iter().find(|task| task.id == id))
                    .is_some_and(|task| task.change_kind.has_delivery());
                let default = super::super::workbench::default_tab(&details.key, delivery);
                self.navigation
                    .update_presentation(|presentation| presentation.tab = default);
            }
            let presentation = self.navigation.presentation();
            details.selected_round = presentation.round;
            details.prepare_workbench_details(None, presentation.tab);
            let details_key = details.key.clone();
            self.navigation.focus = presentation.focus;
            details.native_focus = presentation.native_input;
            if presentation.tab == WorkbenchTab::Diff {
                self.open_diff(&details_key);
            } else {
                self.diff = None;
            }
            if presentation.tab == WorkbenchTab::Results {
                self.schedule_output_document_fetch();
            }
            return true;
        }
        if matches!(key.code, KeyCode::Char(',') | KeyCode::Char('.')) && !control && !alt {
            let direction = if key.code == KeyCode::Char('.') {
                1
            } else {
                -1
            };
            if let Some(details) = &mut self.details {
                self.result_fetch.cancel_document_fetch();
                details.change_round(&self.view, direction);
                self.navigation
                    .update_presentation(|presentation| presentation.native_input = false);
                self.navigation.update_presentation(|presentation| {
                    presentation.round = details.selected_round;
                });
                return true;
            }
        }
        if key.code == KeyCode::Tab || key.code == KeyCode::BackTab {
            let backwards = key.code == KeyCode::BackTab;
            self.navigation.focus = match (self.navigation.focus, backwards) {
                (NavigationFocus::WorkbenchContent, false) => NavigationFocus::WorkbenchTerminal,
                (NavigationFocus::WorkbenchTerminal, false) => NavigationFocus::WorkbenchContent,
                (NavigationFocus::WorkbenchTerminal, true) => NavigationFocus::WorkbenchContent,
                (_, true) => NavigationFocus::WorkbenchTerminal,
                _ => NavigationFocus::WorkbenchContent,
            };
            let focus = self.navigation.focus;
            self.navigation
                .update_presentation(|presentation| presentation.focus = focus);
            if focus == NavigationFocus::WorkbenchTerminal {
                self.fetch_workbench_sessions();
            }
            if let Some(details) = &mut self.details {
                details.native_focus = self.navigation.presentation().native_input;
            }
            return true;
        }
        if !control && !alt && key.code == KeyCode::Char('z') {
            let zoomed = !self.navigation.presentation().zoomed;
            self.navigation.zoom(zoomed);
            if zoomed && self.navigation.focus == NavigationFocus::WorkbenchTerminal {
                self.navigation.focus = NavigationFocus::WorkbenchTerminal;
            }
            return true;
        }
        false
    }

    fn workbench_change_tab(&mut self, direction: isize) -> bool {
        let Some(details) = &mut self.details else {
            return false;
        };
        let delivery = details
            .task_id()
            .and_then(|id| self.view.tasks.iter().find(|task| task.id == id))
            .is_some_and(|task| task.change_kind.has_delivery());
        let current = self.navigation.presentation().tab;
        let next =
            super::super::workbench::neighbour_tab(&details.key, delivery, current, direction);
        self.navigation
            .update_presentation(|presentation| presentation.tab = next);
        details.prepare_workbench_details(Some(current), next);
        let details_key = details.key.clone();
        if next == WorkbenchTab::Diff {
            self.open_diff(&details_key);
        } else {
            self.diff = None;
        }
        if next == WorkbenchTab::Results {
            self.schedule_output_document_fetch();
        }
        true
    }

    fn route_native_or_history(&mut self, key: KeyEvent) -> Option<bool> {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        if self.native_focused()
            && (matches!(key.code, KeyCode::F(5) | KeyCode::F(6))
                || (control && matches!(key.code, KeyCode::Char('p' | 'x'))))
        {
            self.navigation
                .update_presentation(|presentation| presentation.native_input = false);
            if let Some(details) = &mut self.details {
                details.native_focus = false;
            }
            return None;
        }
        if self.navigation.screen == Screen::Workbench
            && self.navigation.focus == NavigationFocus::WorkbenchTerminal
            && self.graph_expanded.is_none()
            && !self.native_input_blocked()
        {
            if !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                && matches!(key.code, KeyCode::Char('t' | 'i'))
            {
                let enabled = key.code == KeyCode::Char('i');
                self.navigation
                    .update_presentation(|presentation| presentation.native_input = enabled);
                if enabled {
                    self.fetch_workbench_sessions();
                }
                if let Some(details) = &mut self.details {
                    details.native_focus = enabled;
                }
                return Some(true);
            }
            if self.native_focused() {
                self.route_native_key(key);
                return Some(true);
            }
        }
        if self.native_focused() {
            if self.embedded_diff_graph_active()
                && embedded_diff_graph_key(key, self.graph_tab == 3)
            {
                return None;
            }
            self.route_native_key(key);
            return Some(true);
        }
        let browser = self.plan_history.as_mut()?;
        let was_loading = browser.loading;
        if !browser.key(key) {
            self.plan_history = None;
        } else if !was_loading && browser.loading {
            let offset = browser.offset;
            self.load_plan_history(offset);
        }
        Some(true)
    }

    fn route_native_key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char(']' | '5'))
        {
            if self.navigation.screen == Screen::Workbench {
                self.navigation
                    .update_presentation(|presentation| presentation.native_input = false);
            }
            if let Some(details) = &mut self.details {
                details.native_focus = false;
            }
            if self.focus == Focus::Graph && self.graph_tab == 1 {
                self.graph_tab = 0;
            }
        } else {
            self.native_send(crate::lean::terminal::Input::Key { event: key });
        }
    }

    fn route_selection_and_config(&mut self, key: KeyEvent) -> Option<bool> {
        self.text_selection
            .invalidate_context(&self.selection_context());
        if key.code == KeyCode::Char('y')
            && key.modifiers.contains(KeyModifiers::CONTROL)
            && !self.selection_blocked()
        {
            self.pending_clipboard = self.text_selection.text();
            return Some(true);
        }
        self.config_key(key).then_some(true)
    }

    fn route_exit(&mut self, key: KeyEvent) -> Option<bool> {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if self.picker.is_none() {
            self.found = None;
        }
        if key.code == KeyCode::Char('c') && control {
            return Some(false);
        }
        let editing = self.is_editing();
        (key.code == KeyCode::Char('q') && !control && !alt && !editing && self.plan_view.is_none())
            .then_some(false)
    }

    fn is_editing(&self) -> bool {
        self.interview_visible()
            || self.graph_filters.is_some()
            || self.editor.is_some()
            || self.flow_editor.is_some()
            || self.graph_link_source.is_some()
            || self.picker.is_some()
            || self
                .details
                .as_ref()
                .is_some_and(|details| details.searching)
            || (self.focus == Focus::Chat
                && self.help.is_none()
                && self.plan_view.is_none()
                && (self.details.is_none() || self.graph_expanded.is_some())
                && self.diff.is_none()
                && !self.input.is_empty())
    }

    fn route_overlays(&mut self, key: KeyEvent) -> Option<bool> {
        if self.help.is_some() && self.help_key(key) {
            return Some(true);
        }
        if key.code == KeyCode::F(1) {
            self.show_help(self.help_here());
            return Some(true);
        }
        if let Some(handled) = self.route_approval(key) {
            return Some(handled);
        }
        if let Some(handled) = self.route_flow_editor(key) {
            return Some(handled);
        }
        if let Some(handled) = self.route_graph_filters(key) {
            return Some(handled);
        }
        self.route_unblock(key)
    }

    fn route_flow_editor(&mut self, key: KeyEvent) -> Option<bool> {
        let editor = self.flow_editor.as_mut()?;
        let task_id = editor.id_input.text.clone();
        let task_title = editor.title_input.text.clone();
        match editor.key(key, &self.view) {
            flow_editor::FlowEditorOutcome::None => {}
            flow_editor::FlowEditorOutcome::Close => self.flow_editor = None,
            flow_editor::FlowEditorOutcome::Submit {
                revision,
                request_ai_review,
            } => {
                self.send_flow_revision(revision, request_ai_review, task_id, task_title);
            }
        }
        Some(true)
    }

    fn route_graph_filters(&mut self, key: KeyEvent) -> Option<bool> {
        let menu = self.graph_filters.as_mut()?;
        let providers = graph::providers(&self.view);
        let previous_scope = self.graph_scope.clone();
        let outcome = menu.key(key, &providers, &mut self.relations, &mut self.graph_scope);
        if self.graph_scope != previous_scope {
            self.graph_visibility_exception = None;
        }
        if outcome == graph_filters::Outcome::Close {
            self.graph_filters = None;
        }
        Some(true)
    }

    fn route_unblock(&mut self, key: KeyEvent) -> Option<bool> {
        let modal = self.unblock.as_mut()?;
        match modal.key(key) {
            unblock::Outcome::None => {}
            unblock::Outcome::Close => self.unblock = None,
            unblock::Outcome::Choose(choice) => {
                if let Some(state) = self.unblock.take() {
                    self.unblock_choice(state, choice);
                }
            }
        }
        Some(true)
    }

    fn route_screen(&mut self, key: KeyEvent) -> Option<bool> {
        if self.picker.is_some() {
            return Some(self.picker_key(key));
        }
        if self.editor.is_some() {
            self.editor_key(key);
            return Some(true);
        }
        if let Some(result) = self.route_interview(key) {
            return Some(result);
        }
        if let Some(result) = self.route_plan_view(key) {
            return Some(result);
        }
        if self.embedded_diff_graph_active() {
            if embedded_diff_graph_key(key, self.graph_tab == 3) {
                return Some(self.graph_key(key));
            }
            self.diff_key(key);
            return Some(true);
        }
        if self.diff.is_some() {
            self.diff_key(key);
            return Some(true);
        }
        if self.details.is_some() && self.graph_expanded.is_none() {
            self.details_key(key);
            return Some(true);
        }
        None
    }

    fn embedded_diff_graph_active(&self) -> bool {
        self.diff.is_some() && self.graph_expanded.is_some()
    }

    fn route_interview(&mut self, key: KeyEvent) -> Option<bool> {
        if !self.interview_visible() {
            return None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('o') {
            self.open_conversation_picker();
        } else if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('p') {
            self.picker = Some(Picker::new(Kind::Commands));
        } else if let Some(answer) = self
            .interview_forms
            .get_mut(&self.conversation)
            .and_then(|form| form.key(key))
        {
            self.send_interview_answer(answer);
        }
        Some(true)
    }

    fn route_plan_view(&mut self, key: KeyEvent) -> Option<bool> {
        let plan = self.plan_view.as_mut()?;
        let action = plan.key(key, &self.view);
        self.handle_plan_action(action);
        Some(true)
    }

    fn route_base_layout(&mut self, key: KeyEvent) -> bool {
        if self.route_global_shortcut(key) {
            return true;
        }
        if self.focus == Focus::Graph {
            return self.route_graph(key);
        }
        self.chat_key(key)
    }

    fn route_navigation_escape(&mut self, key: KeyEvent) -> bool {
        if key.code != KeyCode::Esc {
            return false;
        }
        if self.navigation.presentation().zoomed {
            self.navigation.zoom(false);
            return true;
        }
        if self.navigation.screen == Screen::Workbench
            && self.details.is_none()
            && self.diff.is_none()
            && self.plan_view.is_none()
        {
            self.navigation.screen = Screen::Graph;
            self.set_focus(Focus::Graph);
            return true;
        }
        false
    }

    fn route_frontend_shortcut(&mut self, key: KeyEvent) -> bool {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        if control && key.code == KeyCode::Char('k') {
            if self.editor.is_some()
                || self.flow_editor.is_some()
                || self.graph_filters.is_some()
                || self.interview_visible()
                || self.details.as_ref().is_some_and(|details| details.editing)
                || self
                    .details
                    .as_ref()
                    .is_some_and(|details| details.searching)
            {
                return false;
            }
            self.navigation.screen = Screen::Workbench;
            self.navigation.target = WorkbenchTarget::Session;
            self.navigation.focus = NavigationFocus::Composer;
            self.focus = Focus::Chat;
            return true;
        }
        if control {
            match key.code {
                KeyCode::Char('p') => {
                    self.picker = Some(Picker::new(Kind::Commands));
                    return true;
                }
                KeyCode::Char('t') => {
                    self.navigation.screen = Screen::Workbench;
                    self.navigation.target = WorkbenchTarget::Session;
                    self.navigation.focus = NavigationFocus::WorkbenchContent;
                    self.open_details("conversation".into());
                    return true;
                }
                KeyCode::Char('f') => {
                    self.navigation.screen = Screen::Workbench;
                    self.navigation.target = WorkbenchTarget::Session;
                    self.picker = Some(Picker::new(Kind::History));
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    fn route_global_shortcut(&mut self, key: KeyEvent) -> bool {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        if control && (key.code == KeyCode::Char('m') || key.code == KeyCode::Enter) {
            self.run_workflow(Command::OpenRigorPicker);
            return true;
        }
        match key.code {
            KeyCode::Char('p') if control => self.picker = Some(Picker::new(Kind::Commands)),
            KeyCode::Char('f') if control => self.picker = Some(Picker::new(Kind::History)),
            KeyCode::Char('n') if control => self.new_conversation(),
            KeyCode::Char('o') if control => self.open_conversation_picker(),
            // Ctrl+I reaches here only where the terminal tells it from Tab.
            KeyCode::Char('t' | 'i') if control => self.open_details("conversation".into()),
            KeyCode::Char('l') if control => self.run_workflow(Command::Plan),
            KeyCode::F(3) => self.run_workflow(Command::InterviewQuestion),
            KeyCode::F(4) => self.run_workflow(Command::Plans),
            KeyCode::F(5) => self.run_workflow(Command::TaskActions),
            KeyCode::F(6) => self.run_workflow(Command::PauseOrContinue),
            KeyCode::F(7) => self.run_workflow(Command::Reject),
            KeyCode::F(8) => self.run_workflow(Command::Config),
            KeyCode::Char('g') if control => self.open_projects_picker(),
            KeyCode::Tab | KeyCode::BackTab => self.toggle_focus(),
            _ => return false,
        }
        true
    }
}

fn embedded_diff_graph_key(key: KeyEvent, allow_selection: bool) -> bool {
    matches!(
        key.code,
        KeyCode::Char('1'..='4' | ',' | '.') | KeyCode::Enter
    ) || (allow_selection && matches!(key.code, KeyCode::Left | KeyCode::Right))
}
