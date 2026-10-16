//! Full-screen details of one agent: what it was asked, its session, the
//! messages it sent and received, and what it delivered.

use super::view::{View, agent_name, dim, polish, wrap};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use serde_json::Value;

pub const TABS: [&str; 5] = ["Przegląd", "Sesja", "Wiadomości", "Wynik", "Komentarze"];
pub const RESULT: usize = 3;
pub const COMMENTS: usize = 4;

#[derive(Debug)]
pub struct Details {
    pub key: String,
    pub tab: usize,
    /// Lines scrolled down from the top.
    pub scroll: usize,
    /// The task as the daemon stores it, once fetched.
    pub task: Option<Value>,
    /// The selected comment on the Komentarze tab.
    pub comment: usize,
}

impl Details {
    pub fn new(key: String) -> Self {
        Details {
            key,
            tab: 0,
            scroll: 0,
            task: None,
            comment: 0,
        }
    }

    /// The id of the selected comment, on the Komentarze tab.
    pub fn selected_comment(&self, view: &View) -> Option<u64> {
        (self.tab == COMMENTS)
            .then(|| view.comments_on(&self.key).get(self.comment).map(|c| c.id))
            .flatten()
    }

    /// The task id of a worker or verifier.
    pub fn task_id(&self) -> Option<&str> {
        self.key.split_once(':').map(|(_, id)| id)
    }

    fn lines(&mut self, view: &View) -> Vec<(Style, String)> {
        let plain = |text: String| (Style::default(), text);
        let label = |text: &str| {
            (
                Style::default().add_modifier(Modifier::BOLD),
                text.to_owned(),
            )
        };
        let agent = view.agents.get(&self.key);
        let task = self
            .task_id()
            .and_then(|id| view.tasks.iter().find(|task| task.id == id));
        let verifier = self.key.starts_with("verifier:");
        let mut lines = vec![];
        match self.tab {
            0 => {
                if let Some(agent) = agent.filter(|agent| !agent.provider.is_empty()) {
                    lines.push(plain(format!("CLI: {}", agent.provider)));
                }
                if let Some(task) = task {
                    lines.push(plain(format!(
                        "Status zadania: {}{}",
                        polish(&task.status),
                        if task.round > 0 {
                            format!(" (runda {})", task.round)
                        } else {
                            String::new()
                        }
                    )));
                    lines.push(plain(format!("Zadanie {}: {}", task.id, task.title)));
                    lines.push(plain(format!(
                        "Rodzaj: {}{}",
                        task.kind,
                        if task.writes {
                            " · zmienia pliki"
                        } else {
                            " · tylko odczyt"
                        }
                    )));
                    if !task.depends_on.is_empty() {
                        lines.push(plain(format!("Zależy od: {}", task.depends_on.join(", "))));
                    }
                    if !task.scope.is_empty() {
                        lines.push(plain(format!("Zakres: {}", task.scope.join(", "))));
                    }
                    lines.push(plain(String::new()));
                    lines.push(label("Cel"));
                    lines.push(plain(task.goal.clone()));
                    if verifier || !task.focus.is_empty() {
                        lines.push(plain(String::new()));
                        lines.push(label("Weryfikacja"));
                        lines.push(plain(task.focus.clone()));
                    }
                    // Risks belong to the verifier that guards them.
                    if verifier && !task.risks.is_empty() {
                        lines.push(plain(String::new()));
                        lines.push(label("Ryzyka"));
                        for risk in &task.risks {
                            lines.push(plain(format!("- {risk}")));
                        }
                    }
                }
                if let Some(review) = view.reviews.get(&self.key) {
                    lines.push(plain(String::new()));
                    lines.push(label("Ostatni wynik"));
                    lines.extend(review.iter().cloned().map(plain));
                }
                if let Some(agent) = agent {
                    lines.push(plain(String::new()));
                    lines.push(label("Teraz"));
                    lines.push(plain(if agent.running {
                        format!("pracuje: {}", agent.activity)
                    } else {
                        format!("bezczynny · {}", agent.activity)
                    }));
                } else if task.is_none() {
                    lines.push(plain("Ten agent jeszcze nie pracował.".into()));
                }
            }
            1 => match agent {
                Some(agent) if !agent.session.is_empty() => {
                    lines.extend(agent.session.iter().cloned().map(plain));
                }
                _ => lines.push((dim(), "Brak zdarzeń sesji.".into())),
            },
            2 => match view.mailbox.get(&self.key) {
                Some(mail) => lines.extend(mail.iter().cloned().map(plain)),
                None => lines.push((dim(), "Brak wiadomości.".into())),
            },
            COMMENTS => {
                let comments = view.comments_on(&self.key);
                if comments.is_empty() {
                    lines.push((dim(), "Brak komentarzy. c dodaje komentarz.".into()));
                }
                self.comment = self.comment.min(comments.len().saturating_sub(1));
                for (index, comment) in comments.iter().enumerate() {
                    let mut style = if comment.status == "open" {
                        Style::default()
                    } else {
                        dim()
                    };
                    if index == self.comment {
                        style = style.add_modifier(Modifier::REVERSED);
                    }
                    let state = match comment.status.as_str() {
                        "open" => "otwarty",
                        "resolved" => "rozwiązany",
                        other => other,
                    };
                    let stale = if view.stale(comment) {
                        " · dotyczy poprzedniego planu"
                    } else {
                        ""
                    };
                    lines.push((
                        style,
                        format!(
                            "#{} {} · {state}{stale}",
                            comment.id,
                            super::view::target_name(&comment.target)
                        ),
                    ));
                    lines.push((Style::default(), format!("  {}", comment.text)));
                }
            }
            _ => match (&self.task, task) {
                (_, None) => lines.push((
                    dim(),
                    "Wynik mają workery i weryfikatorzy zadań; wyniki weryfikatora planu \
                     i audytora są w Przeglądzie."
                        .into(),
                )),
                (None, Some(_)) => lines.push((dim(), "Pobieram…".into())),
                (Some(stored), Some(_)) if verifier => match &stored["verdict"] {
                    Value::Null => lines.push((dim(), "Weryfikator jeszcze nie ocenił.".into())),
                    verdict => {
                        lines.extend(
                            super::super::cli::review_lines("werdykt", verdict)
                                .into_iter()
                                .map(plain),
                        );
                    }
                },
                (Some(stored), Some(_)) => {
                    if let (Some(base), Some(result)) =
                        (stored["base"].as_str(), stored["result"].as_str())
                    {
                        lines.push(plain(format!(
                            "Zmiany: {}..{}",
                            &base[..base.len().min(10)],
                            &result[..result.len().min(10)]
                        )));
                        lines.push(plain(String::new()));
                    }
                    match stored["report"].as_str() {
                        Some(report) => {
                            lines.push(label("Raport"));
                            lines.push(plain(report.to_owned()));
                        }
                        None => lines.push((dim(), "Worker jeszcze nie oddał raportu.".into())),
                    }
                }
            },
        }
        lines
    }

