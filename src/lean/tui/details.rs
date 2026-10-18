//! Full-screen details of one agent: what it was asked, its session, the
//! messages it sent and received, and what it delivered.

use super::super::tasks::{TaskSpec, obligations};
use super::view::{View, agent_name, dim, duration, polish, wrap};
use ratatui::{
    Frame,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use serde_json::Value;

pub const TABS: [&str; 6] = [
    "Przegląd",
    "Sesja",
    "Wiadomości",
    "Wynik",
    "Kontrole",
    "Komentarze",
];
pub const RESULT: usize = 3;
pub const CHECKS: usize = 4;
pub const COMMENTS: usize = 5;

#[derive(Debug, Clone, PartialEq)]
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
                    let mut cli = format!("CLI: {}", agent.provider);
                    if let Some(model) = &agent.model {
                        cli.push_str(&format!(" · model {model}"));
                    }
                    if let Some(effort) = &agent.effort {
                        cli.push_str(&format!(" · effort {effort}"));
                    }
                    lines.push(plain(cli));
                    if agent.turns > 0 {
                        lines.push(plain(format!("Tokeny: {}", agent.tokens())));
                    }
                    if let Some(last) = agent.last_ms {
                        lines.push(plain(format!(
                            "Czas w turach: {} (ostatnia {})",
                            duration(agent.busy_ms),
                            duration(last)
                        )));
                    }
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
                    if let Some(reason) = &task.reason {
                        lines.push(plain(format!("Dlaczego: {reason}")));
                    }
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
                _ => lines.push((
                    dim(),
                    "Brak zdarzeń sesji dla tego agenta. Zdarzenia narzędzi pojawią się w trakcie pracy.".into(),
                )),
            },
            2 => match view.mailbox.get(&self.key) {
                Some(mail) => lines.extend(mail.iter().cloned().map(plain)),
                None => {
                    let id = self.task_id().unwrap_or(&self.key);
                    lines.push((
                        dim(),
                        format!("Brak wiadomości. Użyj '@{id} wiadomość' w czacie, aby napisać do tego agenta."),
                    ));
                }
            },
            COMMENTS => {
                let comments = view.comments_on(&self.key);
                if comments.is_empty() {
                    lines.push((
                        dim(),
                        "Brak komentarzy. Naciśnij 'c', aby dodać komentarz do tego elementu.".into(),
                    ));
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
            CHECKS => match (&self.task, task) {
                (_, None) => lines.push((
                    dim(),
                    "Kontrole mają zadania: ryzyka zadania i lista kontrolna jego \
                     rodzaju, rozliczane przez weryfikatora."
                        .into(),
                )),
                (None, Some(_)) => lines.push((dim(), "Pobieram…".into())),
                (Some(stored), Some(_)) => lines.extend(checks(stored)),
            },
            _ => match (&self.task, task) {
                (_, None) => lines.push((
                    dim(),
                    "Wynik mają workery i weryfikatorzy zadań; wyniki weryfikatora planu \
                     i audytora są w Przeglądzie."
                        .into(),
                )),
                (None, Some(_)) => lines.push((dim(), "Pobieram…".into())),
                (Some(stored), Some(_)) if verifier => match &stored["verdict"] {
                    Value::Null => lines.push((
                        dim(),
                        "Weryfikator jeszcze nie ocenił zadania (werdykt pojawi się po weryfikacji).".into(),
                    )),
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
                        None => lines.push((
                            dim(),
                            "Worker jeszcze nie oddał raportu (trwa praca lub oczekiwanie na start).".into(),
                        )),
                    }
                }
            },
        }
        lines
    }

    pub fn draw(&mut self, frame: &mut Frame, view: &View) {
        if self.tab >= TABS.len() {
            self.tab = 0;
        }
        let area = frame.area();
        if area.width == 0 || area.height == 0 {
            return;
        }
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .areas(area);

        let tabs_line = format_tabs(&self.key, self.tab, usize::from(header.width));
        frame.render_widget(Paragraph::new(tabs_line), header);

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

        let hint = details_footer_hint(self.tab, usize::from(footer.width));
        frame.render_widget(Paragraph::new(Span::styled(hint, dim())), footer);
    }
}

