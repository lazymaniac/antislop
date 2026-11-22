use super::super::input_owner::{InputKind, InputOwner};
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
        if self.input_owner(InputKind::Key) == InputOwner::Help
            && let Some(result) = self.route_overlays(key)
        {
            return result;
        }
        if self.input_owner(InputKind::Key) == InputOwner::Decisions
            && let Some(result) = self.route_decision_inbox(key)
        {
            return result;
        }
        if self.input_owner(InputKind::Key) == InputOwner::Picker {
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
        if self.input_owner(InputKind::Key) == InputOwner::Interview {
            return self.route_interview(key).unwrap_or(true);
        }
        if self.details.as_ref().is_some_and(|details| details.editing) && self.editor.is_none() {
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
        if self.navigation.focus == NavigationFocus::Composer
            && self.editor.is_none()
            && self.plan_view.is_none()
        {
            if self.details.as_ref().is_some_and(|details| details.editing) {
                self.details_key(key);
            } else if key.code == KeyCode::Esc {
                if self.history_index.is_some() {
                    self.chat_escape();
                }
                self.navigation.focus = if self.navigation.screen == Screen::Graph {
                    NavigationFocus::Graph
                } else {
                    NavigationFocus::WorkbenchContent
                };
                self.navigation.background_focus = if self.navigation.screen == Screen::Graph {
                    Focus::Graph
                } else {
                    Focus::Chat
                };
            } else if key.modifiers.contains(KeyModifiers::CONTROL)
                && key.code == KeyCode::Char('r')
            {
                self.picker = Some(Picker::new(Kind::Recipients));
            } else if key.modifiers.contains(KeyModifiers::CONTROL)
                && key.code == KeyCode::Char('t')
            {
                use crate::lean::store::Delivery;
                self.navigation.session_delivery = match self.navigation.session_delivery {
                    Delivery::Queue => Delivery::Steer,
                    Delivery::Steer => Delivery::Interrupt,
                    Delivery::Interrupt => Delivery::Queue,
                };
            } else {
                return self.chat_key(key);
            }
            return true;
        }
        if self.route_workbench_navigation(key)
            || self.route_navigation_escape(key)
            || self.route_frontend_shortcut(key)
        {
            return true;
        }
        if self.editor.is_none()
            && self.plan_view.is_none()
            && !self
                .details
                .as_ref()
                .is_some_and(|details| details.editing || details.searching)
            && self.route_global_shortcut(key)
        {
            return true;
        }
        if let Some(result) = self.route_screen(key) {
            return result;
        }
        self.route_base_layout(key)
    }

    fn route_workbench_navigation(&mut self, key: KeyEvent) -> bool {
        let switching_role =
            key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('w');
        if self.navigation.screen != Screen::Workbench
            || self.graph_expanded.is_some()
            || self.interview_visible()
            || self.editor.is_some()
            || self.plan_view.is_some()
            || self
                .details
                .as_ref()
                .is_some_and(|details| details.searching || (details.editing && !switching_role))
        {
            return false;
        }
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if !control && !alt {
            match key.code {
                KeyCode::Char('d') => {
                    self.open_decision_inbox();
                    return true;
                }
                KeyCode::Char('h') => {
                    self.run_workflow(Command::Plans);
                    return true;
                }
                KeyCode::Char(' ') => {
                    self.run_workflow(Command::PauseOrContinue);
                    return true;
                }
                _ => {}
            }
        }
        if self.navigation.focus == NavigationFocus::WorkbenchList
            && matches!(key.code, KeyCode::Up | KeyCode::Down | KeyCode::Enter)
        {
            if key.code == KeyCode::Enter {
                self.navigation.focus = NavigationFocus::WorkbenchContent;
                return true;
            }
            let rows = super::super::workbench::sidebar(&self.view);
            let address = self
                .details
                .as_ref()
                .map_or("conversation", |details| details.key.as_str());
            let at = rows
                .iter()
                .position(|row| row.stage && row.key == address)
                .unwrap_or(0);
            let next = if key.code == KeyCode::Down {
                ((at + 1)..rows.len())
                    .find(|index| rows[*index].stage)
                    .unwrap_or(at)
            } else {
                (0..at)
                    .rev()
                    .find(|index| rows[*index].stage || *index == 0)
                    .unwrap_or(at)
            };
            self.open_work_card_or_stage(&rows[next]);
            self.navigation.focus = NavigationFocus::WorkbenchList;
            return true;
        }
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
            let compact = self.terminal_area.width < 120;
            self.navigation.update_presentation(|presentation| {
                if compact {
                    presentation.list_requested = !presentation.list_requested;
                } else {
                    presentation.list_visible = !presentation.list_visible;
                }
            });
            return true;
        }
        if control && matches!(key.code, KeyCode::Left | KeyCode::Right) {
            return self.workbench_change_tab(if key.code == KeyCode::Right { 1 } else { -1 });
        }
        if control && key.code == KeyCode::Char('w') {
            let work = crate::lean::tui::work_units::WorkUnits::new(&self.view);
            let Some(details) = self.details.as_ref() else {
                return false;
            };
            let Some(unit) = work.for_stage(&details.key) else {
                return false;
            };
            let at = unit
                .stages
                .iter()
                .position(|stage| stage.id.0 == details.key)
                .unwrap_or(0);
            let next = unit.stages[(at + 1) % unit.stages.len()].id.0.clone();
            self.open_details(next);
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
            let panes = if self.workbench_areas.list_visible {
                vec![
                    NavigationFocus::WorkbenchList,
                    NavigationFocus::WorkbenchContent,
                    NavigationFocus::WorkbenchTerminal,
                ]
            } else {
                vec![
                    NavigationFocus::WorkbenchContent,
                    NavigationFocus::WorkbenchTerminal,
                ]
            };
            let at = panes
                .iter()
                .position(|pane| *pane == self.navigation.focus)
                .unwrap_or(0);
            self.navigation.focus =
                panes[(at + if backwards { panes.len() - 1 } else { 1 }) % panes.len()];
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
        self.remember_workbench();
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
        if self.native_focused() {
            self.route_native_key(key);
            return Some(true);
        }
        if self.navigation.screen == Screen::Workbench
            && self.graph_expanded.is_none()
            && !self.native_input_blocked()
            && self.navigation.focus != NavigationFocus::Composer
            && !self
                .details
                .as_ref()
                .is_some_and(|details| details.editing || details.searching)
            && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
            && key.code == KeyCode::Char('t')
        {
            self.navigation.focus = NavigationFocus::WorkbenchTerminal;
            self.navigation.update_presentation(|presentation| {
                presentation.focus = NavigationFocus::WorkbenchTerminal;
                presentation.native_input = true;
            });
            self.fetch_workbench_sessions();
            if let Some(details) = &mut self.details {
                details.native_focus = true;
            }
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
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char(']') {
            if self.navigation.screen == Screen::Workbench {
                self.navigation
                    .update_presentation(|presentation| presentation.native_input = false);
            }
            if let Some(details) = &mut self.details {
                details.native_focus = false;
            }
            if self.navigation.background_focus == Focus::Graph && self.graph_tab == 1 {
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
            || (self.navigation.background_focus == Focus::Chat
                && self.help.is_none()
                && self.plan_view.is_none()
                && (self.details.is_none() || self.graph_expanded.is_some())
                && self.diff.is_none()
                && !self.input.is_empty())
    }

    fn route_overlays(&mut self, key: KeyEvent) -> Option<bool> {
        if self.input_owner(InputKind::Key) == InputOwner::Help && self.help_key(key) {
            return Some(true);
        }
        if key.code == KeyCode::F(1) {
            self.show_help(self.help_here());
            return Some(true);
        }
        if let Some(handled) = self.route_approval(key) {
            return Some(handled);
        }
        match self.input_owner(InputKind::Key) {
            InputOwner::Flow => self.route_flow_editor(key),
            InputOwner::Filters => self.route_graph_filters(key),
            InputOwner::Consent | InputOwner::Recovery => self.route_unblock(key),
            _ => None,
        }
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
        if let Some(modal) = self.recovery_consent.as_mut() {
            match modal.key(key) {
                consent::Outcome::None => {}
                consent::Outcome::Close => self.recovery_consent = None,
                consent::Outcome::Confirm => {
                    let consent = self.recovery_consent.take().expect("consent modal exists");
                    self.submit_recovery_consent(consent);
                }
            }
            return Some(true);
        }
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

    fn submit_recovery_consent(&self, consent: consent::RecoveryConsent) {
        let client = self.client.clone();
        let updates = self.updates.clone();
        let conversation = self.conversation.clone();
        std::thread::spawn(move || {
            let request = Request::Conversation(crate::lean::daemon::ConversationRequest::Recover(
                crate::lean::TaskRecoveryRequest {
                    conversation: conversation.clone(),
                    plan: consent.plan.clone(),
                    task: consent.task.clone(),
                    action: crate::lean::RecoveryAction::IntegrateWithConsent,
                    consent_files: Some(consent.selected_files()),
                    freshness_token: consent.token,
                    message: None,
                },
            ));
            let notice = match client.call(&request) {
                Ok(reply) => match reply["status"].as_str() {
                    Some(status) => format!(
                        "{}: {status} — {}",
                        consent.task,
                        reply["notice"].as_str().unwrap_or("recovery completed")
                    ),
                    None => format!("✗ recovery response has no final status: {reply}"),
                },
                Err(error) => format!("✗ {error}"),
            };
            let _ = updates.send(Update::Notice(notice));
        });
    }

    fn route_screen(&mut self, key: KeyEvent) -> Option<bool> {
        match self.input_owner(InputKind::Key) {
            InputOwner::Picker => return Some(self.picker_key(key)),
            InputOwner::Editor => {
                self.editor_key(key);
                return Some(true);
            }
            InputOwner::Interview => return self.route_interview(key),
            InputOwner::Plan => return self.route_plan_view(key),
            _ => {}
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
        if matches!(key.code, KeyCode::Tab | KeyCode::BackTab) {
            if self.navigation.background_focus == Focus::Graph {
                self.cycle_work_stage(key.code == KeyCode::BackTab);
            } else {
                self.toggle_focus();
            }
            return true;
        }
        if self.navigation.background_focus == Focus::Graph {
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
            if self.plan_view.is_some()
                || self.editor.is_some()
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
            if matches!(self.navigation.target, WorkbenchTarget::Agent(_))
                && self.navigation.screen == Screen::Workbench
                && let Some(details) = &mut self.details
            {
                if details.selected_round.is_some() {
                    self.notice = Some("Select the live round to send a message.".into());
                    return true;
                }
                details.editing = true;
            }
            self.navigation.focus = NavigationFocus::Composer;
            self.navigation.background_focus = Focus::Chat;
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
            KeyCode::F(5)
                if self.navigation.screen == Screen::Workbench
                    && self.graph_expanded.is_none()
                    && self.details.is_some() =>
            {
                self.details_key(key);
            }
            KeyCode::F(5) => self.run_workflow(Command::TaskActions),
            KeyCode::F(6) => self.run_workflow(Command::PauseOrContinue),
            KeyCode::F(7) => self.run_workflow(Command::Reject),
            KeyCode::F(8) => self.run_workflow(Command::Config),
            KeyCode::Char('g') if control => self.open_projects_picker(),
            KeyCode::Char('x') if control => self.run_workflow(Command::Cancel),
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

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use crate::lean::daemon::{Client, socket};
    use serde_json::{Value, json};
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixListener;

    #[test]
    fn recovery_handler_loads_snapshot_and_submits_only_confirmed_files() {
        let directory = tempfile::tempdir().unwrap();
        let socket = socket(directory.path());
        let listener = UnixListener::bind(socket).unwrap();
        let (requests_tx, requests_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            for response in [
                json!({"freshness_token":"pre-review-token"}),
                json!({"status":"needs_user","notice":"manual snapshot review ready"}),
                json!({
                    "status":"needs_user", "result":"snapshot-42", "round":3,
                    "verdict":{"approved":true,"result":"snapshot-42","round":3},
                    "base":"base-17", "freshness_token":"fresh-token-9",
                    "spec":{"delivery":"code"},
                    "feedback":{"manual_snapshot":{"result":"snapshot-42","round":3}}
                }),
                json!({"changed_files":["src/keep.rs","src/skip.rs"]}),
                json!({"status":"needs_user","notice":"one selected file integrated"}),
            ] {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let request: Value = serde_json::from_str(&line).unwrap();
                requests_tx.send(request["request"].clone()).unwrap();
                let mut writer = stream;
                serde_json::to_writer(&mut writer, &json!({"data":response,"error":null})).unwrap();
                writer.write_all(b"\n").unwrap();
                writer.flush().unwrap();
            }
        });

        let client = Client::at(directory.path());
        let (mut app, updates) = App::test_app_with_client(client);
        app.view.apply(&json!({
            "seq":1,"kind":"plan_proposed",
            "payload":{"plan":"plan-7","tasks":[{"id":"task-4","title":"Manual fix","change_kind":"code","status":"failed"}]}
        }));
        app.view.apply(&json!({
            "seq":2,"kind":"task","payload":{"id":"task-4","status":"failed","reason":"worker exited"}
        }));
        app.open_unblock(Some("task-4".into()));
        assert!(
            app.unblock.as_ref().unwrap().choices.iter().any(|choice| {
                choice.action == crate::lean::RecoveryAction::ReviewManualSnapshot
            })
        );
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        for _ in 0..3 {
            app.route_unblock(key(KeyCode::Down));
        }
        app.route_unblock(key(KeyCode::Enter));
        let notice = updates
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        app.apply_update(notice);
        assert!(app.notice.as_deref().unwrap().contains("needs_user"));
        let task_request = requests_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert_eq!(task_request["op"], "task");
        let review_request = requests_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert_eq!(review_request["op"], "recover");
        assert_eq!(review_request["action"], "review_manual_snapshot");
        assert_eq!(review_request["freshness_token"], "pre-review-token");

        app.view.apply(&json!({
            "seq":3,"kind":"task","payload":{"id":"task-4","status":"needs_user"}
        }));
        app.open_unblock(Some("task-4".into()));
        assert!(
            app.unblock.as_ref().unwrap().choices.iter().any(|choice| {
                choice.action == crate::lean::RecoveryAction::IntegrateWithConsent
            })
        );
        app.route_unblock(key(KeyCode::Down));
        app.route_unblock(key(KeyCode::Enter));
        let conversation = app.conversation.clone();
        let generation = app.generation;
        let Update::RecoveryConsent { result, .. } = updates
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap()
        else {
            panic!("recovery handler must return the loaded consent snapshot");
        };
        app.apply_update(Update::RecoveryConsent {
            conversation,
            generation,
            plan: "plan-7".into(),
            task: "task-4".into(),
            result,
        });
        let consent = app.recovery_consent.as_ref().unwrap();
        assert_eq!(consent.result, "snapshot-42");
        assert_eq!(consent.token, "fresh-token-9");
        assert_eq!(consent.target, "project working tree");

        app.route_unblock(key(KeyCode::Down));
        app.route_unblock(key(KeyCode::Char(' '))); // select only src/skip.rs
        app.route_unblock(key(KeyCode::Enter));
        assert!(app.recovery_consent.as_ref().unwrap().confirming);
        app.route_unblock(key(KeyCode::Enter));

        let task_request = requests_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert_eq!(task_request["op"], "task");
        assert_eq!(task_request["id"], "task-4");
        assert_eq!(task_request["plan"], "plan-7");
        let diff_request = requests_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert_eq!(diff_request["op"], "diff");
        let recover_request = requests_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert_eq!(recover_request["op"], "recover");
        assert_eq!(recover_request["task"], "task-4");
        assert_eq!(recover_request["freshness_token"], "fresh-token-9");
        assert_eq!(recover_request["consent_files"], json!(["src/skip.rs"]));

        let notice = updates
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        app.apply_update(notice);
        assert!(app.notice.as_deref().unwrap().contains("needs_user"));
        assert!(
            app.notice
                .as_deref()
                .unwrap()
                .contains("one selected file integrated")
        );
        server.join().unwrap();
    }
}
