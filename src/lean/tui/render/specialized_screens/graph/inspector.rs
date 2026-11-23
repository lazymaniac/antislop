use crate::lean::tui::taskinfo;
use crate::lean::tui::theme::Theme;
use crate::lean::tui::view::{self, AUDITOR, ORCHESTRATOR, PLAN_REVIEWER, TaskRow, View};
use ratatui::{style::Style, text::Line};

#[cfg(test)]
thread_local! {
    static PLAN_PROBE_BYTES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

pub(super) fn role_content(
    view: &View,
    key: &str,
    task: Option<&TaskRow>,
    width: usize,
    height: usize,
    theme: &Theme,
) -> Vec<Line<'static>> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let mut preview = Preview {
        lines: Vec::new(),
        width,
        height,
        theme,
    };
    match key {
        ORCHESTRATOR | crate::lean::COMMENT_ORCHESTRATOR => preview.messages(view, key),
        crate::lean::PLANNER => preview.plan(view),
        PLAN_REVIEWER | AUDITOR => {
            preview.heading("Latest review");
            if key == AUDITOR
                && let Some(error) = &view.audit_error
            {
                preview.text(error, theme.error(), height);
            } else if let Some(review) = view.reviews.get(key).filter(|lines| !lines.is_empty()) {
                for line in review {
                    preview.text(line, theme.text(), height);
                    if preview.remaining() == 0 {
                        break;
                    }
                }
            } else {
                preview.text("No review submitted yet.", theme.dim(), height);
            }
        }
        _ => match task {
            Some(task) => preview.task(task, key.starts_with("reviewer:"), view),
            None => preview.messages(view, key),
        },
    }
    preview.lines
}

struct Preview<'a> {
    lines: Vec<Line<'static>>,
    width: usize,
    height: usize,
    theme: &'a Theme,
}

