//! An agent CLI's approval request as a chat card, with choices sent back to the CLI.

use super::sanitize::sanitize_bounded;
use super::theme::{Theme, wrap};
use super::view::PendingApproval;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Decide later; the agent keeps waiting.
    Later,
    Choose(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalCard {
    pub approval: PendingApproval,
}

impl ApprovalCard {
    #[cfg(test)]
    pub fn new(approval: PendingApproval) -> Self {
        Self { approval }
    }

    pub fn key(&self, key: KeyEvent) -> Option<Outcome> {
        // Alt+digit is what terminals without extended key reporting (tmux,
        // many SSH sessions) send for Ctrl+Alt+digit; both choose.
        if key.modifiers != KeyModifiers::ALT
            && key.modifiers != (KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return None;
        }
        match key.code {
            KeyCode::Char('0') => Some(Outcome::Later),
            KeyCode::Char(c @ '1'..='9') => self
                .approval
                .options
                .get(c as usize - '1' as usize)
                .map(|option| Outcome::Choose(option.0.clone())),
            _ => None,
        }
    }

    pub(in crate::lean::tui) fn draw_with_theme(
        &self,
        frame: &mut Frame,
        area: Rect,
        theme: &Theme,
    ) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let inner_width = usize::from(area.width.saturating_sub(2)).max(1);
        if area.height == 1 {
            let summary = bounded_clean(&self.approval.summary, usize::from(area.width));
            let text = if summary.is_empty() {
                "! Approval".to_owned()
            } else {
                theme.fit(&format!("! {summary}"), usize::from(area.width))
            };
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(text, theme.warning())))
                    .style(theme.raised().add_modifier(Modifier::REVERSED)),
                area,
            );
            return;
        }
        let visible_rows = usize::from(area.height.saturating_sub(2));
        if visible_rows == 0 {
            return;
        }
        let header_rows = 1 + usize::from(self.approval.orchestrator);
        let footer_rows = usize::from(visible_rows > 1);
        let option_rows = self
            .approval
            .options
            .len()
            .min(visible_rows.saturating_sub(header_rows + footer_rows));
        let summary_rows = visible_rows.saturating_sub(header_rows + footer_rows + option_rows);
        let mut lines = vec![Line::from(Span::styled(
            theme.fit(
                &format!(
                    "{} asks permission {} {} CLI",
                    bounded_clean(&self.approval.role, inner_width),
                    theme.glyphs().dot,
                    bounded_clean(&self.approval.provider, inner_width)
                ),
                inner_width,
            ),
            Style::default().add_modifier(Modifier::BOLD),
        ))];
        if self.approval.orchestrator && visible_rows > 1 {
            lines.push(Line::from(Span::styled(
                "orchestrator decision may arrive later",
                theme.dim(),
            )));
        }
        for summary_line in bounded_summary(&self.approval.summary, inner_width, summary_rows) {
            lines.push(Line::raw(theme.fit(&summary_line, inner_width)));
        }
        for (index, (_, label, _)) in self.approval.options.iter().take(option_rows).enumerate() {
            let shortcut = index + 1;
            let binding = if shortcut <= 9 {
                format!("Alt+{shortcut}  ")
            } else {
                String::new()
            };
            let option = format!("{binding}{}", bounded_clean(label, inner_width));
            lines.push(Line::from(Span::styled(
                theme.fit(&option, inner_width),
                theme.selected(),
            )));
        }
        if footer_rows > 0 {
            lines.push(Line::from(Span::styled("Alt+0  decide later", theme.dim())));
        }
        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(theme.borders())
            .border_style(theme.separator(false))
            .title(" Approval ");
        frame.render_widget(
            Paragraph::new(lines).style(theme.raised()).block(block),
            area,
        );
    }
}

pub(super) fn card_summary_rows(text: &str, width: usize, max_lines: usize) -> usize {
    bounded_summary(text, width, max_lines).len()
}

