use super::*;

impl LedgerView {
    pub(super) fn edit_key(&mut self, key: KeyEvent, ctrl: bool, conversation: &str) -> Action {
        if key.code == KeyCode::Esc {
            self.editing = None;
            return Action::None;
        }
        if ctrl && key.code == KeyCode::Char('s') && !self.pending {
            return self.prepare_edit(conversation);
        }
        if ctrl && key.code == KeyCode::Char('r') {
            return Action::Fetch;
        }
        self.move_edit_cursor(key);
        Action::None
    }

    fn prepare_edit(&mut self, conversation: &str) -> Action {
        self.refresh_operation_id();
        let Some(page) = self.page.as_ref() else {
            return Action::None;
        };
        let Some(form) = self.editing.as_ref().and_then(|key| self.drafts.get(key)) else {
            return Action::None;
        };
        match form.spec() {
            Ok(spec) => Action::Prepare {
                change: LedgerChange {
                    conversation: conversation.into(),
                    run_id: page.run.id.clone(),
                    expected_spec_revision: form.revision,
                    operation_id: form.operation_id.clone(),
                    reason: form.fields[FormField::Reason as usize].text.clone(),
                    entries: vec![spec],
                },
                source: form.fields[FormField::Source as usize].text.clone(),
            },
            Err(error) => {
                self.error = Some(error.to_string());
                Action::None
            }
        }
    }

    fn refresh_operation_id(&mut self) {
        let Some((origin, snapshot)) = &self.prepared_draft else {
            return;
        };
        let changed = self.editing.as_ref() == Some(origin)
            && self.drafts.get(origin).is_some_and(|form| {
                form.fields != snapshot.fields || form.revision != snapshot.revision
            });
        if changed && let Some(form) = self.form_mut() {
            form.operation_id = crate::id();
        }
    }

    fn move_edit_cursor(&mut self, key: KeyEvent) {
        let width = self.form_area().width as usize;
        if let Some(form) = self.form_mut() {
            let field = form.field as usize;
            match key.code {
                KeyCode::Tab => form.field = form.field.next(false),
                KeyCode::BackTab => form.field = form.field.next(true),
                KeyCode::Up => form.fields[field].move_up(width),
                KeyCode::Down => form.fields[field].move_down(width),
                KeyCode::Enter => form.fields[field].insert_char('\n'),
                _ => form.fields[field].edit_key(key),
            }
        }
    }

    pub(super) fn pending_action(&self, key: KeyEvent) -> bool {
        self.pending
            && matches!(
                self.pending_kind.as_str(),
                "preview" | "applied" | "created"
            )
            || (key.code == KeyCode::Enter && self.pending && self.pending_kind == "page")
    }

    pub(super) fn pending_page_edit(&self, key: KeyEvent) -> bool {
        self.pending
            && self.pending_kind == "page"
            && matches!(key.code, KeyCode::Char('e' | 'n' | 'N'))
    }

    pub(super) fn edit_filter_key(&self, key: KeyEvent) -> bool {
        (self.searching || self.status_edit || self.task_filter_edit)
            && !matches!(key.code, KeyCode::Esc | KeyCode::Enter)
    }

    pub(super) fn finish_filter_key(&mut self, key: KeyEvent) -> Action {
        if !matches!(key.code, KeyCode::Esc | KeyCode::Enter) {
            self.edit_filter_text(key);
            self.selected = 0;
            return Action::None;
        }
        let fetch = self.status_edit || self.task_filter_edit;
        if self.task_filter_edit {
            self.commit_task_filter();
        }
        self.searching = false;
        self.status_edit = false;
        self.task_filter_edit = false;
        if fetch { Action::Fetch } else { Action::None }
    }

    fn edit_filter_text(&mut self, key: KeyEvent) {
        if self.searching {
            self.query.edit_key(key);
        } else if self.task_filter_edit {
            self.task_filter.edit_key(key);
        } else {
            self.status.edit_key(key);
        }
    }

    fn commit_task_filter(&mut self) {
        self.task = if self.task_filter.text.trim().is_empty() {
            None
        } else {
            Some(self.task_filter.text.trim().into())
        };
    }
}
