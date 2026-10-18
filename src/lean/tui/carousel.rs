//! Task carousel view showing full task details, real-time counters,
//! and transitions to worker and verifier.

use super::graph::mark;
use super::theme::Theme;
use super::view::{TaskRow, View, dim, polish};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Carousel {
    pub selected: usize,
    pub scroll: usize,
}

impl Carousel {
    pub fn new() -> Self {
        Self {
            selected: 0,
            scroll: 0,
        }
    }

    pub fn prev(&mut self, total: usize) {
        if total == 0 {
            self.selected = 0;
            return;
        }
        if self.selected == 0 {
            self.selected = total.saturating_sub(1);
        } else {
            self.selected = self.selected.saturating_sub(1);
        }
    }

    pub fn next(&mut self, total: usize) {
        if total == 0 {
            self.selected = 0;
            return;
        }
        if self.selected + 1 >= total {
            self.selected = 0;
        } else {
            self.selected += 1;
        }
    }

    pub fn set_selected(&mut self, idx: usize, total: usize) {
        if total == 0 {
            self.selected = 0;
        } else {
            self.selected = idx.min(total.saturating_sub(1));
        }
    }

    pub fn current_task<'a>(&self, view: &'a View) -> Option<&'a TaskRow> {
        if view.tasks.is_empty() {
            None
        } else {
            let idx = self.selected.min(view.tasks.len().saturating_sub(1));
            view.tasks.get(idx)
        }
    }

    pub fn draw(&mut self, frame: &mut Frame, area: Rect, view: &View, theme: &Theme) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        if view.tasks.is_empty() {
            let empty_text = vec![
                Line::styled(
                    "Brak zadań w bieżącym planie.",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Line::styled("", Style::default()),
                Line::styled(
                    "Orkiestrator zaproponuje zadania po analizie Twojego zapytania w czacie.",
                    dim(),
                ),
                Line::styled(
                    "Naciśnij Tab, aby wrócić do zespołu lub Esc, aby pisać w czacie.",
                    dim(),
                ),
            ];
            let block = Block::default()
                .borders(Borders::ALL)
                .title(" Zadania (karuzela) ");
            let p = Paragraph::new(empty_text).block(block);
            frame.render_widget(p, area);
            return;
        }

        let total = view.tasks.len();
        self.selected = self.selected.min(total.saturating_sub(1));
        let task = &view.tasks[self.selected];

        let [ribbon_area, body_area, actions_area] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .areas(area);

        // 1. Ribbon / selector bar
        self.draw_ribbon(frame, ribbon_area, view, theme);

        // 2. Main task card with real counters
        self.draw_task_card(frame, body_area, task, view, theme);

        // 3. Action bar with transitions to worker and verifier
        self.draw_actions(frame, actions_area, task, theme);
    }

    fn draw_ribbon(&self, frame: &mut Frame, area: Rect, view: &View, theme: &Theme) {
        let total = view.tasks.len();
        let mut spans = Vec::new();
        spans.push(Span::styled("◄ ", theme.accent()));

        for (i, t) in view.tasks.iter().enumerate() {
            let is_cur = i == self.selected;
            let (m, col) = mark(&t.status);
            let style = if is_cur {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Blue)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(col)
            };
            let sep_style = theme.dim();
            let label = format!(" [{}/{}] {} {} ", i + 1, total, m, t.id);
            spans.push(Span::styled(label, style));
            if i + 1 < total {
                spans.push(Span::styled("│", sep_style));
            }
        }
        spans.push(Span::styled(" ►", theme.accent()));

        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Karuzela zadań ");
        let p = Paragraph::new(Line::from(spans)).block(block);
        frame.render_widget(p, area);
    }

    fn draw_task_card(
        &self,
        frame: &mut Frame,
        area: Rect,
        task: &TaskRow,
        view: &View,
        theme: &Theme,
    ) {
        let (status_mark, status_color) = mark(&task.status);
        let open_comments = view
            .comments_on(&format!("worker:{}", task.id))
            .iter()
            .chain(view.comments_on(&format!("verifier:{}", task.id)).iter())
            .filter(|c| c.status == "open")
            .count();
        let total_comments = view
            .comments_on(&format!("worker:{}", task.id))
            .iter()
            .chain(view.comments_on(&format!("verifier:{}", task.id)).iter())
            .count();

        let worker_key = format!("worker:{}", task.id);
        let verifier_key = format!("verifier:{}", task.id);
        let worker_agent = view.agents.get(&worker_key);
        let verifier_agent = view.agents.get(&verifier_key);

        let mut lines = Vec::new();

        // Header line
        lines.push(Line::from(vec![
            Span::styled(
                format!("Zadanie {}: ", task.id),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                task.title.clone(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        lines.push(Line::styled("", Style::default()));

        // Real counters section
        lines.push(Line::from(vec![Span::styled(
            "● Rzeczywiste liczniki i stan:",
            theme.accent(),
        )]));
        lines.push(Line::from(vec![
            Span::styled("  Stan: ", theme.dim()),
            Span::styled(
                format!("{status_mark} {}", polish(&task.status)),
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("  ·  Runda: r{}", task.round),
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                format!("  ·  Komentarze: ✎{open_comments} otwartych ({total_comments} łącznie)"),
                Style::default().fg(if open_comments > 0 {
                    Color::Cyan
                } else {
                    Color::DarkGray
                }),
            ),
        ]));

        let deps_str = if task.depends_on.is_empty() {
            "brak (korzeń grafu)".into()
        } else {
            task.depends_on.join(", ")
        };
        lines.push(Line::from(vec![
            Span::styled("  Zależności [D]: ", theme.dim()),
            Span::styled(
                format!("{} (liczba: {})", deps_str, task.depends_on.len()),
                Style::default().fg(Color::Blue),
            ),
        ]));

        let scope_str = if task.scope.is_empty() {
            "cały projekt".into()
        } else {
            task.scope.join(", ")
        };
        lines.push(Line::from(vec![
            Span::styled("  Zakres plików: ", theme.dim()),
            Span::styled(
                format!("{} ścieżek ({scope_str})", task.scope.len()),
                Style::default(),
            ),
            Span::styled(
                if task.writes {
                    " · modyfikuje pliki"
                } else {
                    " · tylko do odczytu"
                },
                Style::default().fg(if task.writes {
                    Color::Yellow
                } else {
                    Color::Green
                }),
            ),
        ]));

        // Agents information
        let worker_info = worker_agent.map_or("nieuruchomiony", |a| {
            if a.running {
                "● pracuje"
            } else if !a.activity.is_empty() {
                a.activity.as_str()
            } else {
                "bezczynny"
            }
        });
        let verifier_info = verifier_agent.map_or("nieuruchomiony", |a| {
            if a.running {
                "● sprawdza"
            } else if !a.activity.is_empty() {
                a.activity.as_str()
            } else {
                "bezczynny"
            }
        });
        lines.push(Line::from(vec![
            Span::styled("  Worker: ", theme.dim()),
            Span::styled(worker_info, Style::default().fg(Color::Cyan)),
            Span::styled("  │  Weryfikator [V]: ", theme.dim()),
            Span::styled(verifier_info, Style::default().fg(Color::Yellow)),
        ]));

        lines.push(Line::styled("", Style::default()));

        // Goal & Focus
        if !task.goal.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("Cel: ", theme.dim()),
                Span::styled(task.goal.clone(), Style::default()),
            ]));
        }
        if !task.focus.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("Fokus weryfikacji: ", theme.dim()),
                Span::styled(task.focus.clone(), Style::default().fg(Color::Yellow)),
            ]));
        }

        // Plan metrics summary
        let approved = view.tasks.iter().filter(|t| t.status == "approved").count();
        let working = view
            .tasks
            .iter()
            .filter(|t| t.status == "working" || t.status == "verifying")
            .count();
        let queued = view
            .tasks
            .iter()
            .filter(|t| matches!(t.status.as_str(), "proposed" | "queued" | "waiting"))
            .count();
        let problems = view
            .tasks
            .iter()
            .filter(|t| {
                matches!(
                    t.status.as_str(),
                    "failed" | "needs_user" | "cancelled" | "skipped" | "revised"
                )
            })
            .count();
        let unclassified = view
            .tasks
            .len()
            .saturating_sub(approved + working + queued + problems);
        let problems = problems + unclassified;

        lines.push(Line::styled("", Style::default()));
        lines.push(Line::from(vec![
            Span::styled(format!("Podsumowanie planu: {} zadań łącznie  │  ✓ {} zatwierdzone  │  ● {} w toku  │  ○ {} oczekuje  │  ✕ {} problemów",
                view.tasks.len(), approved, working, queued, problems
            ), theme.dim()),
        ]));

        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" Szczegóły zadania {} ", task.id));
        let p = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
        frame.render_widget(p, area);
    }

    fn draw_actions(&self, frame: &mut Frame, area: Rect, _task: &TaskRow, theme: &Theme) {
        let spans = vec![
            Span::styled("Skróty: ", theme.dim()),
            Span::styled(
                "[w] ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Przejdź do workera na grafie  ", Style::default()),
            Span::styled(
                "[v] ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Przejdź do weryfikatora  ", Style::default()),
            Span::styled(
                "[Enter] ",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Szczegóły  ", Style::default()),
            Span::styled(
                "[d] ",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Diff  ", Style::default()),
            Span::styled(
                "[c] ",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Komentarz  ", Style::default()),
            Span::styled(
                "[←/→] ",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Nawigacja karuzeli", Style::default()),
        ];
        let p = Paragraph::new(Line::from(spans)).wrap(Wrap { trim: true });
        frame.render_widget(p, area);
    }
}
