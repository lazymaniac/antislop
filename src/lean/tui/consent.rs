use super::*;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

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
            KeyCode::PageUp => {
                self.scroll = self.scroll.saturating_sub(1);
                Outcome::None
            }
            KeyCode::PageDown => {
                self.scroll = self.scroll.saturating_add(1);
                Outcome::None
            }
            KeyCode::Esc | KeyCode::Char('q') => Outcome::Close,
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected = self.selected.saturating_sub(1);
                self.scroll = 0;
                Outcome::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.files.len().saturating_sub(1));
                self.scroll = 0;
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
                self.scroll = 0;
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
                .iter()
                .map(|path| format!("{path:?}"))
                .collect::<Vec<_>>()
        } else {
            self.files
                .iter()
                .map(|(path, selected)| {
                    format!("{} {path:?}", if *selected { "[x]" } else { "[ ]" })
                })
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
        // Scroll physical, grapheme-wrapped rows, not file indices: even one
        // legal Git path can occupy more than the entire viewport.
        let width = usize::from(inner.width).max(1);
        let mut file_lines = Vec::new();
        let mut selected_start = 0;
        for (index, path) in files.iter().enumerate() {
            if index == self.selected {
                selected_start = file_lines.len();
            }
            let style = if !self.confirming && index == self.selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            file_lines.extend(
                theme::wrap(path, width)
                    .into_iter()
                    .map(|row| Line::from(Span::styled(row, style)).alignment(Alignment::Left)),
            );
        }
        let mut body = lines
            .drain(..)
            .flat_map(|line| {
                theme::wrap(&line.to_string(), width)
                    .into_iter()
                    .map(Line::raw)
            })
            .collect::<Vec<_>>();
        let header_rows = body.len();
        body.extend(file_lines);
        if self.require_all && !self.confirming {
            body.extend(
                theme::wrap(
                    "All declared output files require consent for artifact delivery.",
                    width,
                )
                .into_iter()
                .map(Line::raw),
            );
        }
        if let Some(error) = &self.error {
            body.extend(
                theme::wrap(error, width)
                    .into_iter()
                    .map(|line| Line::styled(line, theme.warning())),
            );
        }
        let visible = usize::from(inner.height.saturating_sub(1));
        let selection_start = if self.confirming || self.selected == 0 {
            0
        } else {
            header_rows
                .saturating_add(selected_start)
                .saturating_sub(visible.saturating_sub(1))
        };
        let start = selection_start
            .saturating_add(self.scroll)
            .min(body.len().saturating_sub(visible));
        let content = Rect::new(
            inner.x,
            inner.y,
            inner.width,
            inner.height.saturating_sub(1),
        );
        frame.render_widget(
            Paragraph::new(
                body.into_iter()
                    .skip(start)
                    .take(visible)
                    .collect::<Vec<_>>(),
            ),
            content,
        );
        let hint = if inner.width < 40 {
            "Enter/Esc".to_owned()
        } else if self.confirming {
            theme.chrome("Enter integrates these files · ↑/↓ scroll · Esc returns to selection")
        } else {
            theme.chrome("Space toggle · Enter review · PgUp/PgDn scroll · Esc cancel")
        };
        if inner.height > 0 {
            frame.render_widget(
                Paragraph::new(view::truncate(&hint, width)),
                Rect::new(inner.x, inner.bottom() - 1, inner.width, 1),
            );
        }
    }
}

fn centered(area: Rect) -> Rect {
    let width = if area.width < 16 {
        area.width
    } else {
        area.width.saturating_sub(4).min(84)
    };
    let height = if area.height < 9 {
        area.height
    } else {
        area.height.saturating_sub(4).min(28)
    };
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
    fn consent_distinguishes_control_names_and_scrolls_to_long_path_suffix() {
        use ratatui::{Terminal, backend::TestBackend};
        let mut state = consent();
        let paths = ["a\tb.txt", "a\nb.txt", "ab.txt", "a\\nb.txt"];
        state.files = paths.iter().map(|path| ((*path).into(), true)).collect();
        state.confirming = true;
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        let text = |terminal: &Terminal<TestBackend>| {
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>()
        };
        terminal.draw(|frame| state.draw(frame)).unwrap();
        let rendered = text(&terminal);
        for path in paths {
            assert!(rendered.contains(&format!("{path:?}")), "{rendered}");
        }
        assert_eq!(state.selected_files(), paths);
        let path = format!("{}END-OF-AUTHORIZED-PATH.txt", "segment/".repeat(180));
        state.files = vec![(path.clone(), true)];
        terminal.draw(|frame| state.draw(frame)).unwrap();
        assert!(!text(&terminal).contains("END-OF-AUTHORIZED-PATH.txt"));
        for _ in 0..100 {
            state.key(KeyEvent::new(
                KeyCode::Down,
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        terminal.draw(|frame| state.draw(frame)).unwrap();
        assert!(text(&terminal).contains("END-OF-AUTHORIZED-PATH.txt"));
        assert!(text(&terminal).contains("Enter integrates"));
        assert_eq!(state.selected_files(), [path]);
    }

    #[test]
    fn consent_long_path_remains_readable_at_every_required_geometry() {
        use ratatui::{Terminal, backend::TestBackend};
        for (width, height) in [(12, 5), (40, 12), (80, 24), (100, 30), (120, 40), (160, 50)] {
            let mut state = consent();
            let path = format!("{}PATHEND", "segment/".repeat(180));
            state.files = vec![(path.clone(), true)];
            state.confirming = true;
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut reached_end = false;
            for _ in 0..200 {
                terminal.draw(|frame| state.draw(frame)).unwrap();
                let inner = centered(Rect::new(0, 0, width, height))
                    .inner(ratatui::layout::Margin::new(1, 1));
                let visible = (inner.y..inner.bottom().saturating_sub(1))
                    .map(|y| {
                        (inner.x..inner.right())
                            .map(|x| terminal.backend().buffer()[(x, y)].symbol())
                            .collect::<String>()
                            .trim_end()
                            .to_owned()
                    })
                    .collect::<String>();
                reached_end |= visible.contains("PATHEND");
                let footer = (inner.x..inner.right())
                    .map(|x| terminal.backend().buffer()[(x, inner.bottom() - 1)].symbol())
                    .collect::<String>();
                assert!(footer.contains("Enter"), "{width}x{height}");
                state.key(KeyEvent::new(
                    KeyCode::Down,
                    crossterm::event::KeyModifiers::NONE,
                ));
            }
            assert!(reached_end, "{width}x{height} hid the path suffix");
            assert_eq!(state.selected_files(), [path]);
        }
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
