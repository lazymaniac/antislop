use super::*;

impl App {
    #[cfg(test)]
    pub(in crate::lean::tui) fn bind_native_input_for_test(&mut self) -> mpsc::Receiver<Command> {
        let details = self.details.as_mut().expect("Workbench details");
        if details.native_areas[0].is_empty() {
            details.native_areas[0] = Rect::new(1, 1, 50, 12);
        }
        let area = details.native_areas[0];
        let (role, identity) = self
            .workbench_native_binding(self.details.as_ref().unwrap())
            .expect("live terminal owner");
        let (commands, received) = mpsc::sync_channel(16);
        self.native_views.insert(
            format!("{}:{role}", self.conversation),
            View {
                state: Arc::new(std::sync::Mutex::new(Snapshot::default())),
                commands,
                stop: Arc::new(AtomicBool::new(false)),
                dimensions: (area.width, area.height),
                identity,
            },
        );
        received
    }

    pub(in crate::lean::tui) fn sync_native_views(&mut self) {
        let wanted = self.wanted_native_views();
        let (retired, reset_input) = self.retain_native_views(&wanted);
        self.attach_native_views(wanted, retired, reset_input);
    }

    fn wanted_native_views(&self) -> Vec<WantedView> {
        if self.navigation.screen == crate::lean::tui::Screen::Workbench {
            let Some(details) = self.details.as_ref() else {
                return Vec::new();
            };
            let area = details.native_areas[0];
            if area.is_empty() {
                return Vec::new();
            }
            let Some((role, identity)) = self.workbench_native_binding(details) else {
                return Vec::new();
            };
            return vec![WantedView {
                local_key: role.clone(),
                role,
                identity,
                columns: area.width.max(1),
                rows: area.height.max(1),
            }];
        }
        Vec::new()
    }

    fn retain_native_views(&mut self, wanted: &[WantedView]) -> (bool, bool) {
        let prefix = format!("{}:", self.conversation);
        let mut retired = false;
        let mut initial_identity_discovery = false;
        self.native_views.retain(|key, view| {
            let keep = wanted.iter().any(|entry| {
                *key == format!("{prefix}{}", entry.role) && view.identity == entry.identity
            });
            if !keep && view.identity.starts_with("waiting:") {
                initial_identity_discovery |= wanted.iter().any(|entry| {
                    *key == format!("{prefix}{}", entry.role) && view.identity != entry.identity
                });
            }
            retired |= !keep;
            keep
        });
        (retired, retired && !initial_identity_discovery)
    }

    fn attach_native_views(&mut self, wanted: Vec<WantedView>, retired: bool, reset_input: bool) {
        if let Some(details) = &mut self.details {
            details.native.clear();
            if retired {
                details.native_focus = false;
            }
        }
        if reset_input && self.navigation.screen == crate::lean::tui::Screen::Workbench {
            self.navigation
                .update_presentation(|presentation| presentation.native_input = false);
        }
        for entry in wanted {
            self.attach_native_view(entry);
        }
    }

    fn attach_native_view(&mut self, entry: WantedView) {
        let key = format!("{}:{}", self.conversation, entry.role);
        let controller = self.native_views.entry(key).or_insert_with(|| {
            View::start(
                self.client.clone(),
                self.conversation.clone(),
                entry.role,
                entry.columns,
                entry.rows,
                self.updates.clone(),
                entry.identity,
            )
        });
        controller.resize(entry.columns, entry.rows);
        let snapshot = controller.state.lock().expect("native view").clone();
        if let Some(details) = &mut self.details {
            details.native.insert(entry.local_key, snapshot);
        }
    }
    fn native_key_role(&self) -> Option<String> {
        if self.navigation.screen == crate::lean::tui::Screen::Workbench {
            let details = self.details.as_ref()?;
            return self.workbench_native_binding(details).map(|(role, _)| role);
        }
        None
    }
    pub(in crate::lean::tui) fn native_focused(&self) -> bool {
        if self.navigation.screen == crate::lean::tui::Screen::Workbench {
            if self.native_input_blocked() {
                return false;
            }
            if !self.navigation.presentation().native_input {
                return false;
            }
            let Some(details) = self.details.as_ref() else {
                return false;
            };
            let Some((role, identity)) = self.workbench_native_binding(details) else {
                return false;
            };
            return self
                .native_views
                .get(&format!("{}:{role}", self.conversation))
                .is_none_or(|view| view.identity == identity);
        }
        false
    }

