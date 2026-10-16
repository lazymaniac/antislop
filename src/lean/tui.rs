//! Terminal frontend for one conversation. It never waits on the daemon in
//! its draw loop: a poller thread long-polls events and every action runs on
//! its own thread, so typing, scrolling and sending stay instant.

use super::cli::Options;
use super::daemon::{Client, Request};
use crate::Result;
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub style: Style,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskRow {
    pub id: String,
    pub title: String,
    pub status: String,
    pub round: u64,
    pub activity: String,
}

/// What the screen shows, built only from events.
#[derive(Debug, Default)]
pub struct View {
    pub transcript: Vec<Entry>,
    pub tasks: Vec<TaskRow>,
    /// The plan the task rows show.
    pub plan: Option<String>,
    /// The orchestrator's current activity, while it works.
    pub activity: Option<String>,
    pub idle: bool,
    pub after: u64,
    roles: HashMap<String, String>,
    /// Task row index by the task's database id.
    task_rows: HashMap<String, usize>,
}

const ORCHESTRATOR: &str = "orchestrator";

fn dim() -> Style {
    Style::default().fg(Color::DarkGray)
}

impl View {
    fn push(&mut self, style: Style, text: impl Into<String>) {
        self.transcript.push(Entry {
            style,
            text: clean(&text.into()),
        });
    }

    /// Whether the plan ended with tasks that can run again.
    pub fn plan_resumable(&self) -> bool {
        self.tasks
            .iter()
            .all(|task| super::tasks::status::finished(&task.status))
            && self.tasks.iter().any(|task| {
                matches!(
                    task.status.as_str(),
                    "cancelled" | "failed" | "needs_user" | "skipped" | "waiting"
                )
            })
    }

    pub fn plan_proposed(&self) -> bool {
        !self.tasks.is_empty() && self.tasks.iter().all(|task| task.status == "proposed")
    }

    pub fn apply(&mut self, event: &Value) {
        self.after = event["seq"].as_u64().unwrap_or(self.after);
        let payload = &event["payload"];
        let text = |key: &str| payload[key].as_str().unwrap_or_default().to_owned();
        let turn = event["turn_id"].as_str().unwrap_or_default();
        let role = self.roles.get(turn).cloned().unwrap_or_default();
        match event["kind"].as_str().unwrap_or_default() {
            "message" => match payload["author"].as_str().unwrap_or_default() {
                "user" => {
                    self.push(
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                        "Ty",
                    );
                    self.push(Style::default(), text("text"));
                    self.push(Style::default(), "");
                }
                "runtime" => {
                    self.push(dim(), "· wyniki zadań przekazane orkiestratorowi");
                    self.push(Style::default(), "");
                }
                author => {
                    self.push(
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                        match author {
                            ORCHESTRATOR => "orkiestrator".to_owned(),
                            "plan-verifier" => "weryfikator planu".to_owned(),
                            author => author.to_owned(),
                        },
                    );
                    self.push(Style::default(), text("text"));
                    self.push(Style::default(), "");
                }
            },
            "turn_started" => {
                let role = text("role");
                if role == ORCHESTRATOR {
                    self.activity = Some(format!("orkiestrator ({}) pracuje…", text("provider")));
                } else if let Some(row) = self.row(&role) {
                    let who = role.split(':').next().unwrap_or_default();
                    row.activity = format!("{who} ({})", text("provider"));
                }
                self.roles.insert(turn.to_owned(), role);
            }
            "agent" if payload["type"] == "tool" => {
                let detail = format!("{} {}", text("name"), text("detail"));
                if role == ORCHESTRATOR {
                    self.activity = Some(format!("orkiestrator: {detail}"));
                    self.push(dim(), format!("  ↳ {detail}"));
                } else if let Some(row) = self.row(&role) {
                    let who = role.split(':').next().unwrap_or_default();
                    row.activity = format!("{who}: {detail}");
                }
            }
            "turn_finished" => {
                let failed = payload["status"] != "done";
                if role == ORCHESTRATOR {
                    self.activity = None;
                    if failed {
                        self.push(
                            Style::default().fg(Color::Red),
                            format!("✗ orkiestrator: {} — {}", text("status"), text("error")),
                        );
                    }
                } else if let Some(row) = self.row(&role) {
                    row.activity = if failed {
                        format!("✗ {}", text("error"))
                    } else {
                        String::new()
                    };
                }
            }
            "plan_proposed" => {
                self.plan = payload["plan"].as_str().map(str::to_owned);
                self.tasks = payload["tasks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|task| TaskRow {
                        id: task["id"].as_str().unwrap_or_default().to_owned(),
                        title: task["title"].as_str().unwrap_or_default().to_owned(),
                        status: "proposed".into(),
                        round: 0,
                        activity: if task["writes"] == true {
                            "zmienia pliki".into()
                        } else {
                            String::new()
                        },
                    })
                    .collect();
                self.task_rows.clear();
                for line in super::cli::review_lines("weryfikator planu", &payload["review"]) {
                    self.push(Style::default().fg(Color::Yellow), line);
                }
                let next = if payload["automatic"] == true {
                    "uruchamiam (tryb autonomiczny).".to_owned()
                } else {
                    let waiting = payload["waiting"]
                        .as_str()
                        .map(|reason| format!("Czeka na Ciebie: {reason}. "))
                        .unwrap_or_default();
                    format!("{waiting}Ctrl+A uruchamia, albo napisz, co zmienić.")
                };
                self.push(
                    Style::default().fg(Color::Yellow),
                    format!("Plan: {} zadań. {next}", self.tasks.len()),
                );
                self.push(Style::default(), "");
            }
            "audit" => {
                for line in super::cli::review_lines("audyt", payload) {
                    self.push(Style::default().fg(Color::Yellow), line);
                }
            }
            "plan_invalid" => self.push(
                Style::default().fg(Color::Red),
                format!("✗ plan odrzucony: {}", text("error")),
            ),
            "task" => {
                let id = text("id");
                let index = self.tasks.iter().position(|row| row.id == id);
                if let Some(index) = index {
                    self.task_rows.insert(text("task"), index);
                    let row = &mut self.tasks[index];
                    row.status = text("status");
                    row.round = payload["round"].as_u64().unwrap_or_default();
                    if super::tasks::status::finished(&row.status) {
                        row.activity.clear();
                    }
                }
            }
            "integration" => self.push(
                if payload["status"] == "approved" {
                    dim()
                } else {
                    Style::default().fg(Color::Yellow)
                },
                format!("· [{}] {}: {}", text("id"), text("title"), text("detail")),
            ),
            _ => {}
        }
    }

    fn row(&mut self, role: &str) -> Option<&mut TaskRow> {
        let task = role.split_once(':')?.1;
        let index = *self.task_rows.get(task)?;
        self.tasks.get_mut(index)
    }
}