pub(super) fn approval_panel_height(
    summary: &str,
    width: usize,
    options: usize,
    max_summary_lines: usize,
    chrome_rows: usize,
    available: usize,
) -> u16 {
    card_summary_rows(summary, width, max_summary_lines)
        .saturating_add(options)
        .saturating_add(chrome_rows)
        .min(available)
        .min(usize::from(u16::MAX)) as u16
}

fn bounded_summary(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    let width = width.max(1);
    let max_clusters = width.saturating_mul(max_lines);
    // Keep sanitization allocation tied to the card viewport. The byte cap
    // leaves room for multi-byte emoji clusters before the grapheme bound.
    let byte_limit = max_clusters.saturating_mul(64);
    let summary = sanitize_bounded(text, byte_limit);
    let bounded = summary
        .graphemes(true)
        .take(max_clusters)
        .collect::<String>();
    wrap(&bounded, width).into_iter().take(max_lines).collect()
}

fn bounded_clean(text: &str, width: usize) -> String {
    // These fields can come from an agent tool request. Sanitize only enough
    // bytes for the visible row before composing or truncating the display.
    sanitize_bounded(text, width.max(1).saturating_mul(64))
}

impl super::App {
    /// Tracks the first visible approval and advances when it is deferred or settled.
    pub(super) fn sync_approval(&mut self) {
        if let Some(card) = &self.approval
            && !self
                .view
                .approvals
                .iter()
                .any(|pending| pending.request == card.approval.request)
        {
            self.approval = None;
        }
    }

    pub(super) fn route_approval(&mut self, key: KeyEvent) -> Option<bool> {
        let card = self.approval.as_ref()?;
        let outcome = card.key(key)?;
        let request = card.approval.request.clone();
        match outcome {
            Outcome::Later => {
                self.approval_hidden.insert(request);
                self.approval = None;
            }
            Outcome::Choose(choice) => {
                self.approval_hidden.insert(request.clone());
                self.approval = None;
                self.send_command(None, |conversation| {
                    crate::lean::daemon::Request::Conversation(
                        crate::lean::daemon::ConversationRequest::AnswerApproval {
                            conversation,
                            operation_id: Some(crate::id()),
                            request,
                            choice,
                        },
                    )
                });
            }
        }
        Some(true)
    }

    pub(super) fn set_approval_mode(&mut self, mode: &str) {
        let mode = mode.to_owned();
        self.send_command(None, |conversation| {
            crate::lean::daemon::Request::Conversation(
                crate::lean::daemon::ConversationRequest::SetApprovalMode { conversation, mode },
            )
        });
    }

