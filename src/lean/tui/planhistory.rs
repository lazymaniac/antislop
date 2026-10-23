//! Read-only historical graphs, independent of the live conversation projection.
use super::*;
use crate::lean::store::PlansPage;

pub(super) struct Browser {
    pub offset: usize,
    pub total: usize,
    pub loading: bool,
    pub error: Option<String>,
    plan: Option<crate::lean::store::PlanHistory>,
    view: View,
    selected: String,
    scroll: (u16, u16),
    report_scroll: u16,
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
            selected: ORCHESTRATOR.into(),
            scroll: (0, 0),
            report_scroll: 0,
        }
    }

    pub fn loaded(&mut self, page: PlansPage) {
        self.loading = false;
        self.error = None;
        self.total = page.total;
        self.plan = page.plans.into_iter().next();
        self.view = View::default();
        self.scroll = (0, 0);
        self.report_scroll = 0;
        self.selected = ORCHESTRATOR.into();
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
    }

    pub fn key(&mut self, key: event::KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return false,
            KeyCode::Char('[') if !self.loading && self.offset + 1 < self.total => {
                self.offset += 1;
                self.loading = true;
            }
            KeyCode::Char(']') if !self.loading && self.offset > 0 => {
                self.offset -= 1;
                self.loading = true;
            }
            KeyCode::PageDown => self.report_scroll = self.report_scroll.saturating_add(8),
            KeyCode::PageUp => self.report_scroll = self.report_scroll.saturating_sub(8),
            code => {
                let direction = match code {
                    KeyCode::Left => Some((-1, 0)),
                    KeyCode::Right => Some((1, 0)),
                    KeyCode::Up => Some((0, -1)),
                    KeyCode::Down => Some((0, 1)),
                    _ => None,
                };
                if let Some((dx, dy)) = direction {
                    let graph = Graph::new(&self.view);
                    if let Some(next) = graph.neighbour(&self.selected, dx, dy) {
                        self.selected = next;
                        self.report_scroll = 0;
                    }
                }
            }
        }
        true
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let borders = if Theme::from_env().ascii {
            ratatui::symbols::border::Set {
                top_left: "+",
                top_right: "+",
                bottom_left: "+",
                bottom_right: "+",
                vertical_left: "|",
                vertical_right: "|",
                horizontal_top: "-",
                horizontal_bottom: "-",
            }
        } else {
            ratatui::symbols::border::PLAIN
        };
        frame.render_widget(Clear, area);
        let title = if self.loading {
            "Plan history · loading".into()
        } else if let Some(plan) = &self.plan {
            format!(
                "Plan history {}/{} · {} · {}",
                self.offset + 1,
                self.total,
                plan.id,
                plan.status
            )
        } else {
            "Plan history · no plans".into()
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(borders)
            .title(title)
            .title_bottom("[ older · ] newer · arrows select · PgUp/PgDn report · Esc close");
        let inner = block.inner(area);
        frame.render_widget(block, area);
        if let Some(error) = &self.error {
            frame.render_widget(Paragraph::new(sanitize(error)), inner);
            return;
        }
        let regions =
            Layout::vertical([Constraint::Percentage(65), Constraint::Min(2)]).split(inner);
        let graph = Graph::new_with_viewport(
            &self.view,
            None,
            &Default::default(),
            GroupingMode::default(),
            Some((regions[0].width, regions[0].height)),
        );
        if let Some(node) = graph.node(&self.selected) {
            self.scroll.0 = node.x.saturating_sub(regions[0].width / 4);
            self.scroll.1 = node.y.saturating_sub(1);
        }
        let mut canvas = Canvas::new_window(
            self.scroll.0,
            self.scroll.1,
            regions[0].width,
            regions[0].height,
        );
        graph.render_with(
            &self.view,
            &mut canvas,
            Filter::All,
            &self.selected,
            &Theme::from_env(),
            0,
        );
        canvas.show(frame.buffer_mut(), regions[0], self.scroll.0, self.scroll.1);
        let task_id = self.selected.split_once(':').map(|(_, id)| id);
        let report = self.plan.as_ref().and_then(|plan| {
            plan.tasks
                .iter()
                .find(|task| task.spec["id"].as_str() == task_id)
        });
        let text = report
            .map(|task| {
                format!(
                    "{} · {} · round {}\n{}\nVerdict: {}\nBase: {}\nResult: {}",
                    task.spec["title"].as_str().unwrap_or(""),
                    task.status,
                    task.round,
                    task.report.as_deref().unwrap_or("No report"),
                    task.verdict
                        .as_ref()
                        .map(ToString::to_string)
                        .unwrap_or_default(),
                    task.base.as_deref().unwrap_or(""),
                    task.result.as_deref().unwrap_or("")
                )
            })
            .unwrap_or_else(|| {
                "Select a task to inspect its historical report, verdict and commits.".into()
            });
        frame.render_widget(
            Paragraph::new(sanitize(&text))
                .wrap(ratatui::widgets::Wrap { trim: false })
                .scroll((self.report_scroll, 0))
                .block(Block::default().borders(Borders::TOP).border_set(borders)),
            regions[1],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn history_is_read_only_and_preserves_the_live_draft_and_graph() {
        let mut app = App::test_app();
        app.input.insert_str("draft 👩‍💻");
        app.view
            .apply(&json!({"seq":1,"kind":"plan_proposed","payload":{
                "plan":"live", "tasks":[{"id":"current","title":"Current task"}]
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
        let tasks = store
            .add_plan(
                &conversation.id,
                &[
                    json!({"id":"implementation","title":"Implement"}),
                    json!({"id":"security","title":"Security","depends_on":["implementation"]}),
                ],
            )
            .unwrap();
        store
            .set_plan_status(&tasks[0].plan_id, "proposed", "approved")
            .unwrap();
        let page = store.plans_page(&conversation.id, 0, 1).unwrap();
        let mut browser = Browser::new();
        browser.loaded(page);
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
    }
}
