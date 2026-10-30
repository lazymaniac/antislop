use super::*;
use crate::lean::daemon::Request;
use crate::lean::ledger::{LedgerEntry, LedgerQuery};
use crate::lean::tui::InputBuffer;
impl LedgerView {
    pub fn new() -> Self {
        Self {
            visible: true,
            ..Self::default()
        }
    }
    pub fn request(&self, conversation: &str) -> Request {
        let query = LedgerQuery {
            conversation: conversation.into(),
            run_id: self
                .page
                .as_ref()
                .map(|p| p.run.id.clone())
                .unwrap_or_default(),
            status: if self.status.text.trim().is_empty() {
                None
            } else {
                Some(self.status.text.trim().into())
            },
            missing_coverage: self.missing_only,
            offset: self.page.as_ref().map_or(0, |p| p.offset),
            limit: Some(100),
            task: self.task.clone(),
            ..Default::default()
        };
        Request::ListRequirements { query }
    }
    pub fn filtered(&self) -> Vec<&LedgerEntry> {
        self.page
            .as_ref()
            .map(|p| {
                p.entries
                    .iter()
                    .filter(|e| {
                        self.query.text.to_lowercase().split_whitespace().all(|q| {
                            format!("{} {} {}", e.spec.id, e.spec.interpretation, e.status)
                                .to_lowercase()
                                .contains(q)
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn selected_entry(&self) -> Option<&LedgerEntry> {
        self.filtered().get(self.selected).copied()
    }
    pub fn form_mut(&mut self) -> Option<&mut Form> {
        self.editing
            .as_ref()
            .and_then(|key| self.drafts.get_mut(key))
    }
    pub fn paste(&mut self, text: &str) {
        let text = crate::lean::tui::sanitize_paste(text);
        if self.preview.is_some() {
            if self.rejecting {
                self.rejection.insert_str(&text);
            }
        } else if let Some(form) = self.form_mut() {
            let field = form.field;
            form.fields[field as usize].insert_str(&text);
        } else if self.searching {
            self.query.insert_str(&text.replace('\n', " "));
        } else if self.status_edit {
            self.status.insert_str(&text.replace('\n', " "));
        } else if self.task_filter_edit {
            self.task_filter.insert_str(&text.replace('\n', " "));
        }
    }
    pub(super) fn edit(&mut self) {
        if let Some(e) = self.selected_entry().cloned() {
            let key = (e.run_id, e.spec.id.clone());
            let revision = self.page.as_ref().unwrap().run.spec_revision;
            self.drafts
                .entry(key.clone())
                .or_insert_with(|| Form::new(e.spec, revision));
            self.editing = Some(key);
            self.preview = None;
        }
    }
    pub(super) fn preserve_filter(&mut self) {
        if let Some(page) = &self.page {
            self.run_filters.insert(
                page.run.id.clone(),
                RunFilter {
                    query: self.query.clone(),
                    status: self.status.clone(),
                    missing: self.missing_only,
                    task: self.task.clone(),
                    selected: self.selected,
                    tab: self.tab,
                    scroll: self.scroll,
                },
            );
        }
    }
    pub(super) fn restore_filter(&mut self) {
        let key = self.page.as_ref().map(|p| p.run.id.clone());
        let state = key
            .and_then(|key| self.run_filters.get(&key).cloned())
            .unwrap_or_else(|| RunFilter {
                query: InputBuffer::new(),
                status: InputBuffer::new(),
                missing: false,
                task: None,
                selected: 0,
                tab: 0,
                scroll: 0,
            });
        self.query = state.query;
        self.status = state.status;
        self.missing_only = state.missing;
        self.task = state.task;
        self.selected = state.selected;
        self.tab = state.tab;
        self.scroll = state.scroll;
        self.detail = None;
        self.source = None;
    }
    pub(super) fn invalidate_read(&mut self) {
        if !matches!(
            self.pending_kind.as_str(),
            "preview" | "applied" | "created"
        ) {
            self.request_id = self.request_id.wrapping_add(1);
            self.pending = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lean::ledger::{LedgerEntry, LedgerPage, Run};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn page() -> LedgerPage {
        LedgerPage {
            run: Run {
                id: "run".into(),
                conversation_id: "c".into(),
                goal: "goal".into(),
                spec_revision: 1,
                state: "active".into(),
                start_message: 0,
            },
            entries: ["R-1", "R-2"]
                .into_iter()
                .map(|id| LedgerEntry {
                    run_id: "run".into(),
                    version: 1,
                    spec_revision: 1,
                    original: "".into(),
                    spec: serde_json::from_value(
                        serde_json::json!({"id":id,"source":{"source_id":"s","start":0,"end":1},
                    "interpretation":format!("Intent {id}"),"criteria":[],"mandatory":true}),
                    )
                    .unwrap(),
                    status: "open".into(),
                })
                .collect(),
            total: 2,
            offset: 0,
            has_more: true,
        }
    }

    #[test]
    fn default_ledger_view_starts_visible_like_new() {
        assert!(LedgerView::default().visible);
        assert!(LedgerView::new().visible);
    }

    #[test]
    fn case_insensitive_search_filters_and_selection_clamps() {
        let mut view = LedgerView::new();
        view.page = Some(page());
        view.query.text = "intent R-2 open".into();
        assert_eq!(view.filtered().len(), 1);
        assert_eq!(view.selected_entry().unwrap().spec.id, "R-2");
        view.query.text = "no match".into();
        assert!(view.filtered().is_empty());
    }

    #[test]
    fn ledger_paging_respects_has_more_and_restores_previous_offset() {
        let mut view = LedgerView::new();
        view.page = Some(page());
        assert!(matches!(
            view.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), "c"),
            Action::Fetch
        ));
        assert_eq!(view.page.as_ref().unwrap().offset, 100);
        view.page.as_mut().unwrap().has_more = false;
        assert!(matches!(
            view.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE), "c"),
            Action::None
        ));
        assert!(matches!(
            view.key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE), "c"),
            Action::Fetch
        ));
        assert_eq!(view.page.as_ref().unwrap().offset, 0);
    }
}