    /// Shows approvals again, including ones put off or left to the orchestrator.
    pub(super) fn show_pending_approvals(&mut self) {
        self.approval_hidden.clear();
        if self.view.approvals.is_empty() {
            self.notice = Some("No approval is waiting.".into());
        }
        self.open_decision_inbox();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn approval() -> PendingApproval {
        PendingApproval {
            request: "r1".into(),
            role: "worker:t1".into(),
            provider: "claude".into(),
            summary: "Bash command\ntouch /tmp/x".into(),
            orchestrator: false,
            options: vec![
                ("1".into(), "Yes".into(), "allow".into()),
                ("2".into(), "Yes, always".into(), "allow always".into()),
                ("3".into(), "No".into(), "deny".into()),
            ],
        }
    }

    fn press(card: &ApprovalCard, code: KeyCode, modifiers: KeyModifiers) -> Option<Outcome> {
        card.key(KeyEvent::new(code, modifiers))
    }

    #[test]
    fn only_reserved_choice_chords_select_and_defer() {
        let card = ApprovalCard::new(approval());
        let chord = KeyModifiers::CONTROL | KeyModifiers::ALT;
        assert_eq!(
            press(&card, KeyCode::Char('3'), chord),
            Some(Outcome::Choose("3".into()))
        );
        assert_eq!(press(&card, KeyCode::Char('2'), KeyModifiers::NONE), None);
        assert_eq!(press(&card, KeyCode::Enter, KeyModifiers::NONE), None);
        assert_eq!(
            press(&card, KeyCode::Char('0'), chord),
            Some(Outcome::Later)
        );
        assert_eq!(press(&card, KeyCode::Char('9'), chord), None);
        // Terminals without extended keys deliver the chord as Alt+digit.
        assert_eq!(
            press(&card, KeyCode::Char('1'), KeyModifiers::ALT),
            Some(Outcome::Choose("1".into()))
        );
        assert_eq!(
            press(&card, KeyCode::Char('1'), KeyModifiers::CONTROL),
            None
        );
    }

    #[test]
    fn approval_summary_sanitization_is_viewport_bounded_and_grapheme_safe() {
        let summary = format!("{}👩‍👩‍👧‍👦z", "x".repeat(10_000));
        let lines = bounded_summary(&summary, 4, 2);
        assert_eq!(lines.len(), 2);
        assert!(lines.iter().all(|line| line.graphemes(true).count() <= 4));

        let escaped = format!("{}\x1b[31mVisible", "a".repeat(10_000));
        let lines = bounded_summary(&escaped, 5, 1);
        assert_eq!(lines, ["aaaaa"]);
        assert_eq!(sanitize_bounded("👩‍👩‍👧‍👦z", 8), "");
        assert_eq!(sanitize_bounded("👩‍👩‍👧‍👦z", 64), "👩‍👩‍👧‍👦z");

        let metadata = format!("{}\x1b[31m👩‍👩‍👧‍👦tail", "x".repeat(10_000));
        let sanitized = bounded_clean(&metadata, 4);
        assert!(sanitized.len() <= 4 * 64);
        assert!(sanitized.starts_with("xxxx"));
        assert!(!sanitized.contains('\x1b'));
    }

    fn event(kind: &str, payload: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"seq":1,"kind":kind,"payload":payload})
    }

    fn requested(request: &str, decider: &str) -> serde_json::Value {
        event(
            "approval_requested",
            serde_json::json!({
                "request":request,"role":"worker:t1","provider":"claude","kind":"command",
                "summary":"touch /tmp/x","decider":decider,
                "options":[{"id":"1","label":"Yes","decision":"allow"},
                           {"id":"2","label":"No","decision":"deny"}],
            }),
        )
    }

    #[test]
    fn approvals_update_badges_without_stealing_focus_and_open_in_the_inbox() {
        let mut app = crate::lean::tui::tests::test_app();
        app.input = super::super::InputBuffer::with_text("draft 👩‍💻 é");
        let navigation = app.navigation.clone();
        for decider in ["user", "orchestrator"] {
            app.view.apply(&requested(decider, decider));
            app.sync_approval();
            assert!(app.approval.is_none());
            assert!(app.decision_inbox.is_none());
            assert_eq!(app.navigation, navigation);
        }
        assert!(app.header_status().contains("approval(s) waiting"));
        app.show_pending_approvals();
        assert!(app.decision_inbox.is_some());
        assert_eq!(app.input.text, "draft 👩‍💻 é");
        app.key_inner(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.decision_inbox.is_none());
        assert_eq!(app.input.text, "draft 👩‍💻 é");
    }

    #[test]
    fn escalated_approval_stays_in_the_inbox_until_explicitly_opened() {
        let mut app = crate::lean::tui::tests::test_app();
        app.view.apply(&requested("r1", "orchestrator"));
        app.view.apply(&event(
            "approval_escalated",
            serde_json::json!({"request":"r1"}),
        ));
        app.sync_approval();
        assert!(app.approval.is_none());
        assert!(app.decision_inbox.is_none());
        app.show_pending_approvals();
        assert!(app.decision_inbox.is_some());
        assert_eq!(app.view.approvals[0].summary, "touch /tmp/x");
        assert_eq!(app.view.approvals[0].options.len(), 2);
    }
}