/// Agents' text may carry terminal escapes; keep it printable.
fn clean(text: &str) -> String {
    text.chars()
        .filter_map(|ch| match ch {
            '\n' => Some('\n'),
            '\t' => Some(' '),
            ch if ch.is_control() => None,
            ch => Some(ch),
        })
        .collect()
}

/// Word-wraps `text` to `width` columns, breaking long words.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = vec![];
    for paragraph in text.split('\n') {
        let mut line = String::new();
        let mut used = 0;
        for word in paragraph.split_inclusive(' ') {
            let size = word.width();
            if used > 0 && used + size.min(width) > width {
                lines.push(std::mem::take(&mut line));
                used = 0;
            }
            for ch in word.chars() {
                let size = ch.width().unwrap_or(0);
                if used + size > width {
                    lines.push(std::mem::take(&mut line));
                    used = 0;
                }
                line.push(ch);
                used += size;
            }
        }
        lines.push(line);
    }
    lines
}

enum Update {
    Events {
        generation: u64,
        events: Vec<Value>,
        idle: bool,
    },
    Conversation(String),
    Notice(String),
}

struct App {
    client: Client,
    project: String,
    conversation: String,
    generation: u64,
    stop_poller: Arc<AtomicBool>,
    view: View,
    input: String,
    /// Lines scrolled up from the bottom of the transcript.
    scroll: usize,
    notice: Option<String>,
    updates: mpsc::Sender<Update>,
}

impl App {
    fn open(&mut self, conversation: String) {
        self.stop_poller.store(true, Ordering::SeqCst);
        self.generation += 1;
        self.conversation = conversation.clone();
        self.view = View::default();
        self.scroll = 0;
        let stop = Arc::new(AtomicBool::new(false));
        self.stop_poller = stop.clone();
        let client = self.client.clone();
        let updates = self.updates.clone();
        let generation = self.generation;
        std::thread::spawn(move || {
            let mut after = 0;
            let mut wait_ms = 0;
            while !stop.load(Ordering::SeqCst) {
                let reply = client.call(&Request::Events {
                    conversation: conversation.clone(),
                    after,
                    wait_ms,
                });
                let update = match reply {
                    Ok(reply) => {
                        let events = reply["events"].as_array().cloned().unwrap_or_default();
                        wait_ms = if events.is_empty() { 10_000 } else { 0 };
                        if let Some(last) = events.last() {
                            after = last["seq"].as_u64().unwrap_or(after);
                        }
                        Update::Events {
                            generation,
                            events,
                            idle: reply["idle"] == true,
                        }
                    }
                    Err(error) => {
                        std::thread::sleep(Duration::from_secs(1));
                        Update::Notice(format!("demon nie odpowiada: {error}"))
                    }
                };
                if stop.load(Ordering::SeqCst) || updates.send(update).is_err() {
                    return;
                }
            }
        });
    }

