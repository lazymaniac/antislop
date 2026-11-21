//! Read-only historical plan records with an isolated projection and view state.
use super::*;
use crate::lean::store::PlansPage;

pub(super) struct Browser {
    pub offset: usize,
    pub total: usize,
    pub loading: bool,
    pub error: Option<String>,
    plan: Option<crate::lean::store::PlanHistory>,
    view: View,
    plan_view: PlanView,
}

impl Browser {
    pub fn new() -> Self {
        Self {
            offset: 0,
            total: 0,
            loading: true,
            error: None,
            plan: None,
            view: View::default(),
            plan_view: PlanView::new("history".into()),
        }
    }

    pub fn loaded(&mut self, page: PlansPage) {
        self.loading = false;
        self.error = None;
        self.total = page.total;
        self.plan = page.plans.into_iter().next();
        self.view = View::default();
        self.plan_view = PlanView::new(
            self.plan
                .as_ref()
                .map_or_else(|| "history".into(), |plan| plan.id.clone()),
        );
        if let Some(plan) = &self.plan {
            self.view.apply(&serde_json::json!({
                "seq":1, "kind":"plan_proposed", "payload":{
                    "plan":plan.id, "tasks":plan.tasks.iter().map(|task| &task.spec).collect::<Vec<_>>()
                }
            }));
            for (row, task) in self.view.tasks.iter_mut().zip(&plan.tasks) {
                row.uuid = Some(task.id.clone());
                row.status = task.status.clone();
                row.round = u64::from(task.round);
            }
        }
        self.sync_document();
    }

    pub fn key(&mut self, key: event::KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return false,
            KeyCode::Char('[') => self.page_older(),
            KeyCode::Char(']') => self.page_newer(),
            KeyCode::Char('v') if key.modifiers.is_empty() && !self.view.tasks.is_empty() => {
                self.plan_view.graph_focused = !self.plan_view.graph_focused;
                self.plan_view.graph_reveal = true;
            }
            code => self.navigate(code, key.modifiers),
        }
        true
    }

    fn page_older(&mut self) {
        if !self.loading && self.offset + 1 < self.total {
            self.offset += 1;
            self.loading = true;
        }
    }

    fn page_newer(&mut self) {
        if !self.loading && self.offset > 0 {
            self.offset -= 1;
            self.loading = true;
        }
    }

    fn navigate(&mut self, code: KeyCode, modifiers: KeyModifiers) {
        if self.plan_view.graph_focused {
            if modifiers.contains(KeyModifiers::SHIFT) {
                match code {
                    KeyCode::Left => {
                        self.plan_view.graph_offset.0 =
                            self.plan_view.graph_offset.0.saturating_sub(4)
                    }
                    KeyCode::Right => {
                        self.plan_view.graph_offset.0 =
                            self.plan_view.graph_offset.0.saturating_add(4)
                    }
                    KeyCode::Up => {
                        self.plan_view.graph_offset.1 =
                            self.plan_view.graph_offset.1.saturating_sub(2)
                    }
                    KeyCode::Down => {
                        self.plan_view.graph_offset.1 =
                            self.plan_view.graph_offset.1.saturating_add(2)
                    }
                    _ => {}
                }
                return;
            }
            match code {
                KeyCode::Up | KeyCode::Left => {
                    self.plan_view.graph_selected = self.plan_view.graph_selected.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Right => {
                    self.plan_view.graph_selected = (self.plan_view.graph_selected + 1)
                        .min(self.view.tasks.len().saturating_sub(1));
                }
                KeyCode::Home => self.plan_view.graph_selected = 0,
                KeyCode::End => {
                    self.plan_view.graph_selected = self.view.tasks.len().saturating_sub(1)
                }
                _ => return,
            }
            self.plan_view.graph_reveal = true;
            self.sync_document();
            return;
        }

        let last = self
            .plan_view
            .description_lines(&self.view)
            .len()
            .saturating_sub(1);
        match code {
            KeyCode::Up => self.plan_view.cursor = self.plan_view.cursor.saturating_sub(1),
            KeyCode::Down => self.plan_view.cursor = (self.plan_view.cursor + 1).min(last),
            KeyCode::PageUp => self.plan_view.cursor = self.plan_view.cursor.saturating_sub(10),
            KeyCode::PageDown => self.plan_view.cursor = (self.plan_view.cursor + 10).min(last),
            KeyCode::Home => self.plan_view.cursor = 0,
            KeyCode::End => self.plan_view.cursor = last,
            _ => {}
        }
    }

    #[cfg(test)]
    pub fn draw(&mut self, frame: &mut Frame) {
        self.draw_with_theme(frame, &Theme::from_env());
    }

    pub fn draw_with_theme(&mut self, frame: &mut Frame, theme: &Theme) {
        let separator = theme.chrome("·");
        let navigation = if self.loading {
            format!("loading {separator} {} total", self.total)
        } else if let Some(plan) = &self.plan {
            format!(
                "plan {} of {} {separator} {}",
                self.offset + 1,
                self.total,
                plan.status
            )
        } else {
            format!("no plans {separator} {} total", self.total)
        };
        if self.error.is_some() {
            self.view.plan_doc = self.report_text();
        }
        self.plan_view
            .draw_history(frame, frame.area(), &self.view, theme, &navigation);
    }

    fn sync_document(&mut self) {
        self.view.plan_doc = self.report_text();
        self.plan_view.cursor = 0;
        self.plan_view.scroll = 0;
    }

    fn report_text(&self) -> String {
        let body = if let Some(plan) = &self.plan {
            let mut text = format!(
                "# Plan {}\n\nOriginal plan document: not retained.\n\nThe graph is reconstructed from saved task specifications.\n",
                plan.id
            );
            if let Some(task) = plan.tasks.get(self.plan_view.graph_selected) {
                text.push_str(&format!(
                    "\n## Saved task result: {} ({})\n\nStatus: {}\nRound: {}\n\n## Report\n\n{}\n\n## Verdict\n\n{}\n\n## Base\n\n{}\n\n## Result\n\n{}",
                    task.spec["title"].as_str().unwrap_or("Untitled task"),
                    task.spec["id"].as_str().unwrap_or("unknown"),
                    task.status,
                    task.round,
                    task.report.as_deref().unwrap_or("No report was retained."),
                    task.verdict.as_ref().map(ToString::to_string).unwrap_or_else(|| "No verdict was retained.".into()),
                    task.base.as_deref().unwrap_or("No base revision was retained."),
                    task.result.as_deref().unwrap_or("No result revision was retained."),
                ));
            } else {
                text.push_str("\nNo task specifications are retained for this plan.");
            }
            text
        } else if self.loading {
            "# Plan history\n\nLoading saved plan records…".into()
        } else {
            "# Plan history\n\nNo plan records are retained for this conversation.".into()
        };
        if let Some(error) = &self.error {
            format!("# Plan history request failed\n\n{error}\n\n{body}")
        } else {
            body
        }
    }
}

