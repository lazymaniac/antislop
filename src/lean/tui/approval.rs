//! An agent CLI's approval dialog, put to the user: what the agent asks to do
//! and the choices the CLI offers. The user's choice goes back into the CLI.

use super::theme::{Theme, wrap};
use super::view::{PendingApproval, clean, truncate};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    None,
    /// Decide later; the agent keeps waiting.
    Later,
    Choose(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Modal {
    pub approval: PendingApproval,
    pub selected: usize,
}

impl Modal {
    pub fn new(approval: PendingApproval) -> Self {
        Self {
            approval,
            selected: 0,
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let last = self.approval.options.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc => Outcome::Later,
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                Outcome::None
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.selected = (self.selected + 1).min(last);
                Outcome::None
            }
            KeyCode::Char(c @ '1'..='9') => {
                let id = c.to_string();
                self.approval
                    .options
                    .iter()
                    .find(|option| option.0 == id)
                    .map_or(Outcome::None, |option| Outcome::Choose(option.0.clone()))
            }
            KeyCode::Enter => self
                .approval
                .options
                .get(self.selected)
                .map_or(Outcome::None, |option| Outcome::Choose(option.0.clone())),
            _ => Outcome::None,
        }
    }

    pub fn draw(&self, frame: &mut Frame) {
        let theme = Theme::from_env();
        let area = frame.area();
        let width = area.width.saturating_sub(4).clamp(12, 90);
        let text_width = usize::from(width.saturating_sub(4)).max(1);
        let mut lines = vec![Line::from(Span::styled(
            truncate(
                &format!(
                    "{} asks permission ({} CLI)",
                    clean(&self.approval.role),
                    clean(&self.approval.provider)
                ),
                text_width,
            ),
            Style::default().add_modifier(Modifier::BOLD),
        ))];
        if self.approval.orchestrator {
            lines.push(Line::from(Span::styled(
                "the orchestrator decides unless you answer first",
                theme.dim(),
            )));
        }
        lines.push(Line::raw(""));
        let summary = wrap(&self.approval.summary, text_width);
        let summary_room = usize::from(area.height).saturating_sub(self.approval.options.len() + 9);
        lines.extend(
            summary
                .iter()
                .take(summary_room.max(1))
                .map(|line| Line::raw(line.clone())),
        );
        lines.push(Line::raw(""));
        let pointer = theme.glyphs().pointer;
        for (index, (id, label, _)) in self.approval.options.iter().enumerate() {
            let picked = index == self.selected;
            let style = if picked {
                Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
            } else {
                Style::default()
            };
            let marker = if picked { pointer } else { " " };
            lines.push(Line::from(Span::styled(
                truncate(&format!("{marker} {id}. {}", clean(label)), text_width + 1),
                style,
            )));
        }
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(
            truncate(
                "↑/↓ choose · Enter or 1-9 answer · Esc decide later",
                text_width,
            ),
            theme.dim(),
        )));
        let height = (lines.len() as u16 + 2).min(area.height);
        let boxed = Rect::new(
            area.x + area.width.saturating_sub(width) / 2,
            area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, boxed);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(theme.warning())
            .title(" Approval ");
        let inner = block.inner(boxed);
        frame.render_widget(block, boxed);
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

impl super::App {
    /// Opens the first approval that waits for the user, unless the user put
    /// it off; closes the dialog when its request was settled elsewhere.
    pub(super) fn sync_approval(&mut self) {
        if let Some(modal) = &self.approval
            && !self
                .view
                .approvals
                .iter()
                .any(|pending| pending.request == modal.approval.request)
        {
            self.approval = None;
        }
        if self.approval.is_none()
            && let Some(next) = self.view.approvals.iter().find(|pending| {
                !pending.orchestrator && !self.approval_hidden.contains(&pending.request)
            })
        {
            self.approval = Some(Modal::new(next.clone()));
        }
    }

    pub(super) fn route_approval(&mut self, key: KeyEvent) -> Option<bool> {
        let modal = self.approval.as_mut()?;
        let request = modal.approval.request.clone();
        match modal.key(key) {
            Outcome::None => {}
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
        self.approval = self.view.approvals.first().cloned().map(Modal::new);
        if self.approval.is_none() {
            self.notice = Some("No approval is waiting.".into());
        }
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

    fn press(modal: &mut Modal, code: KeyCode) -> Outcome {
        modal.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    #[test]
    fn a_digit_or_enter_chooses_and_escape_decides_later() {
        let mut modal = Modal::new(approval());
        assert_eq!(
            press(&mut modal, KeyCode::Char('3')),
            Outcome::Choose("3".into())
        );
        assert_eq!(press(&mut modal, KeyCode::Char('9')), Outcome::None);
        press(&mut modal, KeyCode::Down);
        assert_eq!(
            press(&mut modal, KeyCode::Enter),
            Outcome::Choose("2".into())
        );
        assert_eq!(press(&mut modal, KeyCode::Esc), Outcome::Later);
    }

    fn event(kind: &str, payload: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"seq":1,"kind":kind,"payload":payload})
    }

    fn requested(decider: &str) -> serde_json::Value {
        event(
            "approval_requested",
            serde_json::json!({
                "request":"r1","role":"worker:t1","provider":"claude","kind":"command",
                "summary":"touch /tmp/x","decider":decider,
                "options":[{"id":"1","label":"Yes","decision":"allow"},
                           {"id":"2","label":"No","decision":"deny"}],
            }),
        )
    }

    #[test]
    fn a_request_opens_the_card_and_its_resolution_closes_it() {
        let mut app = crate::lean::tui::tests::test_app();
        app.view.apply(&requested("user"));
        app.sync_approval();
        assert_eq!(app.approval.as_ref().unwrap().approval.request, "r1");
        assert!(app.header_status().contains("1 approval(s) waiting"));
        // Esc puts it off; it stays open and the palette brings it back.
        assert_eq!(
            app.route_approval(KeyEvent::new(
                KeyCode::Esc,
                crossterm::event::KeyModifiers::NONE
            )),
            Some(true)
        );
        app.sync_approval();
        assert!(app.approval.is_none());
        app.show_pending_approvals();
        assert!(app.approval.is_some());
        app.view.apply(&event(
            "approval_resolved",
            serde_json::json!({"request":"r1","state":"answered","choice":"1"}),
        ));
        app.sync_approval();
        assert!(app.approval.is_none());
        assert!(!app.header_status().contains("approval(s)"));
    }

    #[test]
    fn the_user_is_not_interrupted_by_what_the_orchestrator_decides() {
        let mut app = crate::lean::tui::tests::test_app();
        app.view.apply(&event(
            "approval_mode_changed",
            serde_json::json!({"mode":"autonomous"}),
        ));
        app.view.apply(&requested("orchestrator"));
        app.sync_approval();
        assert!(app.approval.is_none());
        assert!(app.header_status().contains("approvals: auto"));
        app.view.apply(&event(
            "approval_escalated",
            serde_json::json!({"request":"r1"}),
        ));
        app.sync_approval();
        assert!(app.approval.is_some());
    }
}