    /// Runs a daemon request off the draw loop; its error becomes a notice.
    fn request(&self, request: Request, done: Option<&'static str>) {
        let client = self.client.clone();
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            let notice = match client.call(&request) {
                Ok(_) => done.map(str::to_owned),
                Err(error) => Some(format!("✗ {error}")),
            };
            if let Some(notice) = notice {
                let _ = updates.send(Update::Notice(notice));
            }
        });
    }

    fn submit(&mut self) {
        let text = self.input.trim().to_owned();
        self.input.clear();
        let conversation = self.conversation.clone();
        match text.as_str() {
            "" => {}
            "/accept" => match self.view.plan.clone().filter(|_| self.view.plan_proposed()) {
                Some(plan) => self.request(
                    Request::Accept { conversation, plan },
                    Some("plan uruchomiony"),
                ),
                None => self.notice = Some("✗ nie ma planu do uruchomienia".into()),
            },
            "/resume" => match self
                .view
                .plan
                .clone()
                .filter(|_| self.view.plan_resumable())
            {
                Some(plan) => self.request(
                    Request::Resume { conversation, plan },
                    Some("plan wznowiony"),
                ),
                None => self.notice = Some("✗ nie ma czego wznowić".into()),
            },
            "/cancel" => self.request(Request::Cancel { conversation }, Some("anulowano")),
            _ => {
                self.scroll = 0;
                self.request(Request::Send { conversation, text }, None);
            }
        }
    }

    fn new_conversation(&self) {
        let client = self.client.clone();
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            let update = match client.call(&Request::Start) {
                Ok(conversation) => {
                    Update::Conversation(conversation["id"].as_str().unwrap_or_default().into())
                }
                Err(error) => Update::Notice(format!("✗ {error}")),
            };
            let _ = updates.send(update);
        });
    }

    /// Returns false when the user quits.
    fn key(&mut self, key: event::KeyEvent) -> bool {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => return false,
            KeyCode::Char('c') if control => return false,
            KeyCode::Char('a') if control => {
                self.input = "/accept".into();
                self.submit();
            }
            KeyCode::Char('x') if control => {
                self.input = "/cancel".into();
                self.submit();
            }
            KeyCode::Char('r') if control => {
                self.input = "/resume".into();
                self.submit();
            }
            KeyCode::Char('n') if control => self.new_conversation(),
            KeyCode::Char('u') if control => self.input.clear(),
            KeyCode::Char('j') if control => self.input.push('\n'),
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => self.input.push('\n'),
            KeyCode::Enter => self.submit(),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::PageUp => self.scroll += 10,
            KeyCode::PageDown => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::Up => self.scroll += 1,
            KeyCode::Down => self.scroll = self.scroll.saturating_sub(1),
            KeyCode::End => self.scroll = 0,
            KeyCode::Char(ch) if !control => self.input.push(ch),
            _ => {}
        }
        true
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let input_width = area.width.saturating_sub(2) as usize;
        let input_lines = wrap(&self.input, input_width.max(1));
        let input_height = (input_lines.len() as u16).clamp(1, 6) + 2;
        let [header, body, input, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(input_height),
            Constraint::Length(1),
        ])
        .areas(area);

        let status = match (&self.view.activity, self.view.idle) {
            (Some(activity), _) => activity.clone(),
            (None, true) => "gotowy".into(),
            (None, false) => "zadania w toku…".into(),
        };
        let short = self.conversation.get(..8).unwrap_or(&self.conversation);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" antislop ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(format!("{} · {short} · ", self.project), dim()),
                Span::styled(status, Style::default().fg(Color::Yellow)),
            ])),
            header,
        );

        let transcript = if self.view.tasks.is_empty() || body.width < 80 {
            body
        } else {
            let [left, right] =
                Layout::horizontal([Constraint::Min(40), Constraint::Length(46)]).areas(body);
            self.draw_tasks(frame, right);
            left
        };
        self.draw_transcript(frame, transcript);

        let shown = input_lines.len().saturating_sub(6);
        frame.render_widget(
            Paragraph::new(
                input_lines[shown..]
                    .iter()
                    .map(|line| Line::raw(line.clone()))
                    .collect::<Vec<_>>(),
            )
            .block(Block::default().borders(Borders::ALL).title(" wiadomość ")),
            input,
        );
        let last = input_lines.last().map_or(0, |line| line.width());
        frame.set_cursor_position((
            input.x + 1 + last as u16,
            input.y + input_lines[shown..].len() as u16,
        ));

        let hint = self.notice.clone().unwrap_or_else(|| {
            let accept = if self.view.plan_proposed() {
                "Ctrl+A uruchom plan · "
            } else if self.view.plan_resumable() {
                "Ctrl+R wznów plan · "
            } else {
                ""
            };
            format!(
                "Enter wyślij · Alt+Enter nowa linia · {accept}Ctrl+X anuluj · PgUp/PgDn przewiń · Ctrl+N nowa rozmowa · Esc wyjdź"
            )
        });
        frame.render_widget(Paragraph::new(Span::styled(hint, dim())), footer);
    }

    fn draw_transcript(&mut self, frame: &mut Frame, area: Rect) {
        let width = area.width.saturating_sub(1) as usize;
        let lines: Vec<Line> = self
            .view
            .transcript
            .iter()
            .flat_map(|entry| {
                wrap(&entry.text, width)
                    .into_iter()
                    .map(|line| Line::styled(line, entry.style))
            })
            .collect();
        let height = area.height as usize;
        self.scroll = self.scroll.min(lines.len().saturating_sub(height));
        let end = lines.len() - self.scroll;
        let start = end.saturating_sub(height);
        frame.render_widget(Paragraph::new(lines[start..end].to_vec()), area);
    }

    fn draw_tasks(&self, frame: &mut Frame, area: Rect) {
        let width = area.width.saturating_sub(3) as usize;
        let mut lines = vec![];
        for task in &self.view.tasks {
            let (mark, color) = match task.status.as_str() {
                "approved" => ("✓", Color::Green),
                "needs_user" => ("!", Color::Yellow),
                "failed" => ("✗", Color::Red),
                "skipped" | "cancelled" => ("–", Color::DarkGray),
                "working" | "verifying" => ("●", Color::Cyan),
                _ => ("○", Color::Gray),
            };
            let round = if task.round > 1 {
                format!(" r{}", task.round)
            } else {
                String::new()
            };
            lines.push(Line::from(vec![
                Span::styled(format!("{mark} "), Style::default().fg(color)),
                Span::raw(truncate(&format!("{} {}", task.id, task.title), width)),
            ]));
            let state = format!("{}{round}", polish(&task.status));
            lines.push(Line::styled(
                truncate(
                    &if task.activity.is_empty() {
                        format!("  {state}")
                    } else {
                        format!("  {state} · {}", task.activity)
                    },
                    width,
                ),
                dim(),
            ));
        }
        frame.render_widget(
            Paragraph::new(lines).block(Block::default().borders(Borders::LEFT).title(" zadania ")),
            area,
        );
    }
}

