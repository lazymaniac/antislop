//! Terminal frontend for one conversation: the chat with the orchestrator
//! beside the team graph. It never waits on the daemon in its draw loop: a
//! poller thread long-polls events and every action runs on its own thread,
//! so typing, scrolling and sending stay instant.

mod details;
mod diffview;
mod graph;
mod palette;
pub mod view;

use super::cli::Options;
use super::daemon::{Client, Request};
use crate::Result;
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use details::{CHECKS, COMMENTS, Details, RESULT, TABS};
use diffview::DiffView;
use graph::{Canvas, Filter, Graph};
use palette::{Action, Command, Help, Kind, Picker};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;
use unicode_width::UnicodeWidthStr;
use view::{ORCHESTRATOR, View, dim, target_name, truncate, wrap};

/// Narrower terminals show one panel at a time.
const SIDE_BY_SIDE: u16 = 100;

enum Update {
    Events {
        generation: u64,
        events: Vec<Value>,
        idle: bool,
        autonomous: bool,
    },
    Conversation(String),
    Notice(String),
    /// A task's change, for the diff view of `task`.
    Diff {
        generation: u64,
        task: String,
        reply: std::result::Result<Value, String>,
    },
    /// A task as the daemon stores it, for the details of `key`.
    Task {
        generation: u64,
        key: String,
        task: Value,
    },
}

/// A comment being written on one element.
struct Editor {
    target: String,
    text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Chat,
    Graph,
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
    focus: Focus,
    /// The selected graph node, by key, so it survives new events.
    selected: String,
    filter: Filter,
    /// The graph's top-left corner shown in its panel.
    offset: (u16, u16),
    details: Option<Details>,
    diff: Option<DiffView>,
    editor: Option<Editor>,
    picker: Option<Picker>,
    help: Option<Help>,
}

