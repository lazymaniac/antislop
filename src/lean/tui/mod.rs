//! Terminal frontend for one conversation: the chat with the orchestrator
//! beside the team graph. It never waits on the daemon in its draw loop: a
//! poller thread long-polls events and every action runs on its own thread,
//! so typing, scrolling and sending stay instant.

pub mod details;
pub mod diffview;
pub mod editor;
pub mod graph;
pub mod palette;
pub mod view;

pub use details::Details;
pub use diffview::DiffView;
pub use editor::{InputBuffer, sanitize_paste, sanitize_single_line_paste, visual_text_lines};
pub use graph::Filter;
pub use palette::{Help, Kind, Picker};

use super::cli::Options;
use super::daemon::{Client, Request};
use crate::Result;
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use details::{CHECKS, COMMENTS, RESULT, TABS};
use graph::{Canvas, Graph};
use palette::{Action, Command};
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
use view::{
    ORCHESTRATOR, PanelFocus, SIDE_BY_SIDE_BREAKPOINT, View, dim, draw_min_term_guard,
    main_footer_hint, render_header, target_name, transcript_empty_state, truncate, wrap,
};

pub enum Update {
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
    SendResult {
        generation: u64,
        sent_text: String,
        result: std::result::Result<(), String>,
    },
    CommentResult {
        generation: u64,
        editor: Editor,
        result: std::result::Result<(), String>,
    },
}

/// A comment being written on one element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editor {
    pub target: String,
    pub input: InputBuffer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Chat,
    Graph,
}

pub struct App {
    pub client: Client,
    pub project: String,
    pub conversation: String,
    pub generation: u64,
    pub stop_poller: Arc<AtomicBool>,
    pub view: View,
    pub input: InputBuffer,
    pub input_shown: usize,
    pub input_width: usize,
    pub editor_width: usize,
    /// Lines scrolled up from the bottom of the transcript.
    pub scroll: usize,
    /// The transcript entry a search led to: shown at the top, highlighted
    /// until the next key.
    pub found: Option<usize>,
    pub notice: Option<String>,
    pub updates: mpsc::Sender<Update>,
    pub focus: Focus,
    /// The selected graph node, by key, so it survives new events.
    pub selected: String,
    pub filter: Filter,
    /// The graph's top-left corner shown in its panel.
    pub offset: (u16, u16),
    pub details: Option<Details>,
    pub diff: Option<DiffView>,
    pub editor: Option<Editor>,
    pub picker: Option<Picker>,
    pub help: Option<Help>,
    pub help_scroll: usize,
    pub terminal_area: Rect,
    pub history: Vec<String>,
    pub history_index: Option<usize>,
    pub history_draft: String,
    pub sending: bool,
    pub sending_comment: bool,
    pub pending_send: Option<String>,
    pub pending_comment: Option<Editor>,
    pub conversation_drafts: std::collections::HashMap<String, String>,
    pub comment_drafts: std::collections::HashMap<String, String>,
}

impl App {
    pub fn test_app() -> App {
        let (updates, _rx) = mpsc::channel();
        Self::test_app_with_client_and_updates(
            Client::at(std::path::Path::new("/dev/null")),
            updates,
        )
    }

    pub fn test_app_with_client(client: Client) -> (App, mpsc::Receiver<Update>) {
        let (updates, rx) = mpsc::channel();
        (Self::test_app_with_client_and_updates(client, updates), rx)
    }

    pub fn test_app_with_client_and_updates(client: Client, updates: mpsc::Sender<Update>) -> App {
        App {
            client,
            project: "test".into(),
            conversation: "test-conv".into(),
            generation: 1,
            stop_poller: Arc::new(AtomicBool::new(false)),
            view: View::default(),
            input: InputBuffer::new(),
            input_shown: 0,
            input_width: 80,
            editor_width: 80,
            scroll: 0,
            found: None,
            notice: None,
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
            help_scroll: 0,
            terminal_area: Rect::new(0, 0, 80, 24),
            history: vec![],
            history_index: None,
            history_draft: String::new(),
            sending: false,
            sending_comment: false,
            pending_send: None,
            pending_comment: None,
            conversation_drafts: std::collections::HashMap::new(),
            comment_drafts: std::collections::HashMap::new(),
        }
    }

    pub fn help_max_scroll(&self) -> usize {
        if let Some(help) = self.help {
            help.max_scroll(self.terminal_area)
        } else {
            0
        }
    }