    pub(in crate::lean::tui) fn native_input_blocked(&self) -> bool {
        self.navigation.focus == NavigationFocus::Composer
            || self.help.is_some()
            || self
                .config_editor
                .as_ref()
                .is_some_and(|editor| editor.visible)
            || self.unblock.is_some()
            || self.recovery_consent.is_some()
            || self.graph_filters.is_some()
            || self.decision_inbox.is_some()
            || self.picker.is_some()
            || self.editor.is_some()
            || self.flow_editor.is_some()
            || self.interview_visible()
            || self.plan_history.is_some()
            || self.plan_view.is_some()
            || self
                .details
                .as_ref()
                .is_some_and(|details| details.editing || details.searching)
    }
    pub(in crate::lean::tui) fn native_send(&mut self, input: Input) {
        if let Some(role) = self.native_key_role() {
            let result = self
                .native_views
                .get(&format!("{}:{role}", self.conversation))
                .ok_or_else(|| crate::Error::Invalid("native terminal is not ready".into()))
                .and_then(|v| v.send(Command::Input(input.clone())));
            if let Err(error) = result {
                self.recover_native_input(
                    &self.conversation.clone(),
                    &role,
                    input,
                    &error.to_string(),
                );
            }
        }
    }
    pub(in crate::lean::tui) fn recover_native_input(
        &mut self,
        conversation: &str,
        role: &str,
        input: Input,
        error: &str,
    ) {
        if self.navigation.screen == crate::lean::tui::Screen::Workbench {
            self.navigation
                .update_presentation(|presentation| presentation.native_input = false);
        }
        let text = recovered_input_text(input);
        let message = format!(
            "Native input could not be confirmed; text recovered in message draft. Check the terminal before resending: {error}"
        );
        self.save_native_draft(conversation, role, &text, &message);
        self.restore_native_composer(conversation, role, &message);
    }

    fn save_native_draft(&mut self, conversation: &str, role: &str, text: &str, message: &str) {
        let mut drafts = self.agent_drafts.borrow_mut();
        let draft = drafts
            .entry((conversation.into(), role.into()))
            .or_default();
        draft.input.insert_str(text);
        draft.error = Some(message.to_owned());
    }

    fn restore_native_composer(&mut self, conversation: &str, role: &str, message: &str) {
        if self.conversation != conversation {
            return;
        }
        self.notice = Some(message.to_owned());
        if let Some(details) = self.details.as_mut()
            && details
                .session_keys()
                .iter()
                .any(|key| details.composer_recipient(key) == role)
        {
            details.native_focus = false;
            details.editing = true;
        }
    }

    pub(in crate::lean::tui) fn native_retry(&mut self) {
        if self.navigation.screen != crate::lean::tui::Screen::Workbench {
            return;
        }
        self.fetch_workbench_sessions();
        if let Some(role) = self.native_key_role() {
            match self
                .native_views
                .get(&format!("{}:{role}", self.conversation))
            {
                Some(view) => {
                    if let Err(error) = view.send(Command::Retry) {
                        self.notice = Some(format!("Could not retry CLI attachment: {error}"));
                    }
                }
                None => self.notice = Some("CLI attachment is not ready to retry.".into()),
            }
        }
    }
}

impl App {
    pub(in crate::lean::tui) fn workbench_terminal_owner(&self) -> String {
        let presentation = self.navigation.presentation();
        if let Some(owner) = presentation.terminal_owner {
            return owner;
        }
        match &self.navigation.target {
            WorkbenchTarget::Session => "orchestrator".into(),
            WorkbenchTarget::Agent(role) => {
                let actual = role.clone();
                self.details.as_ref().map_or(actual.clone(), |details| {
                    details.composer_recipient(&actual)
                })
            }
        }
    }