impl Preview<'_> {
    fn remaining(&self) -> usize {
        self.height.saturating_sub(self.lines.len())
    }

    fn heading(&mut self, text: &str) {
        self.text(text, self.theme.accent(), 1);
    }

    fn text(&mut self, text: &str, style: Style, limit: usize) {
        self.fragments([text], style, limit);
    }

    fn fragments<'a>(
        &mut self,
        fragments: impl IntoIterator<Item = &'a str>,
        style: Style,
        limit: usize,
    ) {
        let limit = limit.min(self.remaining());
        if limit == 0 || self.width == 0 {
            return;
        }
        let mut rows = crate::lean::tui::theme::wrapped_rows(
            fragments,
            self.width,
            crate::lean::tui::theme::Paragraphs::Lines,
        );
        let start = self.lines.len();
        self.lines.extend(
            rows.by_ref()
                .take(limit)
                .map(|line| Line::styled(line, style)),
        );
        if rows.has_more() && self.lines.len() > start {
            let last = self.lines.last_mut().unwrap();
            let marker = match self.width {
                1 => ".",
                2 => "..",
                _ => "...",
            };
            let text = view::truncate(&last.to_string(), self.width.saturating_sub(marker.len()));
            *last = Line::styled(format!("{text}{marker}"), style);
        }
    }

    fn messages(&mut self, view: &View, key: &str) {
        self.heading("Recent messages");
        if self.remaining() == 0 {
            return;
        }
        // Newest first: the latest exchange stays visible on short terminals.
        let messages: Vec<_> = view
            .messages
            .iter()
            .rev()
            .filter(|message| view.message_involves(message, key))
            .take(3)
            .collect();
        if messages.is_empty() {
            self.text("No messages yet.", self.theme.dim(), self.remaining());
            return;
        }
        for (index, message) in messages.iter().enumerate() {
            let remaining_messages = (messages.len() - index).min(self.remaining()).max(1);
            let rows = self.remaining() / remaining_messages;
            if rows == 0 {
                break;
            }
            self.fragments(
                [message.sender.as_str(), ": ", message.text.as_str()],
                self.theme.text(),
                rows.min(4),
            );
        }
    }

    fn plan(&mut self, view: &View) {
        let document = view.plan_preview.as_deref().unwrap_or(&view.plan_doc);
        self.heading(if view.plan_preview.is_some() {
            "Latest submitted plan"
        } else {
            "Current plan"
        });
        if self.remaining() == 0 {
            return;
        }
        if document.chars().any(|c| {
            #[cfg(test)]
            PLAN_PROBE_BYTES.with(|count| count.set(count.get() + c.len_utf8()));
            !c.is_whitespace()
        }) {
            self.text(document, self.theme.text(), self.remaining());
        } else if !view.tasks.is_empty() {
            for task in &view.tasks {
                self.fragments(
                    [task.id.as_str(), ": ", task.title.as_str()],
                    self.theme.text(),
                    2,
                );
                if self.remaining() == 0 {
                    break;
                }
            }
        } else {
            self.text("No plan submitted yet.", self.theme.dim(), self.remaining());
        }
    }

    fn task(&mut self, task: &TaskRow, reviewer: bool, view: &View) {
        if let Some(reason) = &task.reason {
            self.text(reason, self.theme.error(), 3);
        }
        if reviewer {
            self.heading("Review result");
            self.text(
                task.reviewer_summary
                    .as_deref()
                    .unwrap_or("No review submitted yet."),
                self.theme.text(),
                4,
            );
            self.text(
                &format!("Coverage: {}", taskinfo::coverage_full(task)),
                self.theme.dim(),
                3,
            );
        } else {
            self.heading("Worker result");
            if let Some(effect) = taskinfo::effect_front(task) {
                self.text(&effect, self.theme.text(), 4);
            }
            if let Some(summary) = &task.summary {
                self.text(summary, self.theme.text(), 3);
            }
            if task.integration.is_some() {
                self.text(&taskinfo::integration_line(task), self.theme.dim(), 2);
            }
        }
        if self.remaining() == 0 {
            return;
        }
        self.heading("Goal");
        self.text(
            &view::task_goal(&task.kind, &task.title, &task.goal, None),
            self.theme.text(),
            3,
        );
        if let Some(meta) = view.task_meta.get(&task.id) {
            for criterion in &meta.acceptance_criteria {
                self.fragments(["- ", criterion.text.as_str()], self.theme.dim(), 2);
                if self.remaining() == 0 {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_content_handles_missing_results_and_audit_failures_without_stale_reviews() {
        let theme = Theme::new(false, false);
        let mut view = View::default();
        for (key, expected) in [
            (ORCHESTRATOR, "No messages yet."),
            (crate::lean::PLANNER, "No plan submitted yet."),
            (PLAN_REVIEWER, "No review submitted yet."),
        ] {
            let lines = role_content(&view, key, None, 32, 10, &theme);
            assert!(lines.iter().any(|line| line.to_string() == expected));
        }
        view.plan_doc = "Published plan 👩‍💻 é".into();
        let lines = role_content(&view, crate::lean::PLANNER, None, 32, 10, &theme);
        assert_eq!(lines[0].to_string(), "Current plan");
        assert_eq!(lines[1].to_string(), view.plan_doc);

        view.reviews
            .insert(AUDITOR.into(), vec!["Stale approval".into()]);
        view.audit_error = Some("CLI failed: connection reset".into());
        let lines = role_content(&view, AUDITOR, None, 32, 10, &theme);
        assert_eq!(lines[1].to_string(), "CLI failed: connection reset");
        assert!(
            !lines
                .iter()
                .any(|line| line.to_string().contains("Stale approval"))
        );

        view.plan_doc = "👩‍💻é".repeat(100);
        for height in [0, 1, 2, 10] {
            let lines = role_content(&view, crate::lean::PLANNER, None, 10, height, &theme);
            assert!(lines.len() <= height);
            assert!(lines.iter().all(|line| line.width() <= 10));
            if height > 1 {
                assert!(lines[1].to_string().contains("👩‍💻é"));
            }
        }
    }
}

#[cfg(test)]
#[path = "../../../../../../tests/support/inspector_stream.rs"]
mod stream_tests;