    pub fn open(&mut self, conversation: String) {
        if !self.conversation.is_empty() {
            self.conversation_drafts
                .insert(self.conversation.clone(), self.input.text.clone());
        }
        self.stop_poller.store(true, Ordering::SeqCst);
        self.generation += 1;
        self.conversation = conversation.clone();
        self.view = View::default();
        let restored_draft = self
            .conversation_drafts
            .get(&conversation)
            .cloned()
            .unwrap_or_default();
        self.input = InputBuffer::with_text(restored_draft);
        self.input_shown = 0;
        self.scroll = 0;
        self.details = None;
        self.diff = None;
        self.editor = None;
        self.picker = None;
        self.help = None;
        self.help_scroll = 0;
        self.selected = ORCHESTRATOR.into();
        self.offset = (0, 0);
        self.history_index = None;
        self.history_draft.clear();
        self.sending = false;
        self.pending_send = None;
        self.sending_comment = false;
        self.pending_comment = None;
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

    pub fn open_diff(&mut self, key: &str) {
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
            KeyCode::Char('?') => self.show_help(Help::Diff),
            KeyCode::Char('c') => {
                if let Some(target) = diff.target() {
                    let text = self.comment_drafts.remove(&target).unwrap_or_default();
                    self.editor = Some(Editor {
                        target,
                        input: InputBuffer::with_text(text),
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

    pub fn recall_history(&mut self, older: bool) {
        if self.history.is_empty() {
            return;
        }
        let index = match (self.history_index, older) {
            (None, true) => {
                self.history_draft = self.input.text.clone();
                Some(self.history.len() - 1)
            }
            (None, false) => return,
            (Some(i), true) => Some(i.saturating_sub(1)),
            (Some(i), false) if i + 1 < self.history.len() => Some(i + 1),
            (Some(_), false) => None,
        };
        self.history_index = index;
        let text = match index {
            Some(i) => self.history[i].clone(),
            None => std::mem::take(&mut self.history_draft),
        };
        self.input = InputBuffer::with_text(text);
    }

    pub fn record_history(&mut self, text: &str) {
        self.history_index = None;
        self.history_draft.clear();
        let trimmed = text.trim();
        if !trimmed.is_empty() && self.history.last().map(String::as_str) != Some(trimmed) {
            self.history.push(trimmed.to_owned());
        }
    }

    pub fn submit(&mut self) {
        if self.sending {
            self.notice = Some("Wysyłanie w toku; szkic zachowany.".into());
            return;
        }
        let trimmed = self.input.text.trim();
        if trimmed.is_empty() {
            return;
        }
        if trimmed.starts_with('/') {
            let text = std::mem::take(&mut self.input.text);
            self.input.cursor = 0;
            self.input_shown = 0;
            self.dispatch(text.trim());
            return;
        }

        let text = self.input.text.clone();
        self.record_history(&text);
        let sent_text = std::mem::take(&mut self.input.text);
        self.input.cursor = 0;
        self.input_shown = 0;
        self.scroll = 0;
        self.sending = true;
        self.pending_send = Some(sent_text.clone());

        let client = self.client.clone();
        let updates = self.updates.clone();
        let conversation = self.conversation.clone();
        let generation = self.generation;

        std::thread::spawn(move || {
            let reply = client.call(&Request::Send {
                conversation,
                text: sent_text.clone(),
            });
            let _ = updates.send(Update::SendResult {
                generation,
                sent_text,
                result: reply.map(|_| ()).map_err(|e| e.to_string()),
            });
        });
    }

    pub fn submit_comment(&mut self) {
        if self.sending_comment {
            self.notice = Some("Wysyłanie komentarza w toku; szkic zachowany.".into());
            return;
        }
        let Some(editor) = &mut self.editor else {
            return;
        };
        if editor.input.text.trim().is_empty() {
            return;
        }
        let editor = self.editor.take().expect("editing");
        self.sending_comment = true;
        self.pending_comment = Some(editor.clone());

        let client = self.client.clone();
        let updates = self.updates.clone();
        let conversation = self.conversation.clone();
        let generation = self.generation;

        std::thread::spawn(move || {
            let reply = client.call(&Request::Comment {
                conversation,
                target: editor.target.clone(),
                text: editor.input.text.clone(),
            });
            let _ = updates.send(Update::CommentResult {
                generation,
                editor,
                result: reply.map(|_| ()).map_err(|e| e.to_string()),
            });
        });
    }

    pub fn apply_update(&mut self, update: Update) {
        match update {
            Update::Events {
                generation,
                events,
                idle,
                autonomous,
            } if generation == self.generation => {
                for event in &events {
                    self.view.apply(event);
                }
                self.view.idle = idle;
                self.view.autonomous = Some(autonomous);
            }
            Update::Events { .. } => {}
            Update::Conversation(id) => self.open(id),
            Update::Notice(text) => self.notice = Some(text),
            Update::Task {
                generation,
                key,
                task,
            } if generation == self.generation => {
                if let Some(details) = self.details.as_mut().filter(|details| details.key == key) {
                    details.task = Some(task);
                }
            }
            Update::Task { .. } => {}
            Update::Diff {
                generation,
                task,
                reply,
            } if generation == self.generation => {
                if let Some(diff) = self.diff.as_mut().filter(|diff| diff.task == task) {
                    diff.load(reply);
                }
            }
            Update::Diff { .. } => {}
            Update::SendResult {
                generation,
                sent_text,
                result,
            } if generation == self.generation => {
                self.sending = false;
                self.pending_send = None;
                match result {
                    Ok(_) => {}
                    Err(error) => {
                        if self.input.is_empty() {
                            self.input = InputBuffer::with_text(sent_text);
                        } else {
                            let mut merged = sent_text;
                            if !merged.ends_with('\n')
                                && !self.input.text.starts_with('\n')
                                && !merged.ends_with(' ')
                                && !self.input.text.starts_with(' ')
                            {
                                merged.push(' ');
                            }
                            merged.push_str(&self.input.text);
                            self.input = InputBuffer::with_text(merged);
                        }
                        self.notice = Some(format!("✗ {error} (szkic przywrócony)"));
                    }
                }
            }
            Update::SendResult { .. } => {
                self.sending = false;
                self.pending_send = None;
            }
            Update::CommentResult {
                generation,
                editor,
                result,
            } if generation == self.generation => {
                self.sending_comment = false;
                self.pending_comment = None;
                match result {
                    Ok(_) => {
                        self.comment_drafts.remove(&editor.target);
                    }
                    Err(error) => {
                        if let Some(active_editor) = &mut self.editor {
                            if active_editor.target == editor.target {
                                if active_editor.input.is_empty() {
                                    active_editor.input = editor.input;
                                } else {
                                    let mut merged = editor.input.text;
                                    if !merged.ends_with('\n')
                                        && !active_editor.input.text.starts_with('\n')
                                        && !merged.ends_with(' ')
                                        && !active_editor.input.text.starts_with(' ')
                                    {
                                        merged.push(' ');
                                    }
                                    merged.push_str(&active_editor.input.text);
                                    active_editor.input = InputBuffer::with_text(merged);
                                }
                            }
                        } else {
                            self.editor = Some(editor);
                        }
                        self.notice = Some(format!("✗ {error} (szkic komentarza zachowany)"));
                    }
                }
            }
            Update::CommentResult { .. } => {
                self.sending_comment = false;
                self.pending_comment = None;
            }
        }
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
    pub fn command(&mut self, command: &str) {
        self.dispatch(command);
    }

    fn help_here(&self) -> Help {
        if self.picker.is_some() {
            Help::Picker
        } else if self.editor.is_some() {
            Help::Comment
        } else if self.diff.is_some() {
            Help::Diff
        } else if self.details.is_some() {
            Help::Details
        } else if self.focus == Focus::Graph {
            Help::Graph
        } else {
            Help::Chat
        }
    }

    pub fn show_help(&mut self, help: Help) {
        self.help = Some(help);
        self.help_scroll = 0;
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
                    Some(Action::Show(entry)) => {
                        self.details = None;
                        self.diff = None;
                        self.focus = Focus::Chat;
                        self.found = Some(entry);
                        // Brought to the top at the next draw.
                        self.scroll = usize::MAX;
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
            Command::Search => self.picker = Some(Picker::new(Kind::History)),
            Command::Details => {
                self.details = Some(Details::new(selected));
                self.fetch_task();
            }
            Command::Diff => self.open_diff(&selected),
            Command::Comment => {
                let target = selected;
                let text = self.comment_drafts.remove(&target).unwrap_or_default();
                self.editor = Some(Editor {
                    target,
                    input: InputBuffer::with_text(text),
                })
            }
            Command::Filter => self.filter = self.filter.next(),
            Command::Help => {
                let help = self.help_here();
                self.show_help(help);
            }
            Command::Quit => return false,
        }
        true
    }

    /// Returns false when the user quits.
    pub fn key(&mut self, key: event::KeyEvent) -> bool {
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        if self.picker.is_none() {
            // The search result stays where it is, no longer highlighted.
            self.found = None;
        }

        // Global exit
        if key.code == KeyCode::Char('c') && control {
            return false;
        }

        // Modal routing: active modal gets strict key ownership
        if self.help.is_some() {
            let max_scroll = self.help_max_scroll();
            match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.help_scroll = self.help_scroll.saturating_sub(1);
                    return true;
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.help_scroll = (self.help_scroll + 1).min(max_scroll);
                    return true;
                }
                KeyCode::PageUp => {
                    self.help_scroll = self.help_scroll.saturating_sub(5);
                    return true;
                }
                KeyCode::PageDown => {
                    self.help_scroll = (self.help_scroll + 5).min(max_scroll);
                    return true;
                }
                KeyCode::Home => {
                    self.help_scroll = 0;
                    return true;
                }
                KeyCode::End => {
                    self.help_scroll = max_scroll;
                    return true;
                }
                KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('?') => {
                    self.help = None;
                    self.help_scroll = 0;
                }
                _ => {}
            }
            return true;
        }
        if key.code == KeyCode::F(1) {
            self.show_help(self.help_here());
            return true;
        }
        if self.picker.is_some() {
            return self.picker_key(key);
        }
        if self.editor.is_some() {
            self.editor_key(key);
            return true;
        }
        if self.diff.is_some() {
            self.diff_key(key);
            return true;
        }
        if self.details.is_some() {
            self.details_key(key);
            return true;
        }

        // Base layout global shortcuts
        match key.code {
            KeyCode::Char('p') if control => {
                self.picker = Some(Picker::new(Kind::Commands));
                return true;
            }
            KeyCode::Char('f') if control => {
                self.picker = Some(Picker::new(Kind::History));
                return true;
            }
            KeyCode::Char('n') if control => {
                self.new_conversation();
                return true;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Chat => Focus::Graph,
                    Focus::Graph => Focus::Chat,
                };
                return true;
            }
            _ => {}
        }

        if self.focus == Focus::Graph {
            match key.code {
                KeyCode::Char('a') if control => self.command("/accept"),
                KeyCode::Char('x') if control => self.command("/cancel"),
                KeyCode::Char('r') if control => self.command("/resume"),
                _ => return self.graph_key(key),
            }
            return true;
        }

        // Focus::Chat input editing & transcript navigation
        match key.code {
            KeyCode::Esc => {
                if self.history_index.is_some() {
                    self.history_index = None;
                    let draft = std::mem::take(&mut self.history_draft);
                    self.input = InputBuffer::with_text(draft);
                } else if !self.input.is_empty() {
                    self.history_draft = self.input.text.clone();
                    self.input.clear();
                    self.input_shown = 0;
                } else {
                    return false;
                }
            }
            KeyCode::Char('a') if control => {
                if self.input.is_empty() && self.view.plan_proposed() {
                    self.command("/accept");
                } else {
                    self.input.move_home();
                }
            }
            KeyCode::Char('r') if control => {
                if self.input.is_empty() && self.view.plan_resumable() {
                    self.command("/resume");
                } else {
                    self.picker = Some(Picker::new(Kind::History));
                }
            }
            KeyCode::Char('x') if control => self.command("/cancel"),
            KeyCode::Char('u') if control => self.input.delete_to_start(),
            KeyCode::Char('k') if control => self.input.delete_to_end(),
            KeyCode::Char('w') if control => self.input.delete_word_back(),
            KeyCode::Char('e') if control => self.input.move_end(),
            KeyCode::Char('j') if control => self.input.insert_char('\n'),
            KeyCode::Enter if alt => self.input.insert_char('\n'),
            KeyCode::Char('s') if control => self.submit(),
            KeyCode::Enter => self.submit(),
            KeyCode::Backspace if alt || control => {
                self.input.delete_word_back();
            }
            KeyCode::Backspace => {
                self.input.backspace();
            }
            KeyCode::Delete => {
                self.input.delete();
            }
            KeyCode::Left if alt || control => self.input.move_word_left(),
            KeyCode::Left => self.input.move_left(),
            KeyCode::Right if alt || control => self.input.move_word_right(),
            KeyCode::Right => self.input.move_right(),
            KeyCode::Home if alt || control => self.input.move_buffer_start(),
            KeyCode::Home => self.input.move_home(),
            KeyCode::End if alt || control => {
                self.input.move_buffer_end();
                self.scroll = 0;
            }
            KeyCode::End => self.input.move_end(),
            KeyCode::PageUp => self.scroll += 10,
            KeyCode::PageDown => self.scroll = self.scroll.saturating_sub(10),
            KeyCode::Up => {
                let width = self.input_width.max(1);
                if self.input.is_empty() {
                    if !self.history.is_empty() {
                        self.recall_history(true);
                    } else {
                        self.scroll += 1;
                    }
                } else if self.input.on_first_row(width) && !self.history.is_empty() {
                    self.recall_history(true);
                } else {
                    let (_, row) = self.input.visual_cursor(width);
                    if row > 0 {
                        self.input.move_up(width);
                    }
                }
            }
            KeyCode::Down => {
                let width = self.input_width.max(1);
                if self.history_index.is_some() && self.input.on_last_row(width) {
                    self.recall_history(false);
                } else if self.input.is_empty() {
                    self.scroll = self.scroll.saturating_sub(1);
                } else {
                    let (_, row) = self.input.visual_cursor(width);
                    let lines = self.input.visual_line_count(width);
                    if row + 1 < lines {
                        self.input.move_down(width);
                    } else if self.scroll > 0 {
                        self.scroll = self.scroll.saturating_sub(1);
                    }
                }
            }
            KeyCode::Char(ch) if !control => self.input.insert_char(ch),
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
            KeyCode::Char('?') => self.show_help(Help::Graph),
            KeyCode::Char('/') => self.picker = Some(Picker::new(Kind::Agents)),
            KeyCode::Char('f') => self.filter = self.filter.next(),
            KeyCode::Char('d') => {
                let key = self.selected.clone();
                self.open_diff(&key);
            }
            KeyCode::Char('c') => {
                let target = self.selected.clone();
                let text = self.comment_drafts.remove(&target).unwrap_or_default();
                self.editor = Some(Editor {
                    target,
                    input: InputBuffer::with_text(text),
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
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Esc => {
                if let Some(editor) = self.editor.take().filter(|e| !e.input.is_empty()) {
                    self.comment_drafts.insert(editor.target, editor.input.text);
                }
            }
            KeyCode::Enter if alt => editor.input.insert_char('\n'),
            KeyCode::Char('j') if control => editor.input.insert_char('\n'),
            KeyCode::Char('s') if control => self.submit_comment(),
            KeyCode::Enter => self.submit_comment(),
            KeyCode::Char('w') if control => editor.input.delete_word_back(),
            KeyCode::Char('u') if control => editor.input.delete_to_start(),
            KeyCode::Char('k') if control => editor.input.delete_to_end(),
            KeyCode::Char('a') if control => editor.input.move_home(),
            KeyCode::Char('e') if control => editor.input.move_end(),
            KeyCode::Backspace if alt || control => {
                editor.input.delete_word_back();
            }
            KeyCode::Backspace => {
                editor.input.backspace();
            }
            KeyCode::Delete => {
                editor.input.delete();
            }
            KeyCode::Left if alt || control => editor.input.move_word_left(),
            KeyCode::Right if alt || control => editor.input.move_word_right(),
            KeyCode::Left => editor.input.move_left(),
            KeyCode::Right => editor.input.move_right(),
            KeyCode::Home if alt || control => editor.input.move_buffer_start(),
            KeyCode::End if alt || control => editor.input.move_buffer_end(),
            KeyCode::Home => editor.input.move_home(),
            KeyCode::End => editor.input.move_end(),
            KeyCode::Up => editor.input.move_up(self.editor_width.max(1)),
            KeyCode::Down => editor.input.move_down(self.editor_width.max(1)),
            KeyCode::Char(ch) if !control => {
                editor.input.insert_char(ch);
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
            KeyCode::Char('?') => self.show_help(Help::Details),
            KeyCode::Char('c') => {
                let target = details.key.clone();
                let text = self.comment_drafts.remove(&target).unwrap_or_default();
                self.editor = Some(Editor {
                    target,
                    input: InputBuffer::with_text(text),
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

    pub fn paste(&mut self, text: &str) {
        if self.help.is_some() {
            return;
        }
        if text.len() > editor::MAX_PASTE_BYTES {
            self.notice = Some("Wklejony tekst przycięto do 64 KB".into());
        }
        if let Some(picker) = &mut self.picker {
            let clean_text = sanitize_single_line_paste(text);
            picker.query.push_str(&clean_text);
            picker.selected = 0;
        } else if let Some(editor) = &mut self.editor {
            let clean_text = sanitize_paste(text);
            editor.input.insert_str(&clean_text);
        } else if self.focus == Focus::Chat && self.diff.is_none() && self.details.is_none() {
            let clean_text = sanitize_paste(text);
            self.input.insert_str(&clean_text);
        }
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        self.terminal_area = frame.area();
        if draw_min_term_guard(frame, frame.area()) {
            return;
        }
        if let Some(diff) = &mut self.diff {
            diff.draw(frame, &self.view);
        } else if let Some(details) = &mut self.details {
            details.draw(frame, &self.view);
        } else {
            self.draw_main(frame);
        }
        if let Some(editor) = &self.editor {
            let area = frame.area();
            // Leave a column for the insertion cursor after a full line.
            let inner_width = usize::from(area.width.saturating_sub(3).max(1));
            self.editor_width = inner_width;
            let lines = visual_text_lines(&editor.input.text, inner_width);
            let height = lines.len().clamp(1, 6) as u16 + 2;
            if area.height < height + 1 {
                return;
            }
            let visible_box_lines = (height.saturating_sub(2) as usize).max(1);
            let (cursor_col, cursor_row) = editor.input.visual_cursor(inner_width);
            let mut shown = 0;
            if cursor_row >= visible_box_lines {
                shown = cursor_row - visible_box_lines + 1;
            }
            shown = shown.min(lines.len().saturating_sub(visible_box_lines));
            let end = (shown + visible_box_lines).min(lines.len());
            let shown_lines = &lines[shown..end];

            let box_area = Rect::new(area.x, area.bottom() - 1 - height, area.width, height);
            frame.render_widget(Clear, box_area);
            frame.render_widget(
                Paragraph::new(
                    shown_lines
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
            if self.help.is_none()
                && self.picker.is_none()
                && box_area.width >= 3
                && box_area.height >= 3
            {
                let min_x = box_area.x + 1;
                let max_x = box_area.right().saturating_sub(2);
                let min_y = box_area.y + 1;
                let max_y = box_area.bottom().saturating_sub(2);
                let cursor_x = (box_area.x + 1 + cursor_col).clamp(min_x, max_x);
                let visible_row = cursor_row.saturating_sub(shown).min(visible_box_lines - 1);
                let cursor_y = (box_area.y + 1 + visible_row as u16).clamp(min_y, max_y);
                frame.set_cursor_position((cursor_x, cursor_y));
            }
        }
        if let Some(picker) = &mut self.picker {
            picker.draw_with_cursor(frame, &self.view, self.help.is_none());
        }
        if let Some(help) = self.help {
            let max_scroll = help.max_scroll(frame.area());
            self.help_scroll = self.help_scroll.min(max_scroll);
            help.draw_scroll(frame, self.help_scroll);
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
        let panel_focus = match self.focus {
            Focus::Chat => PanelFocus::Chat,
            Focus::Graph => PanelFocus::Graph,
        };
        frame.render_widget(
            Paragraph::new(render_header(
                &self.project,
                &self.conversation,
                self.view.autonomous,
                &status,
                panel_focus,
                header.width,
            )),
            header,
        );

        if body.width >= SIDE_BY_SIDE_BREAKPOINT {
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

        let hint = main_footer_hint(
            panel_focus,
            self.view.plan_proposed(),
            self.view.plan_resumable(),
            self.notice.as_deref(),
            usize::from(footer.width),
        );
        frame.render_widget(Paragraph::new(Span::styled(hint, dim())), footer);
    }

    fn draw_chat(&mut self, frame: &mut Frame, area: Rect) {
        // Leave a column for the insertion cursor after a full line.
        let input_width = area.width.saturating_sub(3) as usize;
        self.input_width = input_width.max(1);
        let input_lines = visual_text_lines(&self.input.text, self.input_width);
        let input_height = input_lines.len().clamp(1, 6) as u16 + 2;
        let [transcript, input] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(input_height)]).areas(area);
        self.draw_transcript(frame, transcript);

        let visible_box_lines = (input.height.saturating_sub(2) as usize).max(1);
        let (cursor_col, cursor_row) = self.input.visual_cursor(self.input_width);

        if cursor_row < self.input_shown {
            self.input_shown = cursor_row;
        } else if cursor_row >= self.input_shown + visible_box_lines {
            self.input_shown = cursor_row - visible_box_lines + 1;
        }
        self.input_shown = self
            .input_shown
            .min(input_lines.len().saturating_sub(visible_box_lines));

        let end = (self.input_shown + visible_box_lines).min(input_lines.len());
        let shown_lines = &input_lines[self.input_shown..end];

        let focused = self.focus == Focus::Chat
            && self.editor.is_none()
            && self.picker.is_none()
            && self.help.is_none()
            && self.diff.is_none()
            && self.details.is_none();

        frame.render_widget(
            Paragraph::new(
                shown_lines
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
        if focused && input.width >= 3 && input.height >= 3 {
            let min_x = input.x + 1;
            let max_x = input.right().saturating_sub(2);
            let min_y = input.y + 1;
            let max_y = input.bottom().saturating_sub(2);
            let cursor_x = (input.x + 1 + cursor_col).clamp(min_x, max_x);
            let visible_row = cursor_row
                .saturating_sub(self.input_shown)
                .min(visible_box_lines - 1);
            let cursor_y = (input.y + 1 + visible_row as u16).clamp(min_y, max_y);
            frame.set_cursor_position((cursor_x, cursor_y));
        }
    }

    fn draw_transcript(&mut self, frame: &mut Frame, area: Rect) {
        let width = area.width.saturating_sub(1) as usize;
        let mut top = None;
        let mut lines: Vec<Line> = vec![];
        if self.view.transcript.is_empty() {
            lines.extend(transcript_empty_state(width));
        }
        for (index, entry) in self.view.transcript.iter().enumerate() {
            let style = if self.found == Some(index) {
                top = Some(lines.len());
                entry.style.add_modifier(Modifier::REVERSED)
            } else {
                entry.style
            };
            lines.extend(
                wrap(&entry.text, width)
                    .into_iter()
                    .map(|line| Line::styled(line, style)),
            );
        }
        let height = area.height as usize;
        if let Some(top) = top.take().filter(|_| self.scroll == usize::MAX) {
            self.scroll = lines.len().saturating_sub(top + height);
        }
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
            .map(|line| line.width().saturating_add(4))
            .max()
            .unwrap_or(0)
            .clamp(16, 48) as u16;
        let width = width.min(inner.width);
        let height = progress.len().min(6) as u16 + 2;
        let Some(overlay) = graph::compute_progress_overlay(
            inner,
            graph.node(&self.selected),
            self.offset,
            width,
            height,
        ) else {
            return;
        };
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

/// Whether to draw in ASCII: asked for with `ANTISLOP_ASCII`, or the
/// locale is not UTF-8.
fn ascii_terminal() -> bool {
    if std::env::var_os("ANTISLOP_ASCII").is_some_and(|value| !value.is_empty()) {
        return true;
    }
    let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|value| !value.is_empty()))
        .unwrap_or_default()
        .to_ascii_lowercase();
    !(locale.contains("utf-8") || locale.contains("utf8"))
}

/// The ASCII stand-in for a character the screen uses; `?` for others.
fn ascii(ch: char) -> char {
    match ch {
        '─' | '–' | '—' | '−' | '·' => '-',
        // The selected node keeps a border unlike the others.
        '═' => '=',
        '│' | '║' | '⏸' => '|',
        '┌' | '┐' | '└' | '┘' | '├' | '┤' | '┬' | '┴' | '┼' | '╔' | '╗' | '╚' | '╝' | '╭' | '╮'
        | '╯' | '╰' => '+',
        '▶' | '→' | '↳' => '>',
        '◀' | '←' => '<',
        '▲' | '↑' => '^',
        '▼' | '↓' => 'v',
        '…' => '.',
        '○' => 'o',
        '●' => '*',
        '✓' => '+',
        '✗' => 'x',
        '⚠' => '!',
        '⚙' | '✎' => '#',
        '✉' => '@',
        '§' => 'S',
        'ą' => 'a',
        'ć' => 'c',
        'ę' => 'e',
        'ł' => 'l',
        'ń' => 'n',
        'ó' => 'o',
        'ś' => 's',
        'ź' | 'ż' => 'z',
        'Ą' => 'A',
        'Ć' => 'C',
        'Ę' => 'E',
        'Ł' => 'L',
        'Ń' => 'N',
        'Ó' => 'O',
        'Ś' => 'S',
        'Ź' | 'Ż' => 'Z',
        ch if ch.is_ascii() => ch,
        _ => '?',
    }
}

/// Replaces every character with its ASCII stand-in (design §8.3).
pub fn in_ascii(buffer: &mut ratatui::buffer::Buffer) {
    for cell in &mut buffer.content {
        if !cell.symbol().is_ascii() {
            let first = cell.symbol().chars().next().unwrap_or(' ');
            cell.set_char(ascii(first));
        }
    }
}

/// Drops every color; what was dim stays dim, selection is reversed anyway.
pub fn without_color(buffer: &mut ratatui::buffer::Buffer) {
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
        input: InputBuffer::with_text(options.message.clone().unwrap_or_default()),
        input_shown: 0,
        input_width: 80,
        editor_width: 80,
        scroll: 0,
        found: None,
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
        help_scroll: 0,
        terminal_area: Rect::default(),
        history: vec![],
        history_index: None,
        history_draft: String::new(),
        sending: false,
        sending_comment: false,
        pending_send: None,
        pending_comment: None,
        conversation_drafts: std::collections::HashMap::new(),
        comment_drafts: std::collections::HashMap::new(),
    };
    app.open(conversation);

    // https://no-color.org: nothing depends on color alone (design §17).
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
    let ascii = ascii_terminal();
    let mut terminal = ratatui::init();
    let _ = crossterm::execute!(std::io::stdout(), EnableBracketedPaste);
    let result = (|| -> Result<()> {
        loop {
            while let Ok(update) = received.try_recv() {
                app.apply_update(update);
            }
            terminal.draw(|frame| {
                app.draw(frame);
                if no_color {
                    without_color(frame.buffer_mut());
                }
                if ascii {
                    in_ascii(frame.buffer_mut());
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
                Event::Paste(text) => app.paste(&text),
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
pub mod tests {
    use super::*;
    use ratatui::buffer::Buffer;

    #[test]
    fn ascii_replaces_box_drawing_marks_and_letters() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 12, 1));
        buffer.set_string(0, 0, "╭─✓ łódź 漢", Style::default());
        in_ascii(&mut buffer);
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert_eq!(text, "+-+ lodz ?  ");
    }

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

    pub fn test_app() -> App {
        App::test_app()
    }

    pub fn key_press(code: KeyCode) -> event::KeyEvent {
        event::KeyEvent::new(code, KeyModifiers::NONE)
    }

    pub fn key_ctrl(ch: char) -> event::KeyEvent {
        event::KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
    }

    #[test]
    fn ac_key_01_unconditional_pageup_transcript_scroll() {
        let mut app = test_app();
        app.input = InputBuffer::with_text("multiline\ndraft\nmessage");
        app.input.cursor = 5;
        app.key(key_press(KeyCode::PageUp));
        assert_eq!(app.scroll, 10);
        assert_eq!(app.input.cursor, 5);
        app.key(key_press(KeyCode::PageDown));
        assert_eq!(app.scroll, 0);
        assert_eq!(app.input.cursor, 5);
    }

    #[test]
    fn ac_key_02_empty_input_history_scroll() {
        let mut app = test_app();
        assert!(app.input.is_empty());
        app.key(key_press(KeyCode::Up));
        assert_eq!(app.scroll, 1);
        app.key(key_press(KeyCode::Down));
        assert_eq!(app.scroll, 0);
    }

    #[test]
    fn ac_key_03_multiline_boundary_up() {
        let mut app = test_app();
        app.input = InputBuffer::with_text("line1\nline2");
        app.input.cursor = 2; // on line 0 ('n')
        app.key(key_press(KeyCode::Up));
        // Stays on line 0, does not jump or scroll transcript
        assert_eq!(app.scroll, 0);
        assert_eq!(app.input.cursor, 2);
    }

    #[test]
    fn ac_modal_02_comment_editor_dismissal_preserves_graph_selection() {
        let mut app = test_app();
        app.focus = Focus::Graph;
        app.selected = "verifier:t1".into();
        app.editor = Some(Editor {
            target: app.selected.clone(),
            input: InputBuffer::with_text("draft comment"),
        });
        assert!(app.key(key_press(KeyCode::Esc)));
        assert!(app.editor.is_none());
        assert_eq!(app.selected, "verifier:t1");
    }

    #[test]
    fn ac_modal_03_detach_protection_on_non_empty_chat() {
        let mut app = test_app();
        app.input = InputBuffer::with_text("test");
        let open = app.key(key_press(KeyCode::Esc));
        assert!(open);
        assert!(app.input.is_empty());
    }

    #[test]
    fn ac_modal_04_clean_detach_on_empty_chat() {
        let mut app = test_app();
        assert!(app.input.is_empty());
        let open = app.key(key_press(KeyCode::Esc));
        assert!(!open);
    }

    #[test]
    fn ac_modal_05_modal_isolated_plan_shortcuts() {
        let mut app = test_app();
        app.view.apply(&serde_json::json!({
            "seq": 1,
            "kind": "plan_proposed",
            "payload": {"plan": "p1", "tasks": [{"id": "t1", "title": "A"}]}
        }));
        assert!(app.view.plan_proposed());
        app.editor = Some(Editor {
            target: "orchestrator".into(),
            input: InputBuffer::with_text("draft comment"),
        });
        app.editor.as_mut().unwrap().input.cursor = 7;
        app.key(key_ctrl('a'));
        // Moves cursor to line start in editor, does NOT trigger /accept
        assert_eq!(app.editor.as_ref().unwrap().input.cursor, 0);
    }

    #[test]
    fn draft_preservation_across_panel_switching_and_modals() {
        let mut app = test_app();
        app.input = InputBuffer::with_text("@t2 please investigate");
        app.key(key_press(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Graph);
        assert_eq!(app.input.text, "@t2 please investigate");

        app.key(key_press(KeyCode::Char('c')));
        assert!(app.editor.is_some());
        assert_eq!(app.input.text, "@t2 please investigate");

        app.key(key_press(KeyCode::Esc));
        assert!(app.editor.is_none());
        assert_eq!(app.input.text, "@t2 please investigate");

        app.key(key_press(KeyCode::Tab));
        assert_eq!(app.focus, Focus::Chat);
        assert_eq!(app.input.text, "@t2 please investigate");
    }

    #[test]
    fn comment_targets_from_graph_and_details() {
        let mut app = test_app();
        app.selected = "worker:t2".into();
        app.focus = Focus::Graph;
        app.key(key_press(KeyCode::Char('c')));
        assert_eq!(app.editor.as_ref().unwrap().target, "worker:t2");

        app.editor = None;
        app.details = Some(Details::new("verifier:t1".into()));
        app.key(key_press(KeyCode::Char('c')));
        assert_eq!(app.editor.as_ref().unwrap().target, "verifier:t1");
    }

    #[test]
    fn chat_ctrl_a_plan_accept_vs_readline_home() {
        let mut app = test_app();
        app.view.apply(&serde_json::json!({
            "seq": 1,
            "kind": "plan_proposed",
            "payload": {"plan": "p1", "tasks": [{"id": "t1", "title": "A"}]}
        }));

        // Non-empty draft: Ctrl+A acts as Home
        app.input = InputBuffer::with_text("draft message");
        app.input.cursor = 5;
        app.key(key_ctrl('a'));
        assert_eq!(app.input.cursor, 0);
        assert_eq!(app.input.text, "draft message");
    }

    #[test]
    fn paste_routing_respects_help_modal_and_active_overlays() {
        let mut app = test_app();
        app.input = InputBuffer::with_text("draft");

        // When Help is open, paste does NOT touch chat draft
        app.help = Some(Help::Chat);
        app.paste("pasted while help open");
        assert_eq!(app.input.text, "draft");

        app.help = None;

        // When comment editor is open, paste routes to comment editor, not chat
        app.editor = Some(Editor {
            target: "t1".into(),
            input: InputBuffer::with_text("ab"),
        });
        app.editor.as_mut().unwrap().input.cursor = 1;
        app.paste("XYZ");
        assert_eq!(app.editor.as_ref().unwrap().input.text, "aXYZb");
        assert_eq!(app.input.text, "draft");

        app.editor = None;

        // When chat is focused and unobstructed, paste routes to chat at cursor
        app.input.cursor = 0;
        app.paste("new ");
        assert_eq!(app.input.text, "new draft");
    }

    #[test]
    fn ac_resp_03_app_draw_min_term_guard_on_50x10() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = test_app();
        let backend = TestBackend::new(50, 10);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(text.contains("Terminal zbyt mały (wymagane min. 60x12)"));
    }

    #[test]
    fn ac_resp_01_app_draw_main_compact_switch_at_99_cols() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = test_app();
        app.focus = Focus::Chat;
        let backend = TestBackend::new(99, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(text.contains("[Czat]"));
        assert!(text.contains("Zespół"));
        assert!(text.contains("(Tab: przełącz)"));

        // Switch to Focus::Graph
        app.focus = Focus::Graph;
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(text.contains("Czat"));
        assert!(text.contains("[Zespół]"));
        assert!(text.contains("(Tab: przełącz)"));
    }

    #[test]
    fn ac_resp_02_app_draw_main_wide_split_at_120_cols() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = test_app();
        app.view.idle = true;
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(!text.contains("[Czat]"));
        assert!(!text.contains("(Tab: przełącz)"));
        assert!(text.contains("antislop"));
        assert!(text.contains("gotowy"));
    }

    #[test]
    fn app_draw_transcript_empty_state() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = test_app();
        assert!(app.view.transcript.is_empty());
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(text.contains("Brak wiadomości w tej rozmowie"));
    }

    #[test]
    fn ac_edit_exact_width_chat_and_comment_cursor_stays_inside_inner_viewport() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        // 1. Chat input with text exactly matching inner width (60 cols wide -> inner width is 58)
        let mut app = test_app();
        app.input = InputBuffer::with_text("a".repeat(58));
        app.input.move_end();

        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();

        let (cursor_x, cursor_y) = terminal.get_cursor_position().unwrap().into();
        // Inner x is 1..=58. Border is at 0 (left) and 59 (right).
        // Cursor must be strictly inside the inner viewport: x <= 58, x >= 1.
        assert!(
            (1..=58).contains(&cursor_x),
            "Chat cursor x ({}) must be inside inner viewport [1, 58] and not land on right border 59",
            cursor_x
        );
        assert!(
            (1..=10).contains(&cursor_y),
            "Chat cursor y ({}) must be inside inner viewport",
            cursor_y
        );

        // 2. Comment editor with exact inner width text
        let mut app = test_app();
        app.editor = Some(Editor {
            target: "worker:t1".into(),
            input: InputBuffer::with_text("b".repeat(58)),
        });
        app.editor.as_mut().unwrap().input.move_end();

        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();

        let (cursor_x, cursor_y) = terminal.get_cursor_position().unwrap().into();
        assert!(
            (1..=58).contains(&cursor_x),
            "Comment editor cursor x ({}) must be inside inner viewport and not on border",
            cursor_x
        );
        assert!(
            (1..=10).contains(&cursor_y),
            "Comment editor cursor y ({}) must be inside inner viewport",
            cursor_y
        );

        // 3. Resizing from wider to narrower terminal viewports
        let mut app = test_app();
        app.input = InputBuffer::with_text("first line of text\nsecond line of text");
        app.input.move_end();

        // Draw at 100x30 first
        let backend100 = TestBackend::new(100, 30);
        let mut term100 = Terminal::new(backend100).unwrap();
        term100.draw(|frame| app.draw(frame)).unwrap();

        // Resize down to 60x12
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let (rx, ry) = terminal.get_cursor_position().unwrap().into();
        assert!((1..=58).contains(&rx));
        assert!((1..=10).contains(&ry));
    }

    #[test]
    fn ac_resp_app_draw_picker_visible_and_reversed_at_60x12() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = test_app();
        let mut picker = Picker::new(Kind::Commands);
        let items = picker.items(&app.view);
        assert!(items.len() >= 10);
        picker.selected = items.len() - 1;
        app.picker = Some(picker);

        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let last_label = &items[items.len() - 1].0;
        let last_word = last_label.split_whitespace().next().unwrap();

        let mut found_reversed = false;
        for y in 0..12 {
            let row: String = (0..60).map(|x| buffer[(x, y)].symbol()).collect();
            if row.contains(last_word) {
                for x in 0..60 {
                    if buffer[(x, y)].modifier.contains(Modifier::REVERSED) {
                        found_reversed = true;
                        break;
                    }
                }
            }
        }
        assert!(
            found_reversed,
            "Last picker item must be visible and reversed in App::draw at 60x12"
        );
    }

    #[test]
    fn ac_help_app_draw_all_shortcuts_accessible_at_60x12() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = test_app();
        app.input = InputBuffer::with_text("draft message to preserve");
        app.show_help(Help::Chat);

        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();

        // 1. Initial page (scroll = 0): verify first set of shortcuts and full descriptions
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();

        let buffer1 = terminal.backend().buffer();
        let text1: String = buffer1.content.iter().map(|c| c.symbol()).collect();

        // Check shortcuts and their full descriptions
        assert!(text1.contains("wyślij wiadomość orkiestratorowi"));
        assert!(text1.contains("nowa linia w szkicu"));
        assert!(text1.contains("wiersz w szkicu"));
        assert!(text1.contains("przewijanie rozmowy"));
        assert!(text1.contains("uruchom/wznów"));
        assert!(text1.contains("usuń do początku / końca"));

        // 2. Navigate/scroll help down and verify modal isolation
        for _ in 0..8 {
            app.key(event::KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        assert_eq!(app.help_scroll, 8, "Help should scroll down to max_scroll");
        assert_eq!(
            app.input.text, "draft message to preserve",
            "Help navigation must not modify underlying input draft"
        );

        // 3. Scrolled view: verify subsequent shortcuts and complete descriptions
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();

        let buffer2 = terminal.backend().buffer();
        let text2: String = buffer2.content.iter().map(|c| c.symbol()).collect();

        assert!(text2.contains("anuluj trwającą pracę"));
        assert!(text2.contains("wstrzymaj nowe kroki / kontynuuj"));
        assert!(text2.contains("/mode autonomous"));
        assert!(text2.contains("/mode supervised"));
        assert!(text2.contains("szukaj w rozmowie"));
        assert!(text2.contains("paleta poleceń"));
        assert!(text2.contains("czyść szkic; wyjdź gdy pusty"));

        // 4. Verify End followed by Down, PageDown, Up (clamping and no overflow panic)
        let max_scroll = app.help_max_scroll();
        assert_eq!(max_scroll, 8);

        app.key(event::KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        assert_eq!(app.help_scroll, max_scroll);

        // Down at bottom stays clamped to max_scroll without integer overflow
        app.key(event::KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(app.help_scroll, max_scroll);

        // PageDown at bottom stays clamped to max_scroll without integer overflow
        app.key(event::KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
        assert_eq!(app.help_scroll, max_scroll);

        // Up immediately decrements to max_scroll - 1 without deadlock
        app.key(event::KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
        assert_eq!(app.help_scroll, max_scroll - 1);

        // 5. Verify resizing clamps stored help scroll
        let mut terminal_tall = Terminal::new(TestBackend::new(60, 24)).unwrap();
        terminal_tall
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert_eq!(app.help_max_scroll(), 0);
        assert_eq!(
            app.help_scroll, 0,
            "Resizing must clamp stored scroll to new max_scroll"
        );

        // Resize back to 60x12
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        assert_eq!(app.help_max_scroll(), 8);

        // 6. Graph help at 60x12: verify long filter relation description is fully accessible without clipping
        app.show_help(Help::Graph);
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        let buffer_graph = terminal.backend().buffer();
        let text_graph: String = buffer_graph.content.iter().map(|c| c.symbol()).collect();
        assert!(text_graph.contains("filtr relacji:"));
        assert!(text_graph.contains("wszystko"));
        assert!(text_graph.contains("zależności"));
        assert!(text_graph.contains("weryfikacja"));
        assert!(text_graph.contains("wiadomości"));
        assert!(text_graph.contains("nadzór"));

        // 7. Modal dismissal and draft preservation
        app.key(event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        assert!(app.help.is_none(), "Esc must close help modal");
        assert_eq!(
            app.input.text, "draft message to preserve",
            "Closing help modal must preserve the input draft"
        );
    }

    #[test]
    fn ac_resp_app_draw_progress_overlay_collision_free() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        // 1. Render App with active progress in wide mode (100x30)
        let mut app = test_app();
        app.view.activity = Some("trwa kompilacja testów".into());
        app.focus = Focus::Graph;
        app.selected = ORCHESTRATOR.into();

        let backend = TestBackend::new(100, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
        // Progress overlay renders with "teraz" title and doesn't obscure orchestrator
        assert!(text.contains("teraz"));
        assert!(text.contains("orkiestrator") || text.contains(ORCHESTRATOR));

        // Verify orchestrator node cells remain intact in the buffer
        let mut orchestrator_visible = false;
        for y in 0..30 {
            let row: String = (0..100).map(|x| buffer[(x, y)].symbol()).collect();
            if row.contains("orkiestrator") || row.contains(ORCHESTRATOR) {
                orchestrator_visible = true;
                break;
            }
        }
        assert!(orchestrator_visible, "Selected node must remain visible");

        // 2. Compact render with tiny inner area where all overlay placements collide
        // When compute_progress_overlay returns None, App::draw hides the overlay
        let mut app_compact = test_app();
        app_compact.view.activity = Some("bardzo długa linia aktywności".into());
        app_compact.focus = Focus::Graph;

        // Create a fake test terminal where inner area forces collision
        let backend_tiny = TestBackend::new(60, 12);
        let mut term_tiny = Terminal::new(backend_tiny).unwrap();
        term_tiny
            .draw(|frame| {
                app_compact.draw(frame);
            })
            .unwrap();

        let tiny_buf = term_tiny.backend().buffer();
        let tiny_text: String = tiny_buf.content.iter().map(|c| c.symbol()).collect();
        // Even at 60x12 in graph focus, orchestrator node remains visible
        assert!(tiny_text.contains("orkiestrator") || tiny_text.contains(ORCHESTRATOR));
    }
}