impl App {
    fn open(&mut self, conversation: String) {
        self.stop_poller.store(true, Ordering::SeqCst);
        self.generation += 1;
        self.conversation = conversation.clone();
        self.view = View::default();
        self.scroll = 0;
        self.details = None;
        self.diff = None;
        self.editor = None;
        self.picker = None;
        self.help = None;
        self.selected = ORCHESTRATOR.into();
        self.offset = (0, 0);
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
                            autonomous: reply["autonomous"] == true,
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

    /// Fetches the stored task behind the open details.
    fn fetch_task(&self) {
        let (Some(details), Some(plan)) = (&self.details, &self.view.plan) else {
            return;
        };
        let Some(id) = details.task_id() else {
            return;
        };
        let request = Request::Task {
            plan: plan.clone(),
            id: id.to_owned(),
        };
        let key = details.key.clone();
        let generation = self.generation;
        let client = self.client.clone();
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            let _ = updates.send(match client.call(&request) {
                Ok(task) => Update::Task {
                    generation,
                    key,
                    task,
                },
                Err(error) => Update::Notice(format!("✗ {error}")),
            });
        });
    }

    fn open_diff(&mut self, key: &str) {
        if let Some((_, task)) = key.split_once(':') {
            self.diff = Some(DiffView::new(task.to_owned()));
            self.fetch_diff();
        }
    }

    fn fetch_diff(&self) {
        let (Some(diff), Some(plan)) = (&self.diff, &self.view.plan) else {
            return;
        };
        let request = Request::Diff {
            conversation: self.conversation.clone(),
            plan: plan.clone(),
            id: diff.task.clone(),
        };
        let task = diff.task.clone();
        let generation = self.generation;
        let client = self.client.clone();
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            let reply = client.call(&request).map_err(|error| error.to_string());
            let _ = updates.send(Update::Diff {
                generation,
                task,
                reply,
            });
        });
    }

    fn diff_key(&mut self, key: event::KeyEvent) {
        let Some(diff) = &mut self.diff else {
            return;
        };
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.diff = None,
            KeyCode::Char('?') => self.help = Some(Help::Diff),
            KeyCode::Char('c') => {
                if let Some(target) = diff.target() {
                    self.editor = Some(Editor {
                        target,
                        text: String::new(),
                    });
                }
            }
            KeyCode::Char('r') => self.fetch_diff(),
            KeyCode::Char('R') => {
                if let Some(plan) = self.view.plan.clone() {
                    let request = Request::Review {
                        conversation: self.conversation.clone(),
                        plan,
                        id: diff.task.clone(),
                    };
                    self.request(request, Some("recenzja kodu uruchomiona"));
                }
            }
            code => diff.key(code),
        }
    }

    fn submit(&mut self) {
        let text = std::mem::take(&mut self.input);
        self.dispatch(text.trim());
    }

    /// Runs a `/command` or sends a message.
    fn dispatch(&mut self, text: &str) {
        let text = text.to_owned();
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
            "/pause" => self.request(
                Request::Pause {
                    conversation,
                    paused: true,
                },
                None,
            ),
            "/continue" => self.request(
                Request::Pause {
                    conversation,
                    paused: false,
                },
                None,
            ),
            "/mode autonomous" | "/mode supervised" => self.request(
                Request::Mode {
                    conversation,
                    autonomous: text.ends_with("autonomous"),
                },
                None,
            ),
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

    /// A command from a shortcut or the palette; the draft stays as it is.
    fn command(&mut self, command: &str) {
        self.dispatch(command);
    }

    fn help_here(&self) -> Help {
        if self.diff.is_some() {
            Help::Diff
        } else if self.details.is_some() {
            Help::Details
        } else if self.focus == Focus::Graph {
            Help::Graph
        } else {
            Help::Chat
        }
    }

    /// Returns false when the user quits.
    fn picker_key(&mut self, key: event::KeyEvent) -> bool {
        let Some(picker) = &mut self.picker else {
            return true;
        };
        match key.code {
            KeyCode::Esc => self.picker = None,
            KeyCode::Up => picker.selected = picker.selected.saturating_sub(1),
            KeyCode::Down => picker.selected += 1,
            KeyCode::Backspace => {
                picker.query.pop();
                picker.selected = 0;
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                picker.query.push(ch);
                picker.selected = 0;
            }
            KeyCode::Enter => {
                let action = picker
                    .items(&self.view)
                    .into_iter()
                    .nth(picker.selected)
                    .map(|(_, action)| action);
                self.picker = None;
                return match action {
                    Some(Action::Run(command)) => self.run(command),
                    Some(Action::Select(key)) => {
                        self.details = None;
                        self.diff = None;
                        self.focus = Focus::Graph;
                        self.selected = key;
                        true
                    }
                    None => true,
                };
            }
            _ => {}
        }
        true
    }

    /// Returns false when the user quits.
    fn run(&mut self, command: Command) -> bool {
        let selected = self.selected.clone();
        match command {
            Command::Accept => self.command("/accept"),
            Command::Resume => self.command("/resume"),
            Command::Cancel => self.command("/cancel"),
            Command::Pause => self.command("/pause"),
            Command::Continue => self.command("/continue"),
            Command::Autonomous => self.command("/mode autonomous"),
            Command::Supervised => self.command("/mode supervised"),
            Command::NewConversation => self.new_conversation(),
            Command::Find => self.picker = Some(Picker::new(Kind::Agents)),
            Command::Details => {
                self.details = Some(Details::new(selected));
                self.fetch_task();
            }
            Command::Diff => self.open_diff(&selected),
            Command::Comment => {
                self.editor = Some(Editor {
                    target: selected,
                    text: String::new(),
                })
            }
            Command::Filter => self.filter = self.filter.next(),
            Command::Help => self.help = Some(self.help_here()),
            Command::Quit => return false,
        }
        true
    }

    /// Returns false when the user quits.
    fn key(&mut self, key: event::KeyEvent) -> bool {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Char('c') if control => return false,
            KeyCode::Char('a') if control => self.command("/accept"),
            KeyCode::Char('x') if control => self.command("/cancel"),
            KeyCode::Char('r') if control => self.command("/resume"),
            KeyCode::Char('n') if control => self.new_conversation(),
            _ if self.help.is_some() => self.help = None,
            _ if self.picker.is_some() => return self.picker_key(key),
            KeyCode::Char('p') if control => self.picker = Some(Picker::new(Kind::Commands)),
            KeyCode::F(1) => self.help = Some(self.help_here()),
            _ if self.editor.is_some() => self.editor_key(key),
            _ if self.diff.is_some() => self.diff_key(key),
            _ if self.details.is_some() => self.details_key(key),
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Chat => Focus::Graph,
                    Focus::Graph => Focus::Chat,
                }
            }
            _ if self.focus == Focus::Graph => return self.graph_key(key),
            KeyCode::Esc => return false,
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

    fn graph_key(&mut self, key: event::KeyEvent) -> bool {
        let graph = Graph::new(&self.view);
        let step = match key.code {
            KeyCode::Left => Some((-1, 0)),
            KeyCode::Right => Some((1, 0)),
            KeyCode::Up => Some((0, -1)),
            KeyCode::Down => Some((0, 1)),
            _ => None,
        };
        if let Some((dx, dy)) = step {
            if graph.node(&self.selected).is_none() {
                self.selected = ORCHESTRATOR.into();
            } else if let Some(next) = graph.neighbour(&self.selected, dx, dy) {
                self.selected = next;
            }
            return true;
        }
        match key.code {
            KeyCode::Esc => self.focus = Focus::Chat,
            KeyCode::Char('q') => return false,
            KeyCode::Char('?') => self.help = Some(Help::Graph),
            KeyCode::Char('/') => self.picker = Some(Picker::new(Kind::Agents)),
            KeyCode::Char('f') => self.filter = self.filter.next(),
            KeyCode::Char('d') => {
                let key = self.selected.clone();
                self.open_diff(&key);
            }
            KeyCode::Char('c') => {
                self.editor = Some(Editor {
                    target: self.selected.clone(),
                    text: String::new(),
                })
            }
            KeyCode::Enter => {
                self.details = Some(Details::new(self.selected.clone()));
                self.fetch_task();
            }
            _ => {}
        }
        true
    }

    fn editor_key(&mut self, key: event::KeyEvent) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        match key.code {
            KeyCode::Esc => self.editor = None,
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::ALT) => editor.text.push('\n'),
            KeyCode::Enter => {
                let editor = self.editor.take().expect("editing");
                if !editor.text.trim().is_empty() {
                    self.request(
                        Request::Comment {
                            conversation: self.conversation.clone(),
                            target: editor.target,
                            text: editor.text,
                        },
                        None,
                    );
                }
            }
            KeyCode::Backspace => {
                editor.text.pop();
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                editor.text.push(ch)
            }
            _ => {}
        }
    }

    fn set_comment(&self, id: u64, status: &str) {
        self.request(
            Request::CommentStatus {
                conversation: self.conversation.clone(),
                id,
                status: status.into(),
            },
            None,
        );
    }

    fn details_key(&mut self, key: event::KeyEvent) {
        let Some(details) = &mut self.details else {
            return;
        };
        let comment = details.selected_comment(&self.view);
        let on_comments = details.tab == COMMENTS;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.details = None,
            KeyCode::Char('?') => self.help = Some(Help::Details),
            KeyCode::Char('c') => {
                self.editor = Some(Editor {
                    target: details.key.clone(),
                    text: String::new(),
                })
            }
            KeyCode::Down if on_comments => details.comment += 1,
            KeyCode::Up if on_comments => details.comment = details.comment.saturating_sub(1),
            KeyCode::Char(ch @ ('r' | 'o' | 'x')) if comment.is_some() => {
                let status = match ch {
                    'r' => "resolved",
                    'o' => "open",
                    _ => "deleted",
                };
                self.set_comment(comment.expect("selected"), status);
            }
            KeyCode::Char('d') => {
                let key = details.key.clone();
                self.open_diff(&key);
                return;
            }
            KeyCode::Right | KeyCode::Tab => {
                details.tab = (details.tab + 1) % TABS.len();
                details.scroll = 0;
            }
            KeyCode::Left | KeyCode::BackTab => {
                details.tab = (details.tab + TABS.len() - 1) % TABS.len();
                details.scroll = 0;
            }
            KeyCode::Down => details.scroll += 1,
            KeyCode::Up => details.scroll = details.scroll.saturating_sub(1),
            KeyCode::PageDown => details.scroll += 20,
            KeyCode::PageUp => details.scroll = details.scroll.saturating_sub(20),
            KeyCode::Home => details.scroll = 0,
            KeyCode::End => details.scroll = usize::MAX,
            _ => {}
        }
        if self
            .details
            .as_ref()
            .is_some_and(|details| details.tab == RESULT || details.tab == CHECKS)
        {
            self.fetch_task();
        }
    }

    fn draw(&mut self, frame: &mut Frame) {
        if let Some(diff) = &mut self.diff {
            diff.draw(frame, &self.view);
        } else if let Some(details) = &mut self.details {
            details.draw(frame, &self.view);
        } else {
            self.draw_main(frame);
        }
        if let Some(editor) = &self.editor {
            let area = frame.area();
            let width = area.width.saturating_sub(2);
            let lines = wrap(&editor.text, usize::from(width.max(1)));
            let height = (lines.len() as u16).clamp(1, 6) + 2;
            if area.height < height + 1 {
                return;
            }
            let shown = lines.len().saturating_sub(6);
            let box_area = Rect::new(area.x, area.bottom() - 1 - height, area.width, height);
            frame.render_widget(Clear, box_area);
            frame.render_widget(
                Paragraph::new(
                    lines[shown..]
                        .iter()
                        .map(|line| Line::raw(line.clone()))
                        .collect::<Vec<_>>(),
                )
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Cyan))
                        .title(format!(
                            " komentarz do {} · Enter wyślij · Alt+Enter nowa linia · Esc anuluj ",
                            target_name(&editor.target)
                        )),
                ),
                box_area,
            );
            let last = lines.last().map_or(0, |line| line.width());
            frame.set_cursor_position((
                box_area.x + 1 + last as u16,
                box_area.y + lines[shown..].len() as u16,
            ));
        }
        if let Some(picker) = &mut self.picker {
            picker.draw(frame, &self.view);
        }
        if let Some(help) = self.help {
            help.draw(frame);
        }
    }

    fn draw_main(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .areas(area);

        let status = match (&self.view.activity, self.view.idle) {
            (Some(activity), _) => activity.clone(),
            (None, _) if self.view.paused => "wstrzymane".into(),
            (None, true) => "gotowy".into(),
            (None, false) => "zadania w toku…".into(),
        };
        let short = self.conversation.get(..8).unwrap_or(&self.conversation);
        let mode = match self.view.autonomous {
            Some(true) => "autonomiczny · ",
            Some(false) => "nadzorowany · ",
            None => "",
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" antislop ", Style::default().add_modifier(Modifier::BOLD)),
                Span::styled(format!("{} · {short} · {mode}", self.project), dim()),
                Span::styled(status, Style::default().fg(Color::Yellow)),
            ])),
            header,
        );

        if body.width >= SIDE_BY_SIDE {
            let [chat, team] =
                Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)])
                    .areas(body);
            self.draw_chat(frame, chat);
            self.draw_graph(frame, team);
        } else if self.focus == Focus::Chat {
            self.draw_chat(frame, body);
        } else {
            self.draw_graph(frame, body);
        }

        let hint = self.notice.clone().unwrap_or_else(|| {
            let plan = if self.view.plan_proposed() {
                "Ctrl+A uruchom plan · "
            } else if self.view.plan_resumable() {
                "Ctrl+R wznów plan · "
            } else {
                ""
            };
            match self.focus {
                Focus::Chat => format!(
                    "Enter wyślij (@t2 … do agenta) · Alt+Enter nowa linia · {plan}Ctrl+P polecenia · F1 pomoc · Tab zespół · Esc wyjdź"
                ),
                Focus::Graph => format!(
                    "strzałki wybór · Enter szczegóły · / znajdź · d diff · c skomentuj · f filtr · {plan}Ctrl+P polecenia · ? pomoc · q wyjdź"
                ),
            }
        });
        frame.render_widget(Paragraph::new(Span::styled(hint, dim())), footer);
    }

    fn draw_chat(&mut self, frame: &mut Frame, area: Rect) {
        let input_width = area.width.saturating_sub(2) as usize;
        let input_lines = wrap(&self.input, input_width.max(1));
        let input_height = (input_lines.len() as u16).clamp(1, 6) + 2;
        let [transcript, input] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(input_height)]).areas(area);
        self.draw_transcript(frame, transcript);

        let shown = input_lines.len().saturating_sub(6);
        let focused = self.focus == Focus::Chat;
        frame.render_widget(
            Paragraph::new(
                input_lines[shown..]
                    .iter()
                    .map(|line| Line::raw(line.clone()))
                    .collect::<Vec<_>>(),
            )
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(if focused { Style::default() } else { dim() })
                    .title(" wiadomość "),
            ),
            input,
        );
        if focused {
            let last = input_lines.last().map_or(0, |line| line.width());
            frame.set_cursor_position((
                input.x + 1 + last as u16,
                input.y + input_lines[shown..].len() as u16,
            ));
        }
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

    fn draw_graph(&mut self, frame: &mut Frame, area: Rect) {
        let focused = self.focus == Focus::Graph;
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(if focused {
                Style::default().fg(Color::Cyan)
            } else {
                dim()
            })
            .title(format!(" zespół · {} (f) ", self.filter.name()));
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let graph = Graph::new(&self.view);
        if graph.node(&self.selected).is_none() {
            self.selected = ORCHESTRATOR.into();
        }
        // Scroll only as far as needed to keep the selected node in view.
        if let Some(node) = graph.node(&self.selected) {
            let (left, top) = &mut self.offset;
            if node.x < *left {
                *left = node.x.saturating_sub(3);
            } else if node.x + graph::WIDTH + 3 > *left + inner.width {
                *left = (node.x + graph::WIDTH + 3).saturating_sub(inner.width);
            }
            if node.y < *top {
                *top = node.y;
            } else if node.y + graph::HEIGHT > *top + inner.height {
                *top = (node.y + graph::HEIGHT).saturating_sub(inner.height);
            }
        }
        let mut canvas = Canvas::new(graph.width, graph.height);
        graph.render(&self.view, &mut canvas, self.filter, &self.selected);
        canvas.show(frame.buffer_mut(), inner, self.offset.0, self.offset.1);

        // What is happening now, in a corner that leaves the selection free.
        let progress = self.view.progress();
        let width = progress
            .iter()
            .map(|line| line.width() as u16 + 4)
            .max()
            .unwrap_or(0)
            .clamp(16, 48)
            .min(inner.width);
        let height = (progress.len() as u16).min(6) + 2;
        if inner.height < height + 2 {
            return;
        }
        let x = inner.right() - width;
        let mut overlay = Rect::new(x, inner.bottom() - height, width, height);
        if let Some(node) = graph.node(&self.selected) {
            let shown = Rect::new(
                inner.x + node.x.saturating_sub(self.offset.0),
                inner.y + node.y.saturating_sub(self.offset.1),
                graph::WIDTH,
                graph::HEIGHT,
            );
            if shown.intersects(overlay) {
                overlay.y = inner.y;
            }
        }
        let lines: Vec<Line> = progress
            .iter()
            .take(6)
            .map(|line| Line::raw(truncate(line, usize::from(width.saturating_sub(4)))))
            .collect();
        frame.render_widget(Clear, overlay);
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Yellow))
                    .title(" teraz ")
                    .padding(ratatui::widgets::Padding::horizontal(1)),
            ),
            overlay,
        );
    }
}

