//! The way out for a step that stopped: what happened, and what the user can
//! do about it. The choices are the recovery actions the daemon offers.

use super::super::RecoveryAction;
use super::view::{TaskRow, clean, truncate};
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::Frame;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

/// Whether a task has stopped short of approval and waits for the user.
pub fn is_blocked(status: &str) -> bool {
    matches!(
        status,
        "failed" | "needs_user" | "waiting" | "cancelled" | "skipped"
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    pub action: RecoveryAction,
    pub label: &'static str,
    /// The user may write a hint or a reason first.
    pub asks_text: bool,
}

/// The choices that fit a stopped task: a writer's change can be kept or
/// applied anyway, every task can be retried, given more rounds, approved as
/// it is, or cancelled.
pub fn choices_for(task: &TaskRow) -> Vec<Choice> {
    let choice = |action, label, asks_text| Choice {
        action,
        label,
        asks_text,
    };
    if matches!(task.status.as_str(), "queued" | "working" | "verifying") {
        return vec![choice(
            RecoveryAction::RestartAgent,
            "Restart the agent in a fresh session",
            true,
        )];
    }
    let mut choices = vec![choice(
        RecoveryAction::RetryWithGuidance,
        "Retry with a hint for the worker",
        true,
    )];
    if task.status == "needs_user" || task.status == "failed" {
        choices.push(choice(
            RecoveryAction::ExtendRounds,
            "Give it more rounds",
            false,
        ));
    }
    if task.writes && task.status == "needs_user" {
        choices.push(choice(
            RecoveryAction::IntegrateWithConsent,
            "Apply its change anyway",
            false,
        ));
        choices.push(choice(
            RecoveryAction::KeepOnBranch,
            "Preserve its change while integration is pending",
            false,
        ));
        choices.push(choice(
            RecoveryAction::VerifyManualSnapshot,
            "I fixed it myself: verify my working tree",
            false,
        ));
    }
    choices.push(choice(
        RecoveryAction::RestartAgent,
        "Restart the agent in a fresh session",
        true,
    ));
    choices.push(choice(
        RecoveryAction::ForceApprove,
        "Approve it as it is (skips verification)",
        true,
    ));
    choices
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    None,
    Close,
    Choose(Choice),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unblock {
    pub plan: String,
    pub task: String,
    pub title: String,
    pub status: String,
    pub reason: String,
    pub choices: Vec<Choice>,
    pub selected: usize,
    /// Where each choice was drawn, for the mouse.
    pub rects: Vec<Rect>,
}

impl Unblock {
    pub fn orchestrator() -> Self {
        let mut modal = Self::new(
            "",
            &TaskRow {
                id: super::super::ORCHESTRATOR.into(),
                title: "Orchestrator".into(),
                status: "working".into(),
                reason: Some(
                    "Restart orchestration while preserving conversation history and task work."
                        .into(),
                ),
                ..TaskRow::default()
            },
        );
        modal.choices[0].label = "Restart the orchestrator in a fresh session";
        modal.choices[0].asks_text = false;
        modal
    }

    pub fn new(plan: &str, task: &TaskRow) -> Self {
        Self {
            plan: plan.to_owned(),
            task: task.id.clone(),
            title: task.title.clone(),
            status: task.status.clone(),
            reason: task
                .reason
                .clone()
                .unwrap_or_else(|| "no reason was recorded".into()),
            choices: choices_for(task),
            selected: 0,
            rects: Vec::new(),
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        let last = self.choices.len().saturating_sub(1);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => Outcome::Close,
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                Outcome::None
            }
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => {
                self.selected = (self.selected + 1).min(last);
                Outcome::None
            }
            KeyCode::Char(c @ '1'..='9') => {
                let index = (c as usize) - ('1' as usize);
                match self.choices.get(index) {
                    Some(choice) => Outcome::Choose(choice.clone()),
                    None => Outcome::None,
                }
            }
            KeyCode::Enter => self
                .choices
                .get(self.selected)
                .cloned()
                .map_or(Outcome::None, Outcome::Choose),
            _ => Outcome::None,
        }
    }

    pub fn click(&mut self, col: u16, row: u16) -> Outcome {
        let at = Position::new(col, row);
        match self.rects.iter().position(|rect| rect.contains(at)) {
            Some(index) => {
                self.selected = index;
                Outcome::Choose(self.choices[index].clone())
            }
            None => Outcome::None,
        }
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let width = area.width.saturating_sub(4).clamp(12, 78);
        let wanted = self.choices.len() as u16 + 8;
        let height = wanted.min(area.height);
        let boxed = Rect::new(
            area.x + area.width.saturating_sub(width) / 2,
            area.y + area.height.saturating_sub(height) / 2,
            width,
            height,
        );
        frame.render_widget(Clear, boxed);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(super::theme::ayu::YELLOW))
            .title(format!(
                " Task actions {} · {} ",
                clean(&self.task),
                self.status.replace('_', " ")
            ));
        let inner = block.inner(boxed);
        frame.render_widget(block, boxed);
        let text_width = usize::from(inner.width.saturating_sub(2)).max(1);
        let mut lines = vec![
            Line::from(Span::styled(
                truncate(&clean(&self.title), text_width),
                Style::default().add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                truncate(&clean(&self.reason).replace('\n', " "), text_width),
                Style::default().fg(super::theme::ayu::RED),
            )),
            Line::raw(""),
        ];
        let first_choice = lines.len() as u16;
        self.rects.clear();
        for (n, choice) in self.choices.iter().enumerate() {
            let y = inner.y + first_choice + n as u16;
            if y >= inner.bottom().saturating_sub(1) {
                self.rects.push(Rect::default());
                continue;
            }
            self.rects.push(Rect::new(inner.x, y, inner.width, 1));
            let picked = n == self.selected;
            let marker = if picked { "▸" } else { " " };
            let style = if picked {
                Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
            } else {
                Style::default()
            };
            lines.push(Line::from(Span::styled(
                truncate(
                    &format!("{marker} {}. {}", n + 1, choice.label),
                    text_width + 1,
                ),
                style,
            )));
        }
        if inner.height > lines.len() as u16 {
            let gap = inner.height - 1 - lines.len() as u16;
            lines.extend((0..gap).map(|_| Line::raw("")));
            lines.push(Line::from(Span::styled(
                truncate("↑/↓ choose · Enter or 1-9 do it · Esc back", text_width),
                Style::default().fg(super::theme::ayu::DIM),
            )));
        }
        frame.render_widget(Paragraph::new(lines), inner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn row(status: &str, writes: bool) -> TaskRow {
        TaskRow {
            id: "t1".into(),
            title: "Storage".into(),
            status: status.into(),
            writes,
            reason: Some("the verifier repeated the same findings".into()),
            ..TaskRow::default()
        }
    }

    fn press(modal: &mut Unblock, code: KeyCode) -> Outcome {
        modal.key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn actions(task: &TaskRow) -> Vec<RecoveryAction> {
        choices_for(task).into_iter().map(|c| c.action).collect()
    }

    #[test]
    fn the_choices_fit_why_the_task_stopped() {
        use RecoveryAction::*;
        assert_eq!(
            actions(&row("needs_user", true)),
            [
                RetryWithGuidance,
                ExtendRounds,
                IntegrateWithConsent,
                KeepOnBranch,
                VerifyManualSnapshot,
                RestartAgent,
                ForceApprove
            ]
        );
        assert_eq!(
            actions(&row("needs_user", false)),
            [RetryWithGuidance, ExtendRounds, RestartAgent, ForceApprove]
        );
        assert_eq!(
            actions(&row("waiting", true)),
            [RetryWithGuidance, RestartAgent, ForceApprove]
        );
        assert!(is_blocked("skipped") && !is_blocked("working") && !is_blocked("approved"));
    }

    #[test]
    fn active_tasks_offer_only_restart() {
        for status in ["queued", "working", "verifying"] {
            assert_eq!(actions(&row(status, true)), [RecoveryAction::RestartAgent]);
        }
    }

    #[test]
    fn keys_and_clicks_choose_and_the_text_asking_ones_say_so() {
        let mut modal = Unblock::new("p1", &row("needs_user", false));
        assert_eq!(press(&mut modal, KeyCode::Down), Outcome::None);
        let Outcome::Choose(choice) = press(&mut modal, KeyCode::Enter) else {
            panic!("Enter chooses");
        };
        assert_eq!(choice.action, RecoveryAction::ExtendRounds);
        assert!(!choice.asks_text);
        let Outcome::Choose(choice) = press(&mut modal, KeyCode::Char('1')) else {
            panic!("a number chooses");
        };
        assert!(choice.asks_text);
        assert_eq!(press(&mut modal, KeyCode::Char('9')), Outcome::None);
        assert_eq!(press(&mut modal, KeyCode::Esc), Outcome::Close);
        // Up at the top and Down at the bottom stay put.
        modal.selected = 0;
        press(&mut modal, KeyCode::Up);
        assert_eq!(modal.selected, 0);
        for _ in 0..10 {
            press(&mut modal, KeyCode::Down);
        }
        assert_eq!(modal.selected, modal.choices.len() - 1);
    }

    #[test]
    fn the_modal_draws_on_any_terminal_and_clicks_hit_what_was_drawn() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut modal = Unblock::new("p1", &row("needs_user", true));
        for (w, h) in [(80, 24), (40, 12), (12, 5), (2, 2)] {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| modal.draw(f)).unwrap();
        }
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        term.draw(|f| modal.draw(f)).unwrap();
        let text: String = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("Task actions t1"));
        assert!(text.contains("the verifier repeated the same findings"));
        let rect = modal.rects[2];
        let Outcome::Choose(choice) = modal.click(rect.x + 2, rect.y) else {
            panic!("a click on a choice chooses it");
        };
        assert_eq!(choice.action, RecoveryAction::IntegrateWithConsent);
        assert_eq!(modal.click(0, 0), Outcome::None);
    }
}
