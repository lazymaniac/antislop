use super::super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

mod graph_keys;

impl App {
    pub(in crate::lean::tui) fn key_inner(&mut self, key: KeyEvent) -> bool {
        if let Some(result) = self.route_native_or_history(key) {
            return result;
        }
        self.sync_interview();
        if let Some(result) = self.route_ledger(key) {
            return result;
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
        if let Some(result) = self.route_screen(key) {
            return result;
        }
        if let Some(result) = self.route_confirmation(key) {
            return result;
        }
        self.route_base_layout(key)
    }

    fn route_native_or_history(&mut self, key: KeyEvent) -> Option<bool> {
        if self.native_focused() {
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
            if let Some(details) = &mut self.details {
                details.native_focus = false;
            }
        } else {
            self.native_send(crate::lean::terminal::Input::Key { event: key });
        }
    }

    fn route_ledger(&mut self, key: KeyEvent) -> Option<bool> {
        if !self.ledger_view.as_ref().is_some_and(|view| view.visible) {
            return None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('o') {
            self.open_conversation_picker();
            return Some(true);
        }
        if self.picker.is_some() {
            return Some(self.picker_key(key));
        }
        let action = self
            .ledger_view
            .as_mut()
            .unwrap()
            .key(key, &self.conversation);
        if !self.ledger_view.as_ref().unwrap().visible {
            self.return_to_main();
        }
        self.ledger_action(action);
        Some(true)
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
            || self.picker.is_some()
            || self
                .details
                .as_ref()
                .is_some_and(|details| details.searching)
            || (self.focus == Focus::Chat
                && self.help.is_none()
                && self.plan_view.is_none()
                && self.details.is_none()
                && self.diff.is_none()
                && !self.input.is_empty())
    }

    fn route_overlays(&mut self, key: KeyEvent) -> Option<bool> {
        if self.trust_prompt.is_some() {
            return Some(self.route_trust_prompt(key));
        }
        if self.help.is_some() && self.help_key(key) {
            return Some(true);
        }
        if key.code == KeyCode::F(1) {
            self.show_help(self.help_here());
            return Some(true);
        }
        if self.project_trust.is_some() {
            self.project_trust_key(key);
            return Some(true);
        }
        if let Some(handled) = self.route_graph_filters(key) {
            return Some(handled);
        }
        self.route_unblock(key)
    }

    fn route_graph_filters(&mut self, key: KeyEvent) -> Option<bool> {
        let menu = self.graph_filters.as_mut()?;
        let providers = graph::providers(&self.view);
        let outcome = menu.key(key, &providers, &mut self.relations, &mut self.graph_scope);
        if outcome == graph_filters::Outcome::Close {
            self.graph_filters = None;
        }
        Some(true)
    }

    fn route_trust_prompt(&mut self, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.accept_trust();
                true
            }
            KeyCode::Esc | KeyCode::Char('n' | 'N' | 'q' | 'Q') => false,
            _ => true,
        }
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
        if self.diff.is_some() {
            self.diff_key(key);
            return Some(true);
        }
        if self.details.is_some() {
            self.details_key(key);
            return Some(true);
        }
        None
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

    fn route_confirmation(&mut self, key: KeyEvent) -> Option<bool> {
        if !self
            .confirmation
            .as_ref()
            .is_some_and(|card| card.status == ConfirmationStatus::Pending)
        {
            return None;
        }
        match key.code {
            KeyCode::Enter | KeyCode::Char('y' | 'Y') => {
                self.confirm_action();
                Some(true)
            }
            KeyCode::Esc | KeyCode::Char('n' | 'N') => {
                self.reject_confirmation();
                Some(true)
            }
            _ => None,
        }
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

    fn route_global_shortcut(&mut self, key: KeyEvent) -> bool {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('p') if control => self.picker = Some(Picker::new(Kind::Commands)),
            KeyCode::Char('t') if control => self.run_workflow(Command::ToggleMode),
            KeyCode::Char('f') if control => self.picker = Some(Picker::new(Kind::History)),
            KeyCode::Char('n') if control => self.new_conversation(),
            KeyCode::Char('o') if control => self.open_conversation_picker(),
            KeyCode::Char('i') if control => self.open_details("conversation".into()),
            KeyCode::Char('l') if control => self.run_workflow(Command::Plan),
            KeyCode::F(2) => self.run_workflow(Command::Requirements),
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