    pub(in crate::lean::tui) fn workbench_terminal_view_role(&self) -> String {
        self.details
            .as_ref()
            .and_then(|details| self.workbench_native_binding(details))
            .map(|(role, _)| role)
            .unwrap_or_else(|| self.workbench_terminal_owner())
    }

    fn workbench_native_binding(&self, details: &Details) -> Option<(String, String)> {
        let owner = self.workbench_terminal_owner();
        if details_match_terminal_owner(details, &owner)
            && details.selected_round.is_some()
            && !details.resume_round
        {
            return None;
        }
        Some(self.workbench_native_identity(details, &owner))
    }

    fn workbench_native_identity(&self, details: &Details, owner: &str) -> (String, String) {
        if details_match_terminal_owner(details, owner)
            && let Some(round) = details.selected_round
        {
            return (format!("{owner}#r{round}"), format!("round:{round}"));
        }
        if details_match_terminal_owner(details, owner)
            && details.task.is_some()
            && details.fetched_events >= self.workbench_sessions_after
            && native_session_id(details, owner) != "waiting"
        {
            return native_identity(details, owner);
        }
        let run_index = self
            .workbench_sessions
            .iter()
            .filter(|session| session.role == owner)
            .count()
            .checked_sub(1);
        if let (Some(index), Some(session)) = (
            run_index,
            self.workbench_sessions
                .iter()
                .rfind(|session| session.role == owner),
        ) {
            let identity = if details_match_terminal_owner(details, owner)
                && crate::lean::task_role(owner).is_some()
                && details.task.is_some()
            {
                format!(
                    "{}:{}",
                    session.session_id.as_deref().unwrap_or("waiting"),
                    native_round(details, owner)
                )
            } else {
                format!(
                    "{}:run:{index}",
                    session.session_id.as_deref().unwrap_or("waiting")
                )
            };
            return (owner.to_owned(), identity);
        }
        if details_match_terminal_owner(details, owner)
            && details.fetched_events < self.workbench_sessions_after
        {
            (
                owner.to_owned(),
                format!("waiting:after:{}", self.workbench_sessions_after),
            )
        } else if details_match_terminal_owner(details, owner) {
            native_identity(details, owner)
        } else {
            (owner.to_owned(), "waiting:".into())
        }
    }
}

fn details_match_terminal_owner(details: &Details, owner: &str) -> bool {
    details.composer_recipient(&details.key) == owner
        || details.task_uuid.as_deref().is_some_and(|task| {
            owner
                .split_once(':')
                .is_some_and(|(_, owner_task)| owner_task == task)
        })
}

struct WantedView {
    role: String,
    local_key: String,
    identity: String,
    columns: u16,
    rows: u16,
}

fn native_identity(details: &Details, role: &str) -> (String, String) {
    if let Some(round) = details.selected_round {
        return (format!("{role}#r{round}"), format!("round:{round}"));
    }
    (
        role.to_owned(),
        format!(
            "{}:{}",
            native_session_id(details, role),
            native_round(details, role)
        ),
    )
}

fn native_session_id<'a>(details: &'a Details, role: &str) -> &'a str {
    let metadata = details
        .task
        .as_ref()
        .and_then(|task| {
            task["native_sessions"]
                .as_array()
                .or_else(|| task["turns"].as_array())
        })
        .and_then(|turns| turns.iter().rev().find(|turn| turn["role"] == role));
    let stored = details
        .sessions
        .iter()
        .rev()
        .find(|session| session.role == role)
        .filter(|_| details.task.is_none());
    metadata
        .and_then(|turn| turn["session_id"].as_str())
        .or_else(|| stored.and_then(|session| session.session_id.as_deref()))
        .unwrap_or("waiting")
}

fn native_round(details: &Details, role: &str) -> String {
    details
        .task
        .as_ref()
        .filter(|_| crate::lean::task_role(role).is_some())
        .map(|task| task["round"].to_string())
        .unwrap_or_default()
}

fn recovered_input_text(input: Input) -> String {
    match input {
        Input::Paste { text } => text,
        Input::Key { event }
            if !event
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            match event.code {
                KeyCode::Char(ch) => ch.to_string(),
                KeyCode::Enter => "\n".into(),
                _ => String::new(),
            }
        }
        _ => String::new(),
    }
}