/// Drops every color; what was dim stays dim, selection is reversed anyway.
fn without_color(buffer: &mut ratatui::buffer::Buffer) {
    for cell in &mut buffer.content {
        if cell.fg == Color::DarkGray {
            cell.modifier.insert(Modifier::DIM);
        }
        cell.set_fg(Color::Reset).set_bg(Color::Reset);
    }
}

pub fn run(options: &Options, client: Client) -> Result<()> {
    let conversation = options.conversation(&client)?;
    let (updates, received) = mpsc::channel();
    let mut app = App {
        client: client.clone(),
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
        notice: client.notice.clone(),
        updates,
        focus: Focus::Chat,
        selected: ORCHESTRATOR.into(),
        filter: Filter::All,
        offset: (0, 0),
        details: None,
        diff: None,
        editor: None,
        picker: None,
        help: None,
    };
    app.open(conversation);

    // https://no-color.org: nothing depends on color alone (design §17).
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
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
                        autonomous,
                    } if generation == app.generation => {
                        for event in &events {
                            app.view.apply(event);
                        }
                        app.view.idle = idle;
                        app.view.autonomous = Some(autonomous);
                    }
                    Update::Events { .. } => {}
                    Update::Conversation(id) => app.open(id),
                    Update::Notice(text) => app.notice = Some(text),
                    Update::Task {
                        generation,
                        key,
                        task,
                    } if generation == app.generation => {
                        if let Some(details) =
                            app.details.as_mut().filter(|details| details.key == key)
                        {
                            details.task = Some(task);
                        }
                    }
                    Update::Task { .. } => {}
                    Update::Diff {
                        generation,
                        task,
                        reply,
                    } if generation == app.generation => {
                        if let Some(diff) = app.diff.as_mut().filter(|diff| diff.task == task) {
                            diff.load(reply);
                        }
                    }
                    Update::Diff { .. } => {}
                }
            }
            terminal.draw(|frame| {
                app.draw(frame);
                if no_color {
                    without_color(frame.buffer_mut());
                }
            })?;
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
                Event::Paste(text) => {
                    let text = text.replace('\r', "\n");
                    if let Some(picker) = &mut app.picker {
                        picker.query.push_str(text.trim());
                    } else if let Some(editor) = &mut app.editor {
                        editor.text.push_str(&text);
                    } else if app.focus == Focus::Chat && app.details.is_none() {
                        app.input.push_str(&text);
                    }
                }
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
    use ratatui::buffer::Buffer;

    #[test]
    fn without_color_keeps_dim_and_reversed_as_modifiers() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 3, 1));
        buffer.set_string(0, 0, "a", dim());
        buffer.set_string(1, 0, "b", Style::default().fg(Color::Red).bg(Color::Blue));
        buffer.set_string(2, 0, "c", Style::default().add_modifier(Modifier::REVERSED));
        without_color(&mut buffer);
        assert!(
            buffer
                .content
                .iter()
                .all(|cell| cell.fg == Color::Reset && cell.bg == Color::Reset)
        );
        assert!(buffer.content[0].modifier.contains(Modifier::DIM));
        assert!(!buffer.content[1].modifier.contains(Modifier::DIM));
        assert!(buffer.content[2].modifier.contains(Modifier::REVERSED));
    }
}