    pub fn draw(&mut self, frame: &mut Frame, view: &View) {
        let area = frame.area();
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(area);
        let mut tabs = vec![Span::styled(
            format!(" {} ", agent_name(&self.key)),
            Style::default().add_modifier(Modifier::BOLD),
        )];
        for (index, name) in TABS.iter().enumerate() {
            tabs.push(Span::raw(" "));
            tabs.push(if index == self.tab {
                Span::styled(
                    format!("[{name}]"),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )
            } else {
                Span::styled(format!(" {name} "), dim())
            });
        }
        frame.render_widget(Paragraph::new(Line::from(tabs)), header);

        let block = Block::default().borders(Borders::TOP);
        let inner = block.inner(body);
        frame.render_widget(block, body);
        let width = usize::from(inner.width.saturating_sub(1));
        let lines: Vec<Line> = self
            .lines(view)
            .into_iter()
            .flat_map(|(style, text)| {
                wrap(&text, width)
                    .into_iter()
                    .map(move |line| Line::styled(line, style))
            })
            .collect();
        let height = usize::from(inner.height);
        self.scroll = self.scroll.min(lines.len().saturating_sub(height));
        let end = (self.scroll + height).min(lines.len());
        frame.render_widget(Paragraph::new(lines[self.scroll..end].to_vec()), inner);
        frame.render_widget(
            Paragraph::new(Span::styled(
                if self.tab == COMMENTS {
                    "←/→ zakładka · ↑/↓ wybór · c skomentuj · r rozwiąż · o otwórz · x usuń · d diff · Esc powrót"
                } else {
                    "←/→ zakładka · ↑/↓ PgUp/PgDn przewiń · c skomentuj · d diff · Esc powrót do grafu"
                },
                dim(),
            )),
            footer,
        );
    }
}
