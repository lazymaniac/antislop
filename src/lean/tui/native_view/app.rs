use super::*;

impl App {
    pub(in crate::lean::tui) fn sync_native_views(&mut self) {
        let wanted = self.wanted_native_views();
        let retired = self.retain_native_views(&wanted);
        self.attach_native_views(wanted, retired);
    }

    fn wanted_native_views(&self) -> Vec<WantedView> {
        self.details
            .as_ref()
            .filter(|details| {
                details.terminal_visible()
                    && (details.selected_round.is_none() || details.resume_round)
            })
            .map_or_else(Vec::new, native_panels)
    }

    fn retain_native_views(&mut self, wanted: &[WantedView]) -> bool {
        let prefix = format!("{}:", self.conversation);
        let mut retired = false;
        self.native_views.retain(|key, view| {
            let keep = wanted.iter().any(|entry| {
                *key == format!("{prefix}{}", entry.role) && view.identity == entry.identity
            });
            retired |= !keep;
            keep
        });
        retired
    }

    fn attach_native_views(&mut self, wanted: Vec<WantedView>, retired: bool) {
        if let Some(details) = &mut self.details {
            details.native.clear();
            if retired {
                details.native_focus = false;
            }
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
        let details = self
            .details
            .as_ref()
            .filter(|d| d.terminal_visible() && (d.selected_round.is_none() || d.resume_round))?;
        details
            .session_keys()
            .get(details.active_panel)
            .map(|key| details.composer_recipient(key))
    }
    pub(in crate::lean::tui) fn native_focused(&self) -> bool {
        !self.ledger_view.as_ref().is_some_and(|v| v.visible)
            && self.details.as_ref().is_some_and(|d| d.native_focus)
            && self.native_key_role().is_some()
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
        if let Some(role) = self.native_key_role()
            && let Some(view) = self
                .native_views
                .get(&format!("{}:{role}", self.conversation))
        {
            let _ = view.send(Command::Retry);
        }
    }
}

struct WantedView {
    role: String,
    local_key: String,
    identity: String,
    columns: u16,
    rows: u16,
}

fn native_panels(details: &Details) -> Vec<WantedView> {
    details
        .session_keys()
        .iter()
        .enumerate()
        .filter_map(|(panel, key)| {
            let area = details.native_areas[panel];
            if area.is_empty() {
                return None;
            }
            let role = details.composer_recipient(key);
            let (role, identity) = native_identity(details, &role);
            Some(WantedView {
                role,
                local_key: key.clone(),
                identity,
                columns: area.width.max(1),
                rows: area.height.max(1),
            })
        })
        .collect()
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