impl App {
    pub(in crate::lean::tui) fn load_plan_history(&self, offset: usize) {
        let client = self.client.clone();
        let conversation = self.conversation.clone();
        let generation = self.generation;
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            let reply = client
                .call(&crate::lean::daemon::Request::Conversation(
                    crate::lean::daemon::ConversationRequest::Plans {
                        conversation: conversation.clone(),
                        offset: Some(offset),
                        limit: Some(1),
                    },
                ))
                .and_then(|value| serde_json::from_value(value).map_err(Into::into))
                .map_err(|error| error.to_string());
            let _ = updates.send(Update::PlanHistory {
                conversation,
                generation,
                offset,
                reply,
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use serde_json::json;

    #[test]
    fn history_is_read_only_and_preserves_the_live_draft_and_graph() {
        let mut app = App::test_app();
        app.input.insert_str("draft 👩‍💻");
        app.view.apply(&json!({"seq":1,"kind":"plan_proposed","payload":{
            "plan":"live", "tasks":[{"change_kind":"none","id":"current","title":"Current task"}]
        }}));
        app.plan_history = Some(Browser::new());
        app.paste("must not reach the draft");
        app.key(event::KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        assert_eq!(app.input.text, "draft 👩‍💻");
        assert_eq!(app.view.plan.as_deref(), Some("live"));
        app.key(event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.plan_history.is_none());
        assert_eq!(app.input.text, "draft 👩‍💻");
    }

    #[test]
    fn historical_projection_keeps_dependencies_and_execution_results() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::lean::Store::open(&dir.path().join("state.db")).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        let tasks = store.add_plan(&conversation.id, &[
            json!({"change_kind":"none","id":"implementation","title":"Implement"}),
            json!({"change_kind":"none","id":"security","title":"Security","depends_on":["implementation"]}),
        ]).unwrap();
        store
            .set_plan_status(&tasks[0].plan_id, "proposed", "approved")
            .unwrap();
        let mut browser = Browser::new();
        browser.loaded(store.plans_page(&conversation.id, 0, 1).unwrap());
        assert_eq!(browser.view.tasks[1].depends_on, ["implementation"]);
        assert!(
            browser
                .view
                .tasks
                .iter()
                .all(|row| row.status == "approved")
        );
        assert_eq!(
            browser.view.tasks[0].uuid.as_deref(),
            Some(tasks[0].id.as_str())
        );
        assert!(
            browser
                .view
                .plan_doc
                .contains("Original plan document: not retained.")
        );
    }

    #[test]
    fn history_documents_show_retained_results_and_explicit_missing_values() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::lean::Store::open(&dir.path().join("state.db")).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        store
            .add_plan(
                &conversation.id,
                &[json!({"change_kind":"none","id":"task","title":"Task"})],
            )
            .unwrap();
        let mut browser = Browser::new();
        browser.loaded(store.plans_page(&conversation.id, 0, 1).unwrap());
        assert!(browser.view.plan_doc.contains("No report was retained."));
        assert!(browser.view.plan_doc.contains("No verdict was retained."));
        assert!(
            browser
                .view
                .plan_doc
                .contains("No result revision was retained.")
        );
    }

    #[test]
    fn history_uses_document_and_side_graph_and_scrolls_long_reports() {
        use ratatui::{Terminal, backend::TestBackend};
        let dir = tempfile::tempdir().unwrap();
        let store = crate::lean::Store::open(&dir.path().join("state.db")).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        store.add_plan(&conversation.id, &[
            json!({"change_kind":"none","id":"first","title":"First"}),
            json!({"change_kind":"none","id":"second","title":"Second","depends_on":["first"]}),
        ]).unwrap();
        let mut browser = Browser::new();
        browser.loaded(store.plans_page(&conversation.id, 0, 1).unwrap());
        browser.plan.as_mut().unwrap().tasks[0].report = Some(
            (0..80)
                .map(|n| format!("saved-line-{n}"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        browser.sync_document();
        assert!(browser.view.plan_doc.contains("saved-line-0"));
        assert!(
            browser
                .plan_view
                .description_lines(&browser.view)
                .iter()
                .any(|line| line.text.contains("saved-line-0"))
        );
        let mut term = Terminal::new(TestBackend::new(100, 24)).unwrap();
        term.draw(|frame| browser.draw(frame)).unwrap();
        assert!(browser.plan_view.doc_area.width > 0);
        assert!(browser.plan_view.graph_area.x > browser.plan_view.doc_area.x);
        let text = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("Plan History"), "{text}");
        assert!(text.contains("Original plan document"), "{text}");
        assert!(text.contains("Tasks"), "{text}");
        assert!(
            !text.contains("c comment"),
            "history must not advertise editing: {text}"
        );
        browser.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        browser.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        term.draw(|frame| browser.draw(frame)).unwrap();
        let scrolled = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(scrolled.contains("saved-line-"), "{scrolled}");
    }

    #[test]
    fn paging_obeys_loading_state_and_history_bounds() {
        let mut browser = Browser::new();
        browser.loading = false;
        browser.total = 2;
        assert!(browser.key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE)));
        assert_eq!(browser.offset, 1);
        assert!(browser.loading);
        browser.key(KeyEvent::new(KeyCode::Char('['), KeyModifiers::NONE));
        assert_eq!(browser.offset, 1);
        browser.loading = false;
        browser.key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
        assert_eq!(browser.offset, 0);
        browser.key(KeyEvent::new(KeyCode::Char(']'), KeyModifiers::NONE));
        assert_eq!(browser.offset, 0);
    }

    #[test]
    fn no_history_and_raw_errors_are_visible_without_fabricating_records() {
        use ratatui::{Terminal, backend::TestBackend};
        let mut browser = Browser::new();
        browser.loading = false;
        browser.total = 0;
        browser.sync_document();
        let mut term = Terminal::new(TestBackend::new(60, 10)).unwrap();
        term.draw(|frame| browser.draw(frame)).unwrap();
        let text = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("No plan records are retained"), "{text}");
        browser.error = Some("database unavailable".into());
        browser.sync_document();
        term.draw(|frame| browser.draw(frame)).unwrap();
        let text = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("database unavailable"), "{text}");

        let dir = tempfile::tempdir().unwrap();
        let store = crate::lean::Store::open(&dir.path().join("state.db")).unwrap();
        let conversation = store.create_conversation("project").unwrap();
        store
            .add_plan(
                &conversation.id,
                &[json!({"change_kind":"none","id":"saved","title":"Saved"})],
            )
            .unwrap();
        browser.loaded(store.plans_page(&conversation.id, 0, 1).unwrap());
        browser.error = Some("next page failed".into());
        let mut wide = Terminal::new(TestBackend::new(80, 20)).unwrap();
        wide.draw(|frame| browser.draw(frame)).unwrap();
        let text = wide
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains("next page failed"), "{text}");
        assert!(text.contains("Plan "), "{text}");
    }

    #[test]
    fn app_keeps_raw_history_errors_and_ignores_stale_page_replies() {
        let mut app = App::test_app();
        app.plan_history = Some(Browser::new());
        app.apply_update(Update::PlanHistory {
            conversation: app.conversation.clone(),
            generation: app.generation,
            offset: 0,
            reply: Err("history store unavailable".into()),
        });
        let browser = app.plan_history.as_ref().unwrap();
        assert!(!browser.loading);
        assert_eq!(browser.error.as_deref(), Some("history store unavailable"));

        let browser = app.plan_history.as_mut().unwrap();
        browser.offset = 1;
        browser.loading = true;
        app.apply_update(Update::PlanHistory {
            conversation: app.conversation.clone(),
            generation: app.generation,
            offset: 0,
            reply: Ok(PlansPage {
                plans: vec![],
                offset: 0,
                limit: 1,
                total: 2,
                has_more: true,
                active_plan_id: None,
            }),
        });
        let browser = app.plan_history.as_ref().unwrap();
        assert_eq!(browser.offset, 1);
        assert!(browser.loading);
        assert_eq!(browser.error.as_deref(), Some("history store unavailable"));
    }
}