pub fn details_footer_hint(tab: usize, width: usize) -> &'static str {
    if tab == COMMENTS {
        if width >= 95 {
            "←/→ zakładka · ↑/↓ wybór · c skomentuj · r rozwiąż · o otwórz · x usuń · d diff · ? pomoc · Esc powrót"
        } else if width >= 65 {
            "←/→ zakładki · ↑/↓ wybór · c dodaj · r/o/x status · d diff · Esc powrót"
        } else if width >= 45 {
            "←/→ tab · ↑/↓ wybór · c dodaj · r/o/x · Esc"
        } else {
            "c kom · Esc powrót"
        }
    } else {
        if width >= 90 {
            "←/→ zakładka · ↑/↓ PgUp/PgDn przewiń · c skomentuj · d diff · ? pomoc · Esc powrót do grafu"
        } else if width >= 65 {
            "←/→ zakładki · ↑/↓ przewiń · c skomentuj · d diff · Esc powrót"
        } else if width >= 45 {
            "←/→ tab · ↑/↓ przewiń · c kom · d diff · Esc"
        } else {
            "↑/↓ · Esc powrót"
        }
    }
}

pub fn format_tabs<'a>(agent_key: &str, active_tab: usize, max_width: usize) -> Line<'a> {
    let agent = format!(" {} ", agent_name(agent_key));
    let agent_w = agent.len();
    let full_tabs_len: usize = agent_w
        + TABS
            .iter()
            .enumerate()
            .map(|(i, t)| t.len() + if i == active_tab { 4 } else { 3 })
            .sum::<usize>();

    if full_tabs_len <= max_width {
        let mut spans = vec![Span::styled(
            agent,
            Style::default().add_modifier(Modifier::BOLD),
        )];
        for (index, name) in TABS.iter().enumerate() {
            spans.push(Span::raw(" "));
            spans.push(if index == active_tab {
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
        return Line::from(spans);
    }

    // Narrow terminal: ensure active tab is visible with sliding window
    let mut spans = vec![Span::styled(
        agent,
        Style::default().add_modifier(Modifier::BOLD),
    )];

    let mut left = active_tab;
    let mut right = active_tab;
    let mut current_len = agent_w + TABS[active_tab].len() + 4;
    while left > 0 || right + 1 < TABS.len() {
        let mut expanded = false;
        if right + 1 < TABS.len() {
            let next_len = TABS[right + 1].len() + 3;
            if current_len + next_len + 2 <= max_width {
                right += 1;
                current_len += next_len;
                expanded = true;
            }
        }
        if left > 0 {
            let prev_len = TABS[left - 1].len() + 3;
            if current_len + prev_len + 2 <= max_width {
                left -= 1;
                current_len += prev_len;
                expanded = true;
            }
        }
        if !expanded {
            break;
        }
    }

    if left > 0 {
        spans.push(Span::styled("◀ ", dim()));
    }
    for (index, &name) in TABS.iter().enumerate().take(right + 1).skip(left) {
        spans.push(if index == active_tab {
            Span::styled(
                format!("[{name}] "),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::styled(format!("{name} "), dim())
        });
    }
    if right + 1 < TABS.len() {
        spans.push(Span::styled("▶", dim()));
    }

    Line::from(spans)
}

/// The verifier's obligations (the task's risks, then its kind's checklist)
/// and how the latest verdict accounted for each.
fn checks(stored: &Value) -> Vec<(Style, String)> {
    let Ok(spec) = serde_json::from_value::<TaskSpec>(stored["spec"].clone()) else {
        return vec![(dim(), "Nie umiem odczytać zadania.".into())];
    };
    let coverage = &stored["verdict"]["coverage"];
    let mut lines = vec![];
    if stored["verdict"].is_null() {
        lines.push((
            dim(),
            "Weryfikator jeszcze nie ocenił. Ryzyka i lista kontrolna zostaną rozliczone po zakończeniu zadania.".into(),
        ));
        lines.push((Style::default(), String::new()));
    }
    for (id, text) in obligations(&spec) {
        let entry = &coverage[id.as_str()];
        let (mark, style) = match entry["status"]
            .as_str()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("ok") => ("✓", Style::default().fg(Color::Green)),
            Some("issue") => ("✗", Style::default().fg(Color::Red)),
            Some("blocked") => ("!", Style::default().fg(Color::Yellow)),
            Some("n/a") => ("–", dim()),
            _ if stored["verdict"].is_null() => ("○", dim()),
            _ => ("?", Style::default().fg(Color::Yellow)),
        };
        let kind = if id.starts_with('R') {
            "ryzyko"
        } else if id.starts_with('V') {
            "wspólna"
        } else {
            "kontrola"
        };
        lines.push((style, format!("{mark} {id} {kind}: {text}")));
        if let Some(note) = entry["note"].as_str() {
            lines.push((Style::default(), format!("    {note}")));
        } else if !stored["verdict"].is_null() {
            lines.push((dim(), "    nierozliczone".into()));
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn ac_copy_01_polish_tab_names_preserved_and_windowed_when_narrow() {
        // Wide: 100 columns -> all tabs rendered
        let line = format_tabs("orchestrator", 0, 100);
        let rendered: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        for tab in TABS {
            assert!(rendered.contains(tab), "missing tab {tab} in {rendered}");
        }

        // Narrow: 50 columns -> active tab still visible
        let line = format_tabs("worker:t1", 3, 50);
        let rendered: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(
            rendered.contains("[Wynik]"),
            "active tab Wynik missing in {rendered}"
        );
    }

    #[test]
    fn details_footer_hints_prioritized_by_width() {
        // COMMENTS tab
        assert!(details_footer_hint(COMMENTS, 100).contains("d diff"));
        assert!(!details_footer_hint(COMMENTS, 30).contains("d diff"));
        assert!(details_footer_hint(COMMENTS, 30).contains("Esc"));

        // Other tabs
        assert!(details_footer_hint(0, 100).contains("PgUp/PgDn"));
        assert!(!details_footer_hint(0, 40).contains("PgUp/PgDn"));
        assert!(details_footer_hint(0, 40).contains("Esc"));
    }

    #[test]
    fn details_empty_states_in_polish() {
        let view = View::default();
        let mut details = Details::new("worker:t1".into());

        // Tab 1: Sesja empty state
        details.tab = 1;
        let lines = details.lines(&view);
        assert!(
            lines
                .iter()
                .any(|(_, text)| text.contains("Brak zdarzeń sesji"))
        );

        // Tab 2: Wiadomości empty state
        details.tab = 2;
        let lines = details.lines(&view);
        assert!(
            lines
                .iter()
                .any(|(_, text)| text.contains("Brak wiadomości"))
        );

        // Tab 5: Komentarze empty state
        details.tab = COMMENTS;
        let lines = details.lines(&view);
        assert!(
            lines
                .iter()
                .any(|(_, text)| text.contains("Brak komentarzy"))
        );
    }

    #[test]
    fn details_draw_on_tiny_terminals() {
        let view = View::default();
        let mut details = Details::new("orchestrator".into());

        // 60x12 guard floor
        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                details.draw(frame, &view);
            })
            .unwrap();

        // 10x4 tiny terminal (must not panic)
        let backend = TestBackend::new(10, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                details.draw(frame, &view);
            })
            .unwrap();

        // 2x2 extreme tiny terminal (must not panic)
        let backend = TestBackend::new(2, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                details.draw(frame, &view);
            })
            .unwrap();
    }
}
