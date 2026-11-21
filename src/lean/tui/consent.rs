use super::*;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryConsent {
    pub plan: String,
    pub task: String,
    pub status: String,
    pub target: String,
    pub base: String,
    pub result: String,
    pub token: String,
    pub require_all: bool,
    pub files: Vec<(String, bool)>,
    pub selected: usize,
    pub scroll: usize,
    pub confirming: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    None,
    Close,
    Confirm,
}

impl RecoveryConsent {
    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        if self.confirming {
            return match key.code {
                KeyCode::Esc | KeyCode::Char('n') => {
                    self.confirming = false;
                    Outcome::None
                }
                KeyCode::Enter | KeyCode::Char('y') => Outcome::Confirm,
                KeyCode::Up | KeyCode::Char('k') => {
                    self.scroll = self.scroll.saturating_sub(1);
                    Outcome::None
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.scroll = self.scroll.saturating_add(1);
                    Outcome::None
                }
                _ => Outcome::None,
            };
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => Outcome::Close,
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                self.scroll = self.selected;
                Outcome::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.files.len().saturating_sub(1));
                self.scroll = self.selected;
                Outcome::None
            }
            KeyCode::Char(' ') => {
                if !self.require_all
                    && let Some((_, selected)) = self.files.get_mut(self.selected)
                {
                    *selected = !*selected;
                }
                Outcome::None
            }
            KeyCode::Char('a') => {
                if self.require_all {
                    self.files
                        .iter_mut()
                        .for_each(|(_, selected)| *selected = true);
                    return Outcome::None;
                }
                let select_all = self.files.iter().any(|(_, selected)| !selected);
                self.files
                    .iter_mut()
                    .for_each(|(_, selected)| *selected = select_all);
                Outcome::None
            }
            KeyCode::Enter | KeyCode::Char('c')
                if self.files.iter().any(|(_, selected)| *selected) =>
            {
                self.confirming = true;
                Outcome::None
            }
            _ => Outcome::None,
        }
    }

    pub fn selected_files(&self) -> Vec<String> {
        self.files
            .iter()
            .filter(|(_, selected)| *selected)
            .map(|(path, _)| path.clone())
            .collect()
    }

    pub fn draw(&self, frame: &mut Frame) {
        let theme = Theme::from_env();
        let separator = theme.chrome("·");
        let area = centered(frame.area());
        frame.render_widget(Clear, area);
        let title = if self.confirming {
            " Confirm recovery integration "
        } else {
            " Recovery integration consent "
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_set(theme.borders())
            .title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let files = if self.confirming {
            self.selected_files()
        } else {
            self.files
                .iter()
                .map(|(path, selected)| format!("{} {path}", if *selected { "[x]" } else { "[ ]" }))
                .collect()
        };
        let mut lines = vec![
            Line::from(format!(
                "Task: {} {separator} status: {}",
                self.task, self.status
            )),
            Line::from(format!(
                "Target: {} {separator} base {} {separator} snapshot {}",
                self.target, self.base, self.result
            )),
            Line::from(if self.confirming {
                "Files that will be authorized:"
            } else {
                "Select files (Space toggle, a all, Enter review confirmation):"
            }),
        ];
        let visible = usize::from(inner.height.saturating_sub(5)).max(1);
        let start = if self.confirming {
            self.scroll.min(files.len().saturating_sub(visible))
        } else {
            self.selected.saturating_sub(visible.saturating_sub(1))
        };
        lines.extend(files.into_iter().enumerate().skip(start).take(visible).map(
            |(index, path)| {
                let style = if !self.confirming && index == self.selected {
                    Style::default().add_modifier(Modifier::REVERSED)
                } else {
                    Style::default()
                };
                Line::from(Span::styled(path, style)).alignment(Alignment::Left)
            },
        ));
        if self.require_all && !self.confirming {
            lines.push(Line::from(
                "All declared output files require consent for artifact delivery.",
            ));
        }
        if let Some(error) = &self.error {
            lines.push(Line::from(Span::styled(error.clone(), theme.warning())));
        }
        lines.push(Line::from(if self.confirming {
            theme.chrome("Enter integrates these files · ↑/↓ scroll · Esc returns to selection")
        } else {
            theme.chrome("Esc cancels · current freshness token is held for confirmation")
        }));
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
    }
}

fn centered(area: Rect) -> Rect {
    let width = area.width.saturating_sub(4).clamp(12, 100);
    let height = area.height.saturating_sub(4).clamp(5, 28);
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn consent() -> RecoveryConsent {
        RecoveryConsent {
            plan: "p".into(),
            task: "t".into(),
            status: "needs_user".into(),
            target: "project working tree".into(),
            base: "base123456789".into(),
            result: "result123456".into(),
            token: "token".into(),
            require_all: false,
            files: vec![("a.rs".into(), true), ("b.rs".into(), false)],
            selected: 0,
            scroll: 0,
            confirming: false,
            error: None,
        }
    }

    #[test]
    fn confirmation_retains_exact_selection_and_cancel_never_submits() {
        let mut state = consent();
        state.selected = 1;
        state.key(KeyEvent::new(
            KeyCode::Char(' '),
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(state.selected_files(), ["a.rs", "b.rs"]);
        state.key(KeyEvent::new(
            KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(state.confirming);
        assert_eq!(
            state.key(KeyEvent::new(
                KeyCode::Esc,
                crossterm::event::KeyModifiers::NONE
            )),
            Outcome::None
        );
        assert!(!state.confirming);
        assert_eq!(state.selected_files(), ["a.rs", "b.rs"]);
    }

    #[test]
    fn empty_consent_cannot_enter_confirmation() {
        let mut state = consent();
        state
            .files
            .iter_mut()
            .for_each(|(_, selected)| *selected = false);
        state.key(KeyEvent::new(
            KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert!(!state.confirming);
    }

    #[test]
    fn confirmation_displays_target_full_snapshot_and_exact_authorized_files() {
        use ratatui::{Terminal, backend::TestBackend};

        let mut state = consent();
        state.base = "0123456789abcdef".into();
        state.result = "fedcba9876543210".into();
        state.confirming = true;
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal.draw(|frame| state.draw(frame)).unwrap();
        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("project working tree"));
        assert!(rendered.contains("0123456789abcdef"));
        assert!(rendered.contains("fedcba9876543210"));
        assert!(rendered.contains("a.rs"));
        assert!(!rendered.contains("b.rs"));
    }
}
