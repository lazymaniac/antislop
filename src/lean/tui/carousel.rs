//! Task carousel view showing full task details, real-time counters,
//! and transitions to worker and verifier.

use super::graph::mark;
use super::taskinfo;
use super::theme::Theme;
use super::view::{TaskRow, View, dim, status_label};
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
                    "No tasks in the current plan.",
                    Style::default().add_modifier(Modifier::BOLD),
                ),
                Line::styled("", Style::default()),
                Line::styled(
                    "The orchestrator will propose tasks after analyzing your request in chat.",
                    dim(),
                ),
                Line::styled(
                    "Press Tab to return to the team or Esc to type in chat.",
                    dim(),
                ),
            ];
            let block = Block::default()
                .borders(Borders::ALL)
                .title(" Tasks (carousel) ");
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
            .title(" Task carousel ");
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
                format!("Task {}: ", task.id),
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
            "● Live counters and state:",
            theme.accent(),
        )]));
        lines.push(Line::from(vec![
            Span::styled("  Status: ", theme.dim()),
            Span::styled(
                format!("{status_mark} {}", status_label(&task.status)),
                Style::default()
                    .fg(status_color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                match taskinfo::round_shown(task.round) {
                    Some(round) => format!("  ·  Round: {round}"),
                    None => "  ·  Round: not started".to_owned(),
                },
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                format!("  ·  Comments: ✎{open_comments} open ({total_comments} total)"),
                Style::default().fg(if open_comments > 0 {
                    Color::Cyan
                } else {
                    Color::DarkGray
                }),
            ),
        ]));

        let deps_str = if task.depends_on.is_empty() {
            "none (graph root)".into()
        } else {
            task.depends_on.join(", ")
        };
        let par_str = if task.parallel_with.is_empty() {
            "sequential".into()
        } else {
            task.parallel_with.join(", ")
        };
        lines.push(Line::from(vec![
            Span::styled("  Dependencies [D]: ", theme.dim()),
            Span::styled(
                format!("{} (count: {})", deps_str, task.depends_on.len()),
                Style::default().fg(Color::Blue),
            ),
            Span::styled("  ·  Parallel with: ", theme.dim()),
            Span::styled(
                par_str,
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        if let Some(sec) = task.section.as_deref().filter(|s| !s.is_empty()) {
            let mut sec_style = Style::default().fg(Color::Cyan);
            if task.status == "approved" {
                sec_style = sec_style
                    .fg(Color::Green)
                    .add_modifier(Modifier::UNDERLINED | Modifier::BOLD);
            }
            lines.push(Line::from(vec![
                Span::styled("  Plan section: ", theme.dim()),
                Span::styled(sec.to_owned(), sec_style),
            ]));
        }

        let scope_str = if task.scope.is_empty() {
            "entire project".into()
        } else {
            task.scope.join(", ")
        };
        lines.push(Line::from(vec![
            Span::styled("  File scope: ", theme.dim()),
            Span::styled(
                format!("{} paths ({scope_str})", task.scope.len()),
                Style::default(),
            ),
            Span::styled(
                if task.writes {
                    " · modifies files"
                } else {
                    " · read-only"
                },
                Style::default().fg(if task.writes {
                    Color::Yellow
                } else {
                    Color::Green
                }),
            ),
        ]));

        // Agents information: worker and verifier keep separate lines, each
        // with the round its work belongs to.
        let round = taskinfo::round_shown(task.round);
        let with_round = |text: &str| match &round {
            Some(round) => format!("{text} {round}"),
            None => text.to_owned(),
        };
        let worker_info = worker_agent.map_or("not started".to_owned(), |a| {
            if a.running {
                with_round("● working")
            } else if !a.activity.is_empty() {
                a.activity.clone()
            } else {
                "idle".to_owned()
            }
        });
        let verifier_info = verifier_agent.map_or("not started".to_owned(), |a| {
            if a.running {
                with_round("● checking")
            } else if !a.activity.is_empty() {
                a.activity.clone()
            } else {
                "idle".to_owned()
            }
        });
        lines.push(Line::from(vec![
            Span::styled("  Worker: ", theme.dim()),
            Span::styled(worker_info, Style::default().fg(Color::Cyan)),
            Span::styled("  │  Verifier [V]: ", theme.dim()),
            Span::styled(verifier_info, Style::default().fg(Color::Yellow)),
        ]));

        // What the round produced, what the verdict covered, and how it
        // landed — the same reading the graph card gives.
        lines.push(Line::from(vec![
            Span::styled("  Effect: ", theme.dim()),
            Span::styled(
                taskinfo::effect_line(task).unwrap_or_else(|| "no effect reported yet".to_owned()),
                Style::default(),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Obligations: ", theme.dim()),
            Span::styled(
                taskinfo::coverage_full(task),
                Style::default().fg(Color::Green),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Integration: ", theme.dim()),
            Span::styled(
                format!(
                    "{} · {}",
                    taskinfo::integration_label(task),
                    taskinfo::integration_detail(task)
                ),
                Style::default().fg(Color::Magenta),
            ),
        ]));
        lines.push(Line::from(vec![
            Span::styled("  Next: ", theme.dim()),
            Span::styled(
                taskinfo::next_step(view, task),
                Style::default().fg(Color::Cyan),
            ),
        ]));
        if let Some(why) = taskinfo::cause(task) {
            lines.push(Line::from(vec![
                Span::styled("  Cause: ", theme.dim()),
                Span::styled(why, Style::default().fg(Color::Red)),
            ]));
        }

        lines.push(Line::styled("", Style::default()));

        // Goal & Focus
        if !task.goal.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("Goal: ", theme.dim()),
                Span::styled(task.goal.clone(), Style::default()),
            ]));
        }
        if !task.focus.is_empty() {
            lines.push(Line::from(vec![
                Span::styled("Verification focus: ", theme.dim()),
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
        // Plan state counters: the coarse reading first, then every state's
        // own count, so no task ever disappears into an unnamed bucket.
        let per_state = taskinfo::state_counts(view)
            .into_iter()
            .filter(|(_, count)| *count > 0)
            .map(|(state, count)| format!("{state} {count}"))
            .collect::<Vec<_>>()
            .join(" · ");
        lines.push(Line::from(vec![
            Span::styled(format!("Plan summary: {} tasks total │ ✓ {} approved │ ● {} in progress │ ○ {} waiting │ ✕ {} issues",
                view.tasks.len(), approved, working, queued, problems
            ), theme.dim()),
        ]));
        lines.push(Line::from(vec![Span::styled(
            format!("Plan states: {per_state}"),
            theme.dim(),
        )]));

        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" Task {} details ", task.id));
        let p = Paragraph::new(lines).block(block).wrap(Wrap { trim: true });
        frame.render_widget(p, area);
    }

    fn draw_actions(&self, frame: &mut Frame, area: Rect, _task: &TaskRow, theme: &Theme) {
        let spans = vec![
            Span::styled("Shortcuts: ", theme.dim()),
            Span::styled(
                "[w] ",
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Jump to worker on graph  ", Style::default()),
            Span::styled(
                "[v] ",
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Jump to verifier  ", Style::default()),
            Span::styled(
                "[Enter] ",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Details  ", Style::default()),
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
            Span::styled("Comment  ", Style::default()),
            Span::styled(
                "[←/→] ",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Carousel navigation", Style::default()),
        ];
        let p = Paragraph::new(Line::from(spans)).wrap(Wrap { trim: true });
        frame.render_widget(p, area);
    }
}