fn polish(status: &str) -> &str {
    match status {
        "proposed" => "proponowane",
        "queued" => "w kolejce",
        "working" => "worker pracuje",
        "verifying" => "weryfikacja",
        "approved" => "zatwierdzone",
        "needs_user" => "wymaga decyzji",
        "failed" => "nieudane",
        "skipped" => "pominięte",
        "cancelled" => "anulowane",
        other => other,
    }
}

fn truncate(text: &str, width: usize) -> String {
    let text = clean(text).replace('\n', " ");
    if text.width() <= width {
        return text;
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let size = ch.width().unwrap_or(0);
        if used + size + 1 > width {
            break;
        }
        out.push(ch);
        used += size;
    }
    out.push('…');
    out
}

pub fn run(options: &Options, client: Client) -> Result<()> {
    let conversation = options.conversation(&client)?;
    let (updates, received) = mpsc::channel();
    let mut app = App {
        client,
        project: options
            .project
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
        conversation: String::new(),
        generation: 0,
        stop_poller: Arc::new(AtomicBool::new(false)),
        view: View::default(),
        input: options.message.clone().unwrap_or_default(),
        scroll: 0,
        notice: None,
        updates,
    };
    app.open(conversation);

    let mut terminal = ratatui::init();
    let _ = crossterm::execute!(std::io::stdout(), EnableBracketedPaste);
    let result = (|| -> Result<()> {
        loop {
            while let Ok(update) = received.try_recv() {
                match update {
                    Update::Events {
                        generation,
                        events,
                        idle,
                    } if generation == app.generation => {
                        for event in &events {
                            app.view.apply(event);
                        }
                        app.view.idle = idle;
                    }
                    Update::Events { .. } => {}
                    Update::Conversation(id) => app.open(id),
                    Update::Notice(text) => app.notice = Some(text),
                }
            }
            terminal.draw(|frame| app.draw(frame))?;
            if !event::poll(Duration::from_millis(100))? {
                continue;
            }
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    app.notice = None;
                    if !app.key(key) {
                        return Ok(());
                    }
                }
                Event::Paste(text) => app.input.push_str(&text.replace('\r', "\n")),
                _ => {}
            }
        }
    })();
    app.stop_poller.store(true, Ordering::SeqCst);
    let _ = crossterm::execute!(std::io::stdout(), DisableBracketedPaste);
    ratatui::restore();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(seq: u64, turn: Option<&str>, kind: &str, payload: Value) -> Value {
        json!({"seq": seq, "turn_id": turn, "kind": kind, "payload": payload})
    }

    #[test]
    fn the_view_follows_a_conversation_and_its_tasks() {
        let mut view = View::default();
        for event in [
            event(
                1,
                None,
                "message",
                json!({"author": "user", "text": "analiza"}),
            ),
            event(
                2,
                Some("o1"),
                "turn_started",
                json!({"role": "orchestrator", "provider": "codex"}),
            ),
            event(
                3,
                Some("o1"),
                "agent",
                json!({"type": "tool", "name": "exec", "detail": "ls"}),
            ),
            event(
                4,
                None,
                "plan_proposed",
                json!({"tasks": [{"id": "t1", "title": "Store", "writes": false}]}),
            ),
            event(
                5,
                None,
                "task",
                json!({"id": "t1", "task": "uuid1", "title": "Store", "status": "working", "round": 1}),
            ),
            event(
                6,
                Some("w1"),
                "turn_started",
                json!({"role": "worker:uuid1", "provider": "agy"}),
            ),
            event(
                7,
                Some("w1"),
                "agent",
                json!({"type": "tool", "name": "view_file", "detail": "src/x.rs"}),
            ),
        ] {
            view.apply(&event);
        }
        assert_eq!(view.after, 7);
        assert_eq!(view.activity.as_deref(), Some("orkiestrator: exec ls"));
        assert_eq!(view.tasks[0].status, "working");
        assert_eq!(view.tasks[0].activity, "worker: view_file src/x.rs");
        assert!(!view.plan_proposed());
        view.apply(&event(
            8,
            Some("o1"),
            "turn_finished",
            json!({"status": "failed", "error": "limit"}),
        ));
        assert_eq!(view.activity, None);
        assert!(view.transcript.last().unwrap().text.contains("limit"));
    }

    #[test]
    fn reviews_and_audits_are_shown_with_their_findings() {
        let mut view = View::default();
        view.apply(&event(
            1,
            None,
            "plan_proposed",
            json!({"plan": "p1", "tasks": [{"id": "t1", "title": "A"}],
                   "review": {"approved": false, "error": "login required"},
                   "automatic": false, "waiting": "the plan verifier did not approve this plan"}),
        ));
        view.apply(&event(
            2,
            None,
            "audit",
            json!({"plan": "p1", "approved": false,
                   "findings": [{"severity": "high", "issue": "README missing"}]}),
        ));
        let text: Vec<&str> = view
            .transcript
            .iter()
            .map(|entry| entry.text.as_str())
            .collect();
        assert!(text.contains(&"⚠ weryfikator planu nie zadziałał: login required"));
        assert!(
            text.iter()
                .any(|line| line.contains("Czeka na Ciebie") && line.contains("Ctrl+A"))
        );
        assert!(text.contains(&"⚠ audyt: zastrzeżenia"));
        assert!(text.contains(&"  - [high] README missing"));
        assert!(view.plan_proposed());
    }

    #[test]
    fn wrapping_breaks_words_and_long_runs() {
        assert_eq!(wrap("ala ma kota", 7), ["ala ma ", "kota"]);
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("a\n\nb", 10), ["a", "", "b"]);
        assert_eq!(clean("a\u{1b}[31mb\tc"), "a[31mb c");
    }
}
