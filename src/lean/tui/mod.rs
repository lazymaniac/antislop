//! Terminal frontend for one conversation: the chat with the orchestrator
//! beside the team graph. It never waits on the daemon in its draw loop: a
//! poller thread long-polls events and every action runs on its own thread,
//! so typing, scrolling and sending stay instant.

pub mod config;
pub mod details;
pub mod diffview;
pub mod editor;
pub mod graph;
mod history;
pub mod palette;
pub mod planview;
pub mod sanitize;
pub mod session;
pub mod taskinfo;
pub mod theme;
pub mod unblock;
pub mod view;
pub mod workspace;

pub use details::Details;
pub use diffview::DiffView;
pub use editor::{InputBuffer, sanitize_paste, sanitize_single_line_paste, visual_text_lines};
pub use graph::{Filter, GroupingMode};
pub use palette::{Help, Kind, Picker};
pub use planview::{PlanAction, PlanView};
pub use sanitize::{sanitize, sanitize_line};
pub use theme::{
    CardProps, Glyphs, Row, Theme, Tone, card, draw_rows, scrollbar, section, split_row, tone,
};
pub use view::{
    ConfirmationAction, ConfirmationCard, ConfirmationStatus, MessageEntry, TranscriptCard,
};

use super::cli::Options;
use super::daemon::{Client, Request};
use super::{projects, trust};
use crate::Result;
use crossterm::event::{
    self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
    Event, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use details::{CHECKS, COMMENTS, RESULT};
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
    main_footer_hint, target_name, truncate, wrap,
};

pub enum Update {
    Config {
        project: String,
        save: bool,
        reply: std::result::Result<Value, String>,
    },
    Events {
        conversation: String,
        generation: u64,
        events: Vec<Value>,
        idle: bool,
        autonomous: bool,
    },
    Conversation(String),
    Notice(String),
    /// A project was opened: switch to it, to `conversation` if one was named.
    ProjectReady {
        conn: Box<workspace::ProjectConn>,
        conversation: Option<String>,
    },
    /// The directory must be trusted before it is opened.
    ProjectTrust(std::path::PathBuf),
    /// The conversations of a connected project, as it last listed them.
    ProjectStatus {
        path: std::path::PathBuf,
        conversations: Vec<Value>,
    },
    /// A task's change, for the diff view of `task`.
    Diff {
        conversation: String,
        plan: Option<String>,
        generation: u64,
        request_id: u64,
        task: String,
        reply: std::result::Result<Value, String>,
    },
    /// A task as the daemon stores it, for the details of `key`. `round`
    /// is the task's round when the fetch was issued; a reply for a round
    /// that already moved on is dropped instead of shown stale.
    Task {
        conversation: String,
        plan: Option<String>,
        generation: u64,
        request_id: u64,
        key: String,
        round: u64,
        task: Value,
    },
    SendResult {
        conversation: String,
        generation: u64,
        request_id: u64,
        sent_text: String,
        result: std::result::Result<(), String>,
    },
    CommentResult {
        conversation: String,
        plan: Option<String>,
        generation: u64,
        request_id: u64,
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

/// Full preserved state of a conversation across switches.
#[derive(Debug, Clone, PartialEq)]
pub struct ConversationState {
    pub input: InputBuffer,
    pub editor: Option<Editor>,
    pub sending: bool,
    pub pending_send: Option<String>,
    pub send_request_id: u64,
    pub failed_send: Option<String>,
    pub sending_comment: bool,
    pub pending_comment: Option<Editor>,
    pub comment_request_id: u64,
    pub failed_comment: Option<Editor>,
    pub comment_drafts: std::collections::HashMap<String, String>,
    pub comment_draft_cursors: std::collections::HashMap<String, (usize, Option<u16>)>,
    pub selected: String,
    pub filter: Filter,
    pub offset: (u16, u16),
    pub details: Option<Details>,
    pub diff: Option<DiffView>,
    pub plan_view: Option<PlanView>,
    pub scroll: usize,
    pub task_request_id: u64,
    pub diff_request_id: u64,
    pub focus: Focus,
    pub graph_filter: Option<String>,
    pub graph_collapsed: std::collections::HashSet<String>,
    pub graph_grouping: graph::GroupingMode,
    pub confirmation: Option<ConfirmationCard>,
    pub details_scroll_memory: std::collections::HashMap<(String, usize), usize>,
    pub details_query_memory: std::collections::HashMap<(String, usize), String>,
    pub graph_viewport: Option<(u16, u16)>,
}

pub struct App {
    pub config_editor: Option<config::ConfigEditor>,
    config_drafts: std::collections::HashMap<String, config::ConfigEditor>,
    pub client: Client,
    pub project: String,
    pub conversation: String,
    pub generation: u64,
    pub stop_poller: Arc<AtomicBool>,
    /// Frames drawn so far: advances spinners; tests set it directly.
    pub tick: u64,
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
    pub plan_view: Option<PlanView>,
    pub editor: Option<Editor>,
    pub picker: Option<Picker>,
    pub help: Option<Help>,
    pub help_scroll: usize,
    pub terminal_area: Rect,
    pub history: Vec<String>,
    history_path: Option<std::path::PathBuf>,
    pub history_index: Option<usize>,
    pub history_draft: String,
    pub history_draft_cursor: usize,
    pub history_draft_preferred_col: Option<u16>,
    pub history_item_cursors: std::collections::HashMap<usize, usize>,
    pub sending: bool,
    pub sending_comment: bool,
    pub pending_send: Option<String>,
    pub pending_comment: Option<Editor>,
    pub send_request_id: u64,
    pub comment_request_id: u64,
    pub diff_request_id: u64,
    pub task_request_id: u64,
    pub failed_send: Option<String>,
    pub failed_comment: Option<Editor>,
    pub conversation_drafts: std::collections::HashMap<String, String>,
    pub comment_drafts: std::collections::HashMap<String, String>,
    pub comment_draft_cursors: std::collections::HashMap<String, (usize, Option<u16>)>,
    pub conversation_states: std::collections::HashMap<String, ConversationState>,
    pub graph_filter: Option<String>,
    pub graph_collapsed: std::collections::HashSet<String>,
    pub graph_grouping: graph::GroupingMode,
    pub confirmation: Option<ConfirmationCard>,
    pub details_scroll_memory: std::collections::HashMap<(String, usize), usize>,
    pub details_query_memory: std::collections::HashMap<(String, usize), String>,
    /// The last rendered graph viewport in terminal coordinates, for hit testing.
    pub graph_area: Option<Rect>,
    pub graph_viewport: Option<(u16, u16)>,
    pub trust_prompt: Option<String>,
    /// The way out for a stopped step, while the user picks one.
    pub unblock: Option<unblock::Unblock>,
    /// The projects the screen is connected to; the active one's client is
    /// `client`.
    pub projects: Vec<workspace::ProjectConn>,
    pub active_project: usize,
    /// A directory the user is asked about before it is opened.
    pub project_trust: Option<std::path::PathBuf>,
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
            config_editor: None,
            config_drafts: Default::default(),
            client,
            project: "test".into(),
            conversation: "test-conv".into(),
            generation: 1,
            stop_poller: Arc::new(AtomicBool::new(false)),
            tick: 0,
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
            plan_view: None,
            editor: None,
            picker: None,
            help: None,
            help_scroll: 0,
            terminal_area: Rect::new(0, 0, 80, 24),
            history: vec![],
            history_path: None,
            history_index: None,
            history_draft: String::new(),
            history_draft_cursor: 0,
            history_draft_preferred_col: None,
            history_item_cursors: std::collections::HashMap::new(),
            sending: false,
            sending_comment: false,
            pending_send: None,
            pending_comment: None,
            send_request_id: 0,
            comment_request_id: 0,
            diff_request_id: 0,
            task_request_id: 0,
            failed_send: None,
            failed_comment: None,
            conversation_drafts: std::collections::HashMap::new(),
            comment_drafts: std::collections::HashMap::new(),
            comment_draft_cursors: std::collections::HashMap::new(),
            conversation_states: std::collections::HashMap::new(),
            graph_filter: None,
            graph_collapsed: std::collections::HashSet::new(),
            graph_grouping: graph::GroupingMode::default(),
            confirmation: None,
            details_scroll_memory: std::collections::HashMap::new(),
            details_query_memory: std::collections::HashMap::new(),
            graph_viewport: None,
            graph_area: None,
            trust_prompt: None,
            unblock: None,
            projects: Vec::new(),
            active_project: 0,
            project_trust: None,
        }
    }

    pub fn require_trust(&mut self, project_path: impl Into<String>) {
        self.trust_prompt = Some(project_path.into());
    }

    pub fn accept_trust(&mut self) {
        if self.trust_prompt.take().is_some() {
            self.request(
                Request::Trust { trusted: true },
                Some("workspace directory trusted"),
            );
            self.notice = Some("workspace directory trusted".into());
        }
    }

    pub fn help_max_scroll(&self) -> usize {
        if let Some(help) = self.help {
            help.max_scroll(self.terminal_area)
        } else {
            0
        }
    }

    pub fn comment_key(conversation: &str, plan: Option<&str>, target: &str) -> String {
        match plan {
            Some(p) => format!("{conversation}:{p}:{target}"),
            None => format!("{conversation}::{target}"),
        }
    }

    pub fn get_comment_draft(&mut self, target: &str) -> InputBuffer {
        let key = Self::comment_key(&self.conversation, self.view.plan.as_deref(), target);
        if let Some(text) = self.comment_drafts.remove(&key) {
            let (cursor, preferred_col) = self
                .comment_draft_cursors
                .remove(&key)
                .unwrap_or((text.len(), None));
            let mut buf = InputBuffer::with_text(text);
            buf.cursor = cursor.min(buf.text.len());
            buf.preferred_col = preferred_col;
            return buf;
        }
        let fallback_key = format!("{}::{target}", self.conversation);
        if let Some(text) = self.comment_drafts.remove(&fallback_key) {
            let (cursor, preferred_col) = self
                .comment_draft_cursors
                .remove(&fallback_key)
                .unwrap_or((text.len(), None));
            let mut buf = InputBuffer::with_text(text);
            buf.cursor = cursor.min(buf.text.len());
            buf.preferred_col = preferred_col;
            return buf;
        }
        if let Some(text) = self.comment_drafts.remove(target) {
            let (cursor, preferred_col) = self
                .comment_draft_cursors
                .remove(target)
                .unwrap_or((text.len(), None));
            let mut buf = InputBuffer::with_text(text);
            buf.cursor = cursor.min(buf.text.len());
            buf.preferred_col = preferred_col;
            return buf;
        }
        InputBuffer::new()
    }

    pub fn save_comment_draft(&mut self, target: &str, input: InputBuffer) {
        let key = Self::comment_key(&self.conversation, self.view.plan.as_deref(), target);
        self.comment_draft_cursors
            .insert(key.clone(), (input.cursor, input.preferred_col));
        self.comment_drafts.insert(key, input.text);
    }

    pub fn remove_comment_draft(&mut self, target: &str) {
        let key = Self::comment_key(&self.conversation, self.view.plan.as_deref(), target);
        self.comment_drafts.remove(&key);
        self.comment_draft_cursors.remove(&key);
        let fallback_key = format!("{}::{target}", self.conversation);
        self.comment_drafts.remove(&fallback_key);
        self.comment_draft_cursors.remove(&fallback_key);
        self.comment_drafts.remove(target);
        self.comment_draft_cursors.remove(target);
    }

    pub fn open(&mut self, conversation: String) {
        if !self.conversation.is_empty() {
            self.conversation_drafts
                .insert(self.conversation.clone(), self.input.text.clone());
            self.conversation_states.insert(
                self.conversation.clone(),
                ConversationState {
                    input: self.input.clone(),
                    editor: self.editor.clone(),
                    sending: self.sending,
                    pending_send: self.pending_send.clone(),
                    send_request_id: self.send_request_id,
                    failed_send: self.failed_send.clone(),
                    sending_comment: self.sending_comment,
                    pending_comment: self.pending_comment.clone(),
                    comment_request_id: self.comment_request_id,
                    failed_comment: self.failed_comment.clone(),
                    comment_drafts: self.comment_drafts.clone(),
                    comment_draft_cursors: self.comment_draft_cursors.clone(),
                    selected: self.selected.clone(),
                    filter: self.filter,
                    offset: self.offset,
                    details: self.details.clone(),
                    diff: self.diff.clone(),
                    plan_view: self.plan_view.clone(),
                    scroll: self.scroll,
                    task_request_id: self.task_request_id,
                    diff_request_id: self.diff_request_id,
                    focus: self.focus,
                    graph_filter: self.graph_filter.clone(),
                    graph_collapsed: self.graph_collapsed.clone(),
                    graph_grouping: self.graph_grouping,
                    confirmation: self.confirmation.clone(),
                    details_scroll_memory: self.details_scroll_memory.clone(),
                    details_query_memory: self.details_query_memory.clone(),
                    graph_viewport: self.graph_viewport,
                },
            );
        }
        self.stop_poller.store(true, Ordering::SeqCst);
        self.generation += 1;
        self.conversation = conversation.clone();
        self.view = View::default();
        if let Some(state) = self.conversation_states.get(&conversation).cloned() {
            self.input = state.input;
            self.editor = state.editor;
            self.sending = state.sending;
            self.pending_send = state.pending_send;
            self.send_request_id = state.send_request_id;
            self.failed_send = state.failed_send;
            self.sending_comment = state.sending_comment;
            self.pending_comment = state.pending_comment;
            self.comment_request_id = state.comment_request_id;
            self.failed_comment = state.failed_comment;
            self.comment_drafts = state.comment_drafts;
            self.comment_draft_cursors = state.comment_draft_cursors;
            self.selected = state.selected;
            self.filter = state.filter;
            self.offset = state.offset;
            self.details = state.details;
            self.diff = state.diff;
            self.plan_view = state.plan_view;
            self.scroll = state.scroll;
            self.task_request_id = state.task_request_id;
            self.diff_request_id = state.diff_request_id;
            self.focus = state.focus;
            self.graph_filter = state.graph_filter;
            self.graph_collapsed = state.graph_collapsed;
            self.graph_grouping = state.graph_grouping;
            self.confirmation = state.confirmation;
            self.details_scroll_memory = state.details_scroll_memory;
            self.details_query_memory = state.details_query_memory;
            self.graph_viewport = state.graph_viewport;
        } else {
            let restored_draft = self
                .conversation_drafts
                .get(&conversation)
                .cloned()
                .unwrap_or_default();
            self.input = InputBuffer::with_text(restored_draft);
            self.details = None;
            self.diff = None;
            self.plan_view = None;
            self.editor = None;
            self.confirmation = None;
            self.selected = ORCHESTRATOR.into();
            self.filter = Filter::All;
            self.offset = (0, 0);
            self.scroll = 0;
            self.sending = false;
            self.pending_send = None;
            self.send_request_id = 0;
            self.failed_send = None;
            self.sending_comment = false;
            self.pending_comment = None;
            self.comment_request_id = 0;
            self.failed_comment = None;
            self.task_request_id = 0;
            self.diff_request_id = 0;
            self.focus = Focus::Chat;
            self.graph_filter = None;
            self.graph_collapsed.clear();
            self.graph_grouping = graph::GroupingMode::default();
            self.graph_viewport = None;
        }
        self.input_shown = 0;
        self.picker = None;
        self.help = None;
        self.help_scroll = 0;
        self.history_index = None;
        self.history_draft.clear();
        self.history_draft_cursor = 0;
        self.history_draft_preferred_col = None;
        self.history_item_cursors.clear();
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
                            conversation: conversation.clone(),
                            generation,
                            events,
                            idle: reply["idle"] == true,
                            autonomous: reply["autonomous"] == true,
                        }
                    }
                    Err(error) => {
                        std::thread::sleep(Duration::from_secs(1));
                        Update::Notice(format!("daemon is not responding: {error}"))
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

    /// The round of the task row behind a details key, if the row is shown.
    fn task_row_round(&self, key: &str) -> Option<u64> {
        let id = key.split_once(':')?.1;
        self.view
            .tasks
            .iter()
            .find(|task| task.id == id)
            .map(|task| task.round)
    }

    /// Refetches an open details panel once its task has moved on to a new
    /// round, so the stored report it shows never belongs to a past round.
    fn refetch_stale_details(&mut self) {
        let stale = self
            .details
            .as_ref()
            .and_then(|details| {
                let fetched = details.fetched_round?;
                let now = self.task_row_round(&details.key)?;
                // Checks the verifier settles meanwhile change the Checks tab.
                let checks =
                    details.tab == CHECKS && details.fetched_checks != self.view.check_events;
                Some(fetched != now || checks)
            })
            .unwrap_or(false);
        if stale {
            self.fetch_task();
        }
    }

    /// Fetches the stored task behind the open details. The request carries
    /// the task id, the plan and the task's round, and replies are matched
    /// against all of them plus this conversation's generation and the
    /// request id, so a late answer can never paste one round's data into
    /// another round's or another plan's details.
    fn fetch_task(&mut self) {
        let (Some(plan), Some(id)) = (
            self.view.plan.clone(),
            self.details
                .as_ref()
                .and_then(|details| details.task_id().map(str::to_owned)),
        ) else {
            return;
        };
        let Some(key) = self.details.as_ref().map(|details| details.key.clone()) else {
            return;
        };
        let round = self.task_row_round(&key).unwrap_or(0);
        self.task_request_id += 1;
        let request_id = self.task_request_id;
        if let Some(details) = self
            .details
            .as_mut()
            .filter(|details| details.task_id() == Some(id.as_str()))
        {
            details.fetched_round = Some(round);
            details.fetched_checks = self.view.check_events;
        }
        let request = Request::Task {
            plan: plan.clone(),
            id,
        };
        let plan_id = plan;
        let conversation = self.conversation.clone();
        let generation = self.generation;
        let client = self.client.clone();
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            let _ = updates.send(match client.call(&request) {
                Ok(task) => Update::Task {
                    conversation,
                    plan: Some(plan_id),
                    generation,
                    request_id,
                    key,
                    round,
                    task,
                },
                Err(error) => Update::Notice(format!("✗ {error}")),
            });
        });
    }

    pub fn open_details(&mut self, key: String) {
        if let Some(details) = &mut self.details {
            details.switch_key(key);
        } else {
            let mut details = Details::new(key);
            details.scroll_memory = self.details_scroll_memory.clone();
            details.query_memory = self.details_query_memory.clone();
            details.restore_current_tab();
            self.details = Some(details);
        }
        self.fetch_task();
    }

    pub fn request_confirmation(&mut self, mut card: ConfirmationCard) {
        if card.scope_hash.is_empty() {
            card.scope_hash = match card.action {
                ConfirmationAction::AcceptPlan | ConfirmationAction::ResumePlan => self
                    .view
                    .plan_scope_hash
                    .clone()
                    .or_else(|| card.plan_id.clone())
                    .unwrap_or_default(),
                ConfirmationAction::ReviewDiff => {
                    if let Some(tid) = &card.task_id {
                        self.view
                            .tasks
                            .iter()
                            .find(|t| &t.id == tid)
                            .and_then(|t| t.scope_hash.clone())
                            .unwrap_or_default()
                    } else {
                        String::new()
                    }
                }
                ConfirmationAction::CancelWork => self
                    .view
                    .conversation_scope_hash
                    .clone()
                    .unwrap_or_default(),
            };
        }
        self.confirmation = Some(card.clone());
        self.view.cards.push(TranscriptCard::Confirmation(card));
    }

    pub fn is_confirmation_stale(&self, card: &ConfirmationCard) -> bool {
        match card.action {
            ConfirmationAction::AcceptPlan | ConfirmationAction::ResumePlan => {
                if let Some(plan) = &card.plan_id
                    && self.view.plan.as_ref() != Some(plan)
                {
                    return true;
                }
                if let Some(current_hash) = &self.view.plan_scope_hash
                    && !card.scope_hash.is_empty()
                    && current_hash != &card.scope_hash
                {
                    return true;
                }
                false
            }
            ConfirmationAction::ReviewDiff => {
                if let Some(plan) = &card.plan_id
                    && self.view.plan.as_ref() != Some(plan)
                {
                    return true;
                }
                if let Some(tid) = &card.task_id
                    && let Some(task) = self.view.tasks.iter().find(|t| &t.id == tid)
                    && let Some(current_hash) = &task.scope_hash
                    && !card.scope_hash.is_empty()
                    && current_hash != &card.scope_hash
                {
                    return true;
                }
                false
            }
            ConfirmationAction::CancelWork => {
                if let Some(current_hash) = &self.view.conversation_scope_hash
                    && !card.scope_hash.is_empty()
                    && current_hash != &card.scope_hash
                {
                    return true;
                }
                false
            }
        }
    }

    pub fn confirm_action(&mut self) {
        if let Some(mut card) = self.confirmation.take() {
            if self.is_confirmation_stale(&card) {
                card.status = ConfirmationStatus::Stale;
                for c in self.view.cards.iter_mut().rev() {
                    if let TranscriptCard::Confirmation(existing) = c
                        && existing.id == card.id
                    {
                        existing.status = ConfirmationStatus::Stale;
                        break;
                    }
                }
                self.notice = Some("✗ stale_scope: scope changed, action rejected".into());
                return;
            }
            let conversation = self.conversation.clone();
            let scope_hash = Some(card.scope_hash.clone());
            card.status = ConfirmationStatus::Confirmed;
            for c in self.view.cards.iter_mut().rev() {
                if let TranscriptCard::Confirmation(existing) = c
                    && existing.id == card.id
                {
                    existing.status = ConfirmationStatus::Confirmed;
                    break;
                }
            }
            match card.action {
                ConfirmationAction::AcceptPlan => {
                    if let Some(plan) = card.plan_id {
                        self.request(
                            Request::AcceptScoped {
                                conversation,
                                plan,
                                scope_hash,
                            },
                            Some("plan accepted"),
                        );
                    }
                }
                ConfirmationAction::ResumePlan => {
                    if let Some(plan) = card.plan_id {
                        self.request(
                            Request::ResumeScoped {
                                conversation,
                                plan,
                                scope_hash,
                            },
                            Some("plan resumed"),
                        );
                    }
                }
                ConfirmationAction::ReviewDiff => {
                    if let (Some(plan), Some(id)) = (card.plan_id, card.task_id) {
                        self.request(
                            Request::ReviewScoped {
                                conversation,
                                plan,
                                id,
                                scope_hash,
                            },
                            Some("changes approved"),
                        );
                    }
                }
                ConfirmationAction::CancelWork => {
                    self.request(
                        Request::CancelScoped {
                            conversation,
                            scope_hash,
                        },
                        Some("cancelled"),
                    );
                }
            }
        }
    }

    pub fn reject_confirmation(&mut self) {
        if let Some(mut card) = self.confirmation.take() {
            card.status = ConfirmationStatus::Rejected;
            for c in self.view.cards.iter_mut().rev() {
                if let TranscriptCard::Confirmation(existing) = c
                    && existing.id == card.id
                {
                    existing.status = ConfirmationStatus::Rejected;
                    break;
                }
            }
            self.notice = Some("Action rejected".into());
        }
    }

    pub fn open_diff(&mut self, key: &str) {
        if let Some((_, task)) = key.split_once(':') {
            self.diff = Some(DiffView::new(task.to_owned()));
            self.fetch_diff();
        }
    }

    fn fetch_diff(&mut self) {
        let (Some(diff), Some(plan)) = (&self.diff, &self.view.plan) else {
            return;
        };
        self.diff_request_id += 1;
        let request_id = self.diff_request_id;
        let request = Request::Diff {
            conversation: self.conversation.clone(),
            plan: plan.clone(),
            id: diff.task.clone(),
        };
        let task = diff.task.clone();
        let plan_id = plan.clone();
        let conversation = self.conversation.clone();
        let generation = self.generation;
        let client = self.client.clone();
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            let reply = client.call(&request).map_err(|error| error.to_string());
            let _ = updates.send(Update::Diff {
                conversation,
                plan: Some(plan_id),
                generation,
                request_id,
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
                    let input = self.get_comment_draft(&target);
                    self.editor = Some(Editor { target, input });
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
                    self.request(request, Some("code review started"));
                }
            }
            KeyCode::Char(ch @ ('u' | 'd')) if key.modifiers.contains(KeyModifiers::CONTROL) => {
                diff.half_page(ch == 'd');
            }
            code => diff.key(code),
        }
    }

    pub fn recall_history(&mut self, older: bool) {
        if self.history.is_empty() {
            return;
        }
        if let Some(curr) = self.history_index {
            self.history_item_cursors.insert(curr, self.input.cursor);
        }
        let index = match (self.history_index, older) {
            (None, true) => {
                self.history_draft = self.input.text.clone();
                self.history_draft_cursor = self.input.cursor;
                self.history_draft_preferred_col = self.input.preferred_col;
                Some(self.history.len() - 1)
            }
            (None, false) => return,
            (Some(i), true) => Some(i.saturating_sub(1)),
            (Some(i), false) if i + 1 < self.history.len() => Some(i + 1),
            (Some(_), false) => None,
        };
        self.history_index = index;
        match index {
            Some(i) => {
                let text = self.history[i].clone();
                let saved_cursor = self.history_item_cursors.get(&i).copied();
                let mut buf = InputBuffer::with_text(text);
                if let Some(c) = saved_cursor {
                    buf.cursor = c.min(buf.text.len());
                }
                self.input = buf;
            }
            None => {
                let text = std::mem::take(&mut self.history_draft);
                let cursor = self.history_draft_cursor.min(text.len());
                let mut buf = InputBuffer::with_text(text);
                buf.cursor = cursor;
                buf.preferred_col = self.history_draft_preferred_col;
                self.input = buf;
            }
        };
    }

    /// Restores the project's input history before a conversation is opened.
    pub fn load_history(&mut self, state: &std::path::Path) {
        let path = state.join("input-history.db");
        self.history = match history::load(&path) {
            Ok(entries) => entries,
            Err(error) => {
                self.notice = Some(format!(
                    "Cannot load input history ({}): {error}",
                    path.display()
                ));
                Vec::new()
            }
        };
        self.history_path = Some(path);
    }

    pub fn record_history(&mut self, text: &str) {
        self.history_index = None;
        self.history_draft.clear();
        self.history_draft_cursor = 0;
        self.history_draft_preferred_col = None;
        self.history_item_cursors.clear();
        let trimmed = text.trim();
        if !trimmed.is_empty() && self.history.last().map(String::as_str) != Some(trimmed) {
            self.history.push(trimmed.to_owned());
            if self.history.len() > history::LIMIT {
                self.history.remove(0);
            }
            if let Some(path) = &self.history_path
                && let Err(error) = history::append(path, trimmed)
            {
                self.notice = Some(format!(
                    "Cannot save input history ({}): {error}",
                    path.display()
                ));
            }
        }
    }

    pub fn submit(&mut self) {
        if self.sending {
            self.notice = Some("Sending in progress; draft preserved.".into());
            return;
        }
        let trimmed = self.input.text.trim();
        if trimmed.is_empty() {
            return;
        }
        if trimmed.starts_with('/') {
            let text = std::mem::take(&mut self.input.text);
            self.input.cursor = 0;
            self.input.preferred_col = None;
            self.input_shown = 0;
            self.dispatch(text.trim());
            return;
        }

        self.send_request_id += 1;
        let request_id = self.send_request_id;
        let text = self.input.text.clone();
        self.record_history(&text);
        let sent_text = std::mem::take(&mut self.input.text);
        self.input.cursor = 0;
        self.input.preferred_col = None;
        self.input_shown = 0;
        self.scroll = 0;
        self.sending = true;
        self.pending_send = Some(sent_text.clone());
        self.failed_send = None;

        let client = self.client.clone();
        let updates = self.updates.clone();
        let conversation = self.conversation.clone();
        let generation = self.generation;

        std::thread::spawn(move || {
            let reply = client.call(&Request::Send {
                conversation: conversation.clone(),
                text: sent_text.clone(),
            });
            let _ = updates.send(Update::SendResult {
                conversation,
                generation,
                request_id,
                sent_text,
                result: reply.map(|_| ()).map_err(|e| e.to_string()),
            });
        });
    }

    pub fn submit_comment(&mut self) {
        if self.sending_comment {
            self.notice = Some("Sending comment in progress; draft preserved.".into());
            return;
        }
        let Some(editor) = &mut self.editor else {
            return;
        };
        let rejecting =
            editor.target.starts_with("reject:") || editor.target.starts_with("unblock:");
        if editor.input.text.trim().is_empty() && !rejecting {
            return;
        }
        self.comment_request_id += 1;
        let request_id = self.comment_request_id;
        let editor = self.editor.take().expect("editing");
        if rejecting {
            self.plan_view = None;
        }
        self.sending_comment = true;
        self.pending_comment = Some(editor.clone());
        self.failed_comment = None;

        let client = self.client.clone();
        let updates = self.updates.clone();
        let conversation = self.conversation.clone();
        let plan = self.view.plan.clone();
        let generation = self.generation;
        // A comment on a plan line carries the line itself, so the agent need
        // not count lines in the document.
        let quote = editor
            .target
            .rsplit_once(":line:")
            .and_then(|(_, line)| line.parse::<usize>().ok())
            .and_then(|line| {
                let doc = PlanView::new(String::new()).build_lines(&self.view);
                doc.into_iter().find(|l| l.number == line).map(|l| l.text)
            })
            .map(|text| text.trim().chars().take(200).collect::<String>())
            .filter(|text| !text.is_empty());
        let text = match quote {
            Some(quote) => format!("> {quote}\n{}", editor.input.text),
            None => editor.input.text.clone(),
        };

        std::thread::spawn(move || {
            let reply = if let Some(rest) = editor.target.strip_prefix("unblock:") {
                // unblock:<action>:<plan>:<task>; the text is a hint or reason.
                let mut parts = rest.splitn(3, ':');
                let (action, plan, task) = (
                    parts.next().unwrap_or_default(),
                    parts.next().unwrap_or_default(),
                    parts.next().unwrap_or_default(),
                );
                let message = Some(text.trim().to_owned()).filter(|m| !m.is_empty());
                serde_json::from_value::<crate::lean::RecoveryAction>(serde_json::json!(action))
                    .map_err(|e| crate::Error::Invalid(e.to_string()))
                    .and_then(|action| {
                        recover(&client, &conversation, plan, task, action, message)
                            .map(|notice| serde_json::json!(notice))
                            .map_err(crate::Error::Invalid)
                    })
            } else if let Some(rejected) = editor.target.strip_prefix("reject:") {
                let reason = Some(text.trim().to_owned()).filter(|reason| !reason.is_empty());
                client.call(&Request::Reject {
                    conversation: conversation.clone(),
                    plan: rejected.to_owned(),
                    reason,
                })
            } else {
                client.call(&Request::CommentScoped {
                    conversation: conversation.clone(),
                    plan: plan.clone(),
                    target: editor.target.clone(),
                    text,
                })
            };
            let _ = updates.send(Update::CommentResult {
                conversation,
                plan,
                generation,
                request_id,
                editor,
                result: reply.map(|_| ()).map_err(|e| e.to_string()),
            });
        });
    }

    pub fn apply_update(&mut self, update: Update) {
        match update {
            Update::Config {
                project,
                save,
                reply,
            } => {
                let active = project == self.client.state_dir().to_string_lossy();
                if save {
                    let editor = if active {
                        self.config_editor.as_mut()
                    } else {
                        self.config_drafts.get_mut(&project)
                    };
                    if let Some(editor) = editor {
                        editor.pending = false;
                        editor.status = match reply {
                            Ok(value) => {
                                if value["warnings"].as_array().is_some_and(|w| !w.is_empty()) {
                                    format!("Applied live with warnings: {}", value["warnings"])
                                } else {
                                    "Saved and applied live.".into()
                                }
                            }
                            Err(error) => error,
                        };
                    }
                } else {
                    match reply.and_then(|v| {
                        config::ConfigEditor::new(
                            v["toml"].as_str().unwrap_or_default(),
                            v["path"].as_str().unwrap_or_default().into(),
                        )
                        .map_err(|e| e.to_string())
                    }) {
                        Ok(mut editor) => {
                            if active {
                                if self.config_editor.is_none() {
                                    self.config_editor = Some(editor);
                                }
                            } else {
                                editor.visible = false;
                                self.config_drafts.entry(project).or_insert(editor);
                            }
                        }
                        Err(error) => self.notice = Some(error),
                    }
                }
            }

            Update::Events {
                conversation,
                generation,
                events,
                idle,
                autonomous,
            } => {
                if conversation == self.conversation && generation == self.generation {
                    let mut saw_manual_plan = false;
                    for event in &events {
                        if event["kind"] == "plan_proposed" && event["payload"]["automatic"] != true
                        {
                            saw_manual_plan = true;
                        }
                        self.view.apply(event);
                    }
                    self.view.idle = idle;
                    self.view.autonomous = Some(autonomous);
                    // Events run in batches, never per frame: if one started
                    // a new round for the task the details show, the stored
                    // record on screen belongs to the old round. Refetch it
                    // now, once, instead of letting it age on every redraw.
                    self.refetch_stale_details();
                    // Replayed history also carries old proposals; only a plan
                    // still awaiting a decision is worth opening.
                    if saw_manual_plan
                        && self.view.plan_proposed()
                        && !self.view.tasks.is_empty()
                        && self.confirmation.is_none()
                    {
                        let pid = self.view.plan.clone().unwrap_or_else(|| "plan".into());
                        self.plan_view = Some(PlanView::new(pid));
                    }
                    let stale = self
                        .confirmation
                        .as_ref()
                        .is_some_and(|c| self.is_confirmation_stale(c));
                    if stale && let Some(mut card) = self.confirmation.take() {
                        card.status = ConfirmationStatus::Stale;
                        for c in self.view.cards.iter_mut().rev() {
                            if let TranscriptCard::Confirmation(existing) = c
                                && existing.id == card.id
                            {
                                existing.status = ConfirmationStatus::Stale;
                                break;
                            }
                        }
                        self.notice = Some("✗ stale_scope: scope changed".into());
                    }
                }
            }
            Update::Conversation(id) => self.open(id),
            Update::ProjectReady { conn, conversation } => {
                let index = self.register_project(*conn);
                self.switch_project(index, conversation);
            }
            Update::ProjectTrust(path) => self.project_trust = Some(path),
            Update::ProjectStatus {
                path,
                conversations,
            } => {
                if let Some(project) = self.projects.iter_mut().find(|p| p.path == path) {
                    project.conversations = conversations;
                }
            }
            Update::Notice(text) => {
                if text.contains("stale_scope") {
                    if let Some(mut card) = self.confirmation.take() {
                        card.status = ConfirmationStatus::Stale;
                        for c in self.view.cards.iter_mut().rev() {
                            if let TranscriptCard::Confirmation(existing) = c
                                && existing.id == card.id
                            {
                                existing.status = ConfirmationStatus::Stale;
                                break;
                            }
                        }
                    }
                    for c in self.view.cards.iter_mut().rev() {
                        if let TranscriptCard::Confirmation(existing) = c
                            && existing.status == ConfirmationStatus::Confirmed
                        {
                            existing.status = ConfirmationStatus::Stale;
                            break;
                        }
                    }
                }
                self.notice = Some(text);
            }
            Update::Task {
                conversation,
                plan,
                generation: _,
                request_id,
                key,
                round,
                task,
            } => {
                // A task that moved on while its record was in flight must
                // not show the earlier round's report or verdict.
                let round_current = match self.task_row_round(&key) {
                    Some(current) => current == round,
                    None => true,
                };
                if conversation == self.conversation
                    && request_id == self.task_request_id
                    && self.view.plan.as_ref() == plan.as_ref()
                    && round_current
                    && let Some(details) =
                        self.details.as_mut().filter(|details| details.key == key)
                {
                    details.task = Some(task);
                } else if let Some(state) = self.conversation_states.get_mut(&conversation)
                    && state.task_request_id == request_id
                    && round_current
                    && let Some(details) =
                        state.details.as_mut().filter(|details| details.key == key)
                {
                    details.task = Some(task);
                }
            }
            Update::Diff {
                conversation,
                plan,
                generation: _,
                request_id,
                task,
                reply,
            } => {
                if conversation == self.conversation
                    && request_id == self.diff_request_id
                    && self.view.plan.as_ref() == plan.as_ref()
                    && let Some(diff) = self.diff.as_mut().filter(|diff| diff.task == task)
                {
                    diff.load(reply);
                } else if let Some(state) = self.conversation_states.get_mut(&conversation)
                    && state.diff_request_id == request_id
                    && let Some(diff) = state.diff.as_mut().filter(|diff| diff.task == task)
                {
                    diff.load(reply);
                }
            }
            Update::SendResult {
                conversation,
                generation: _,
                request_id,
                sent_text,
                result,
            } => {
                if conversation == self.conversation && request_id == self.send_request_id {
                    self.sending = false;
                    self.pending_send = None;
                    match result {
                        Ok(_) => {
                            self.failed_send = None;
                        }
                        Err(error) => {
                            if self.input.is_empty() {
                                self.input = InputBuffer::with_text(sent_text);
                                self.notice = Some(format!("✗ {error} (draft restored)"));
                            } else {
                                self.failed_send = Some(sent_text);
                                self.notice = Some(format!("✗ {error} (unsent message preserved)"));
                            }
                        }
                    }
                } else if let Some(state) = self.conversation_states.get_mut(&conversation)
                    && state.send_request_id == request_id
                {
                    state.sending = false;
                    state.pending_send = None;
                    match result {
                        Ok(_) => {
                            state.failed_send = None;
                        }
                        Err(_) => {
                            if state.input.is_empty() {
                                state.input = InputBuffer::with_text(sent_text);
                            } else {
                                state.failed_send = Some(sent_text);
                            }
                        }
                    }
                }
            }
            Update::CommentResult {
                conversation,
                plan,
                generation: _,
                request_id,
                editor,
                result,
            } => {
                if conversation == self.conversation && request_id == self.comment_request_id {
                    self.sending_comment = false;
                    self.pending_comment = None;
                    match result {
                        Ok(_) => {
                            self.failed_comment = None;
                            self.remove_comment_draft(&editor.target);
                        }
                        Err(error) => {
                            if let Some(active_editor) = &mut self.editor {
                                if active_editor.target == editor.target {
                                    if active_editor.input.is_empty() {
                                        active_editor.input = editor.input;
                                    } else {
                                        self.save_comment_draft(
                                            &editor.target,
                                            editor.input.clone(),
                                        );
                                        self.failed_comment = Some(editor);
                                    }
                                } else {
                                    self.save_comment_draft(&editor.target, editor.input.clone());
                                    self.failed_comment = Some(editor);
                                }
                            } else {
                                self.editor = Some(editor);
                            }
                            self.notice = Some(format!("✗ {error} (comment draft preserved)"));
                        }
                    }
                } else if let Some(state) = self.conversation_states.get_mut(&conversation)
                    && state.comment_request_id == request_id
                {
                    state.sending_comment = false;
                    state.pending_comment = None;
                    match result {
                        Ok(_) => {
                            state.failed_comment = None;
                            let key =
                                Self::comment_key(&conversation, plan.as_deref(), &editor.target);
                            state.comment_drafts.remove(&key);
                            state.comment_draft_cursors.remove(&key);
                        }
                        Err(_) => {
                            if state.editor.is_none() {
                                state.editor = Some(editor);
                            } else {
                                let key = Self::comment_key(
                                    &conversation,
                                    plan.as_deref(),
                                    &editor.target,
                                );
                                state
                                    .comment_drafts
                                    .insert(key.clone(), editor.input.text.clone());
                                state
                                    .comment_draft_cursors
                                    .insert(key, (editor.input.cursor, editor.input.preferred_col));
                                state.failed_comment = Some(editor);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Runs a `/command` or sends a message.
    fn dispatch(&mut self, text: &str) {
        let text = text.to_owned();
        let conversation = self.conversation.clone();
        match text.as_str() {
            "" => {}
            "/plan" => {
                if self.view.plan.is_some()
                    || !self.view.tasks.is_empty()
                    || !self.view.plan_doc.is_empty()
                {
                    let pid = self.view.plan.clone().unwrap_or_else(|| "plan".into());
                    self.plan_view = Some(PlanView::new(pid));
                } else {
                    self.notice = Some("✗ there is no plan to view".into());
                }
            }
            "/accept" => match self.view.plan.clone().filter(|_| self.view.plan_proposed()) {
                Some(plan) => {
                    if !self.view.open_plan_comments().is_empty() {
                        self.notice =
                            Some("✗ send the draft comments (Review) and resolve the rest before accepting".into());
                    } else {
                        self.plan_view = None;
                        self.request(
                            Request::Accept { conversation, plan },
                            Some("plan accepted"),
                        );
                    }
                }
                None => self.notice = Some("✗ there is no plan to accept".into()),
            },
            "/reject" => match self.view.plan.clone().filter(|_| self.view.plan_proposed()) {
                Some(plan) => {
                    self.plan_view = None;
                    self.request(
                        Request::Reject {
                            conversation,
                            plan,
                            reason: None,
                        },
                        Some("plan rejected"),
                    );
                }
                None => self.notice = Some("✗ there is no plan to reject".into()),
            },
            "/resume" => match self
                .view
                .plan
                .clone()
                .filter(|_| self.view.plan_resumable())
            {
                Some(plan) => {
                    self.request(Request::Resume { conversation, plan }, Some("plan resumed"))
                }
                None => self.notice = Some("✗ there is nothing to resume".into()),
            },
            "/cancel" => self.request(Request::Cancel { conversation }, Some("cancelled")),
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
            "/mode" => {
                // The header changes when the daemon says the mode changed.
                let next_auto = self.view.autonomous != Some(true);
                self.request(
                    Request::Mode {
                        conversation,
                        autonomous: next_auto,
                    },
                    None,
                );
            }
            "/mode autonomous" | "/mode supervised" => {
                let next_auto = text.ends_with("autonomous");
                self.request(
                    Request::Mode {
                        conversation,
                        autonomous: next_auto,
                    },
                    None,
                );
            }
            "/unblock" => self.open_unblock(None),
            _ if text.starts_with("/unblock ") => {
                self.open_unblock(Some(text.trim_start_matches("/unblock ").trim().to_owned()));
            }
            "/config" => self.open_config(),
            "/project" => self.open_projects_picker(),
            _ if text.starts_with("/project ") => {
                let proj = text.trim_start_matches("/project ").trim().to_string();
                if !proj.is_empty() {
                    self.open_project(&proj);
                }
            }
            _ => {
                self.scroll = 0;
                self.request(Request::Send { conversation, text }, None);
            }
        }
    }

    /// Opens the way out for a stopped step: the named one, else the
    /// selected one, else the first that is stopped.
    pub fn open_unblock(&mut self, named: Option<String>) {
        let Some(plan) = self.view.plan.clone() else {
            self.notice = Some("✗ there is no plan, so nothing to unblock".into());
            return;
        };
        let selected = self.selected.split_once(':').map(|(_, id)| id.to_owned());
        let wanted = named.or(selected);
        let task = wanted
            .as_deref()
            .and_then(|id| self.view.tasks.iter().find(|t| t.id == id))
            .filter(|t| unblock::is_blocked(&t.status))
            .or_else(|| {
                self.view
                    .tasks
                    .iter()
                    .find(|t| unblock::is_blocked(&t.status) && t.status != "skipped")
            })
            .or_else(|| {
                self.view
                    .tasks
                    .iter()
                    .find(|t| unblock::is_blocked(&t.status))
            });
        match task {
            Some(task) => self.unblock = Some(unblock::Unblock::new(&plan, task)),
            None => self.notice = Some("✗ no step is stopped; nothing to unblock".into()),
        }
    }

    /// The user chose a way out: ask for a hint first where it takes one,
    /// else ask the daemon at once.
    fn unblock_choice(&mut self, state: unblock::Unblock, choice: unblock::Choice) {
        if choice.asks_text {
            let action = serde_json::to_value(&choice.action)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default();
            let target = format!("unblock:{action}:{}:{}", state.plan, state.task);
            let input = self.get_comment_draft(&target);
            self.editor = Some(Editor { target, input });
            return;
        }
        let client = self.client.clone();
        let updates = self.updates.clone();
        let conversation = self.conversation.clone();
        std::thread::spawn(move || {
            let notice = match recover(
                &client,
                &conversation,
                &state.plan,
                &state.task,
                choice.action,
                None,
            ) {
                Ok(notice) => format!("{}: {notice}", state.task),
                Err(error) => format!("✗ {error}"),
            };
            let _ = updates.send(Update::Notice(notice));
        });
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

    /// Adds a connected project to the screen, once; its conversations are
    /// listed in the background from then on. Returns its place.
    pub fn register_project(&mut self, conn: workspace::ProjectConn) -> usize {
        if let Some(index) = self.projects.iter().position(|p| p.path == conn.path) {
            return index;
        }
        let _ = projects::Projects::from_env().and_then(|list| list.remember(&conn.path));
        let (client, path, updates) =
            (conn.client.clone(), conn.path.clone(), self.updates.clone());
        std::thread::spawn(move || {
            loop {
                if let Ok(list) = client.call(&Request::Conversations) {
                    let mine = list
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|c| c["project"] == path.to_string_lossy().as_ref())
                        .cloned()
                        .collect();
                    let update = Update::ProjectStatus {
                        path: path.clone(),
                        conversations: mine,
                    };
                    // The screen is gone: so is the reason to look.
                    if updates.send(update).is_err() {
                        return;
                    }
                }
                std::thread::sleep(Duration::from_secs(3));
            }
        });
        self.projects.push(conn);
        self.projects.len() - 1
    }

    /// Makes a connected project the one on screen, showing `conversation`
    /// or the project's latest.
    pub fn switch_project(&mut self, index: usize, conversation: Option<String>) {
        let Some(conn) = self.projects.get(index).cloned() else {
            return;
        };
        let conversation = match conversation {
            Some(id) => id,
            None => match workspace::latest_or_new(&conn) {
                Ok(id) => id,
                Err(error) => {
                    self.notice = Some(format!("✗ {}: {error}", conn.name));
                    return;
                }
            },
        };
        if self.active_project != index && self.history_path.is_some() {
            self.load_history(conn.client.state_dir());
        }
        if self.client.state_dir() != conn.client.state_dir() {
            if let Some(mut editor) = self.config_editor.take() {
                editor.visible = false;
                self.config_drafts.insert(
                    self.client.state_dir().to_string_lossy().into_owned(),
                    editor,
                );
            }
            self.config_editor = self
                .config_drafts
                .remove(conn.client.state_dir().to_string_lossy().as_ref());
        }
        self.active_project = index;
        self.client = conn.client.clone();
        self.project = conn.name.clone();
        self.open(conversation);
    }

    /// Opens a project by directory, or switches to it if it is open. The
    /// directory must be trusted; if it is not, the user is asked first.
    pub fn open_project(&mut self, path: &str) {
        let path = expand_home(path);
        if let Ok(canonical) = path.canonicalize()
            && let Some(index) = self.projects.iter().position(|p| p.path == canonical)
        {
            self.switch_project(index, None);
            return;
        }
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            let update = match trust::Registry::from_env() {
                Err(error) => Update::Notice(format!("✗ {error}")),
                Ok(registry) => match workspace::connect(&path, &registry) {
                    Ok(conn) => match workspace::latest_or_new(&conn) {
                        Ok(id) => Update::ProjectReady {
                            conn: Box::new(conn),
                            conversation: Some(id),
                        },
                        Err(error) => Update::Notice(format!("✗ {error}")),
                    },
                    Err(workspace::ConnectError::NotTrusted(path)) => Update::ProjectTrust(path),
                    Err(workspace::ConnectError::Failed(error)) => {
                        Update::Notice(format!("✗ {error}"))
                    }
                },
            };
            let _ = updates.send(update);
        });
    }

    /// The projects open here and those opened before, to pick from.
    pub fn open_projects_picker(&mut self) {
        let mut items = Vec::new();
        for (index, project) in self.projects.iter().enumerate() {
            let (working, attention) = project.badges();
            let mut title = format!("{} · {}", project.name, project.path.display());
            if index == self.active_project {
                title.push_str(" (current)");
            }
            if working > 0 {
                title.push_str(&format!("  ●{working}"));
            }
            if attention > 0 {
                title.push_str(&format!("  !{attention}"));
            }
            items.push(palette::ProjectItem {
                path: project.path.to_string_lossy().into_owned(),
                title,
            });
        }
        if let Ok(list) = projects::Projects::from_env() {
            for path in list.list() {
                if !self.projects.iter().any(|p| p.path == path) {
                    items.push(palette::ProjectItem {
                        title: format!(
                            "{} · {} (not open)",
                            workspace::name_of(&path),
                            path.display()
                        ),
                        path: path.to_string_lossy().into_owned(),
                    });
                }
            }
        }
        let mut picker = Picker::new(Kind::Projects);
        picker.projects = items;
        self.picker = Some(picker);
    }

    /// Keys while the user is asked whether to trust a directory to open.
    fn project_trust_key(&mut self, key: event::KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                if let Some(path) = self.project_trust.take() {
                    match trust::Registry::from_env().and_then(|r| r.trust(&path)) {
                        Ok(()) => self.open_project(&path.to_string_lossy()),
                        Err(error) => self.notice = Some(format!("✗ {error}")),
                    }
                }
            }
            KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Char('q') => {
                self.project_trust = None;
            }
            _ => {}
        }
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
                        self.plan_view = None;
                        self.focus = Focus::Graph;
                        self.select_node(key);
                        true
                    }
                    Some(Action::Show(entry)) => {
                        self.details = None;
                        self.diff = None;
                        self.plan_view = None;
                        self.focus = Focus::Chat;
                        self.found = Some(entry);
                        // Brought to the top at the next draw.
                        self.scroll = usize::MAX;
                        true
                    }
                    Some(Action::SwitchConversation(id)) => {
                        if id != self.conversation {
                            // A conversation of another open project switches to it.
                            let owner = self.projects.iter().position(|p| {
                                p.conversations.iter().any(|c| c["id"] == id.as_str())
                            });
                            match owner {
                                Some(index) if index != self.active_project => {
                                    self.switch_project(index, Some(id));
                                }
                                _ => self.open(id),
                            }
                        }
                        true
                    }
                    Some(Action::OpenProject(path)) => {
                        self.open_project(&path);
                        true
                    }
                    Some(Action::Address(recipient)) => {
                        self.details = None;
                        self.diff = None;
                        self.plan_view = None;
                        self.focus = Focus::Chat;
                        let formatted = format!("@{recipient} ");
                        let text = self.input.text.trim_end();
                        if text.is_empty() {
                            self.input.clear();
                            self.input.insert_str(&formatted);
                        } else if text.ends_with('@') {
                            self.input.text.truncate(text.len() - 1);
                            self.input.cursor = self.input.text.len();
                            self.input.insert_str(&formatted);
                        } else {
                            self.input.insert_str(&format!(" @{recipient} "));
                        }
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
            Command::Reject => self.command("/reject"),
            Command::Plan => self.command("/plan"),
            Command::Resume => self.command("/resume"),
            Command::Cancel => self.command("/cancel"),
            Command::Pause => self.command("/pause"),
            Command::Continue => self.command("/continue"),
            Command::Autonomous => self.command("/mode autonomous"),
            Command::Supervised => self.command("/mode supervised"),
            Command::NewConversation => self.new_conversation(),
            Command::SwitchConversation => self.open_conversation_picker(),
            Command::Find => self.picker = Some(Picker::new(Kind::Agents)),
            Command::Search => self.picker = Some(Picker::new(Kind::History)),
            Command::Details => {
                self.open_details(selected);
            }
            Command::ConversationDetails => {
                self.open_details("conversation".into());
            }
            Command::AddressMessage => {
                self.picker = Some(Picker::new(Kind::Recipients));
            }
            Command::Diff => self.open_diff(&selected),
            Command::Comment => {
                let target = selected;
                let input = self.get_comment_draft(&target);
                self.editor = Some(Editor { target, input });
            }
            Command::Filter => self.filter = self.filter.next(),
            Command::Help => {
                let help = self.help_here();
                self.show_help(help);
            }
            Command::Config => self.open_config(),
            Command::Projects => self.open_projects_picker(),
            Command::Quit => return false,
        }
        true
    }

    pub fn handle_plan_action(&mut self, action: PlanAction) {
        match action {
            PlanAction::Close => {
                self.plan_view = None;
            }
            PlanAction::Accept => {
                if !self.view.open_plan_comments().is_empty() {
                    self.notice =
                        Some("✗ resolve all plan comments before accepting the plan".into());
                } else {
                    self.plan_view = None;
                    self.command("/accept");
                }
            }
            PlanAction::Reject => {
                // The reason is optional: an empty one rejects as is.
                if let Some(plan) = self.view.plan.clone().filter(|_| self.view.plan_proposed()) {
                    let target = format!("reject:{plan}");
                    let input = self.get_comment_draft(&target);
                    self.editor = Some(Editor { target, input });
                } else {
                    self.notice = Some("✗ there is no plan to reject".into());
                }
            }
            PlanAction::None => {}
            PlanAction::CommentLine { line, .. } => {
                if let Some(plan) = &self.view.plan {
                    let target = format!("plan:{plan}:line:{line}");
                    let input = self.get_comment_draft(&target);
                    self.editor = Some(Editor { target, input });
                }
            }
            PlanAction::CommentWholePlan => {
                if let Some(plan) = &self.view.plan {
                    let target = format!("plan:{plan}");
                    let input = self.get_comment_draft(&target);
                    self.editor = Some(Editor { target, input });
                }
            }
            PlanAction::SubmitReview => {
                if let Some(plan) = self.view.plan.clone() {
                    let conversation = self.conversation.clone();
                    self.request(
                        Request::ReviewComments { conversation, plan },
                        Some("comments sent to the agent for review"),
                    );
                }
            }
            PlanAction::ToggleResolveComment { id, next_status } => {
                self.set_comment(id, &next_status);
            }
        }
    }

    /// Returns false when the user quits.
    /// The graph and task panels are not shown before work runs, so focus
    /// cannot rest on them then.
    fn settle_focus(&mut self) {
        if self.view.graph_hidden() && self.focus != Focus::Chat {
            self.focus = Focus::Chat;
        }
    }

    /// The header as drawn, which is also what a click on it is read from.
    pub fn header_line(&self, width: u16) -> Line<'static> {
        let panel_focus = match self.focus {
            Focus::Chat => PanelFocus::Chat,
            Focus::Graph => PanelFocus::Graph,
        };
        let label = match workspace::others_summary(&self.projects, self.active_project) {
            Some(others) => format!("{} ({others})", self.project),
            None => self.project.clone(),
        };
        view::render_header_with(
            &label,
            &self.conversation,
            self.view.autonomous,
            &self.header_status(),
            panel_focus,
            width,
            self.view.controls(),
        )
    }

    /// The header's status text: the stage, then what is going on.
    fn header_status(&self) -> String {
        let status = match (self.view.orchestrator_activity(), self.view.idle) {
            (Some(activity), _) => activity,
            (None, _) if self.view.paused => "paused".into(),
            (None, true) => "ready".into(),
            (None, false) => "tasks in progress…".into(),
        };
        let stage = self.view.stage();
        match stage {
            view::Stage::Fresh => status,
            // The way out is one click away whenever something is stuck.
            view::Stage::Blocked => format!("{} · {status} [Unblock]", stage.label()),
            _ => format!("{} · {status}", stage.label()),
        }
    }

    fn config_request(&self, request: Request, save: bool) {
        let client = self.client.clone();
        let updates = self.updates.clone();
        let project = self.client.state_dir().to_string_lossy().into_owned();
        std::thread::spawn(move || {
            let reply = client.call(&request).map_err(|e| e.to_string());
            let _ = updates.send(Update::Config {
                project,
                save,
                reply,
            });
        });
    }

    fn open_config(&mut self) {
        if let Some(editor) = &mut self.config_editor {
            editor.visible = true;
        } else {
            self.config_request(Request::GetConfig, false);
        }
    }

    pub fn key(&mut self, key: event::KeyEvent) -> bool {
        if let Some(editor) = self.config_editor.as_mut().filter(|e| e.visible) {
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
                if !editor.pending {
                    if let Some(toml) = editor.source() {
                        editor.pending = true;
                        editor.status = "Applying...".into();
                        self.config_request(Request::SetConfig { toml }, true);
                    } else if let Some(index) = editor.fields.iter().position(|f| f.error.is_some())
                    {
                        editor.selected = index;
                    }
                }
            } else {
                editor.key(key);
            }
            return true;
        }
        self.settle_focus();
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
        let editing = self.editor.is_some()
            || self.picker.is_some()
            || self
                .details
                .as_ref()
                .is_some_and(|details| details.searching)
            || (self.focus == Focus::Chat
                && self.help.is_none()
                && self.plan_view.is_none()
                && self.details.is_none()
                && self.diff.is_none()
                && !self.input.is_empty());
        if key.code == KeyCode::Char('q')
            && !control
            && !alt
            && !editing
            && self.plan_view.is_none()
        {
            return false;
        }

        if self.trust_prompt.is_some() {
            match key.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.accept_trust();
                    return true;
                }
                KeyCode::Esc
                | KeyCode::Char('n')
                | KeyCode::Char('N')
                | KeyCode::Char('q')
                | KeyCode::Char('Q') => {
                    return false;
                }
                _ => return true,
            }
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
                KeyCode::Char('u') if control => {
                    self.help_scroll = self.help_scroll.saturating_sub(3);
                    return true;
                }
                KeyCode::Char('d') if control => {
                    self.help_scroll = (self.help_scroll + 3).min(max_scroll);
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
        if self.project_trust.is_some() {
            self.project_trust_key(key);
            return true;
        }
        if let Some(modal) = &mut self.unblock {
            match modal.key(key) {
                unblock::Outcome::None => {}
                unblock::Outcome::Close => self.unblock = None,
                unblock::Outcome::Choose(choice) => {
                    if let Some(state) = self.unblock.take() {
                        self.unblock_choice(state, choice);
                    }
                }
            }
            return true;
        }
        if self.picker.is_some() {
            return self.picker_key(key);
        }
        if self.editor.is_some() {
            self.editor_key(key);
            return true;
        }
        if let Some(pv) = &mut self.plan_view {
            let action = pv.key(key, &self.view);
            self.handle_plan_action(action);
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

        if self
            .confirmation
            .as_ref()
            .is_some_and(|c| c.status == ConfirmationStatus::Pending)
        {
            match key.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    self.confirm_action();
                    return true;
                }
                KeyCode::Esc | KeyCode::Char('n') | KeyCode::Char('N') => {
                    self.reject_confirmation();
                    return true;
                }
                _ => {}
            }
        }

        // Base layout global shortcuts
        match key.code {
            KeyCode::Char('p') if control => {
                self.picker = Some(Picker::new(Kind::Commands));
                return true;
            }
            // Ctrl+M is Enter to a terminal, so the toggle is Ctrl+T.
            KeyCode::Char('t') if control => {
                self.command("/mode");
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
            KeyCode::Char('o') if control => {
                self.open_conversation_picker();
                return true;
            }
            KeyCode::Char('i') if control => {
                self.open_details("conversation".into());
                return true;
            }
            KeyCode::Char('l') if control => {
                self.command("/plan");
                return true;
            }
            KeyCode::Char('g') if control => {
                self.open_projects_picker();
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
                    let text = std::mem::take(&mut self.history_draft);
                    let cursor = self.history_draft_cursor.min(text.len());
                    let mut buf = InputBuffer::with_text(text);
                    buf.cursor = cursor;
                    buf.preferred_col = self.history_draft_preferred_col;
                    self.input = buf;
                } else if !self.input.is_empty() {
                    self.history_draft = self.input.text.clone();
                    self.history_draft_cursor = self.input.cursor;
                    self.history_draft_preferred_col = self.input.preferred_col;
                    self.input.clear();
                    self.input_shown = 0;
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
            // Vim half-page scrolling while there is no draft to edit; with
            // text in the box Ctrl+U keeps deleting to the start of the line.
            KeyCode::Char('u') if control && self.input.is_empty() => self.scroll += 5,
            KeyCode::Char('d') if control && self.input.is_empty() => {
                self.scroll = self.scroll.saturating_sub(5);
            }
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
            KeyCode::Char('@') if !control && self.input.is_empty() => {
                self.picker = Some(Picker::new(Kind::Recipients));
            }
            KeyCode::Char(ch) if !control => self.input.insert_char(ch),
            _ => {}
        }
        true
    }

    fn graph_key(&mut self, key: event::KeyEvent) -> bool {
        self.selected = graph::task_node_key(&self.selected);
        let graph = Graph::new_with_viewport(
            &self.view,
            self.graph_filter.as_deref(),
            &self.graph_collapsed,
            self.graph_grouping,
            self.graph_viewport,
        );
        if matches!(key.code, KeyCode::PageUp | KeyCode::PageDown) {
            let height = self.graph_viewport.map_or(10, |(_, height)| height);
            self.scroll_graph_node(
                &graph,
                key.code == KeyCode::PageDown,
                height.saturating_sub(2).max(1),
            );
            return true;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL)
            && matches!(key.code, KeyCode::Char('u' | 'd'))
        {
            let height = self.graph_viewport.map_or(10, |(_, height)| height);
            self.scroll_graph_node(&graph, key.code == KeyCode::Char('d'), (height / 2).max(1));
            return true;
        }
        let step = match key.code {
            KeyCode::Left => Some((-1, 0)),
            KeyCode::Right => Some((1, 0)),
            KeyCode::Up => Some((0, -1)),
            KeyCode::Down => Some((0, 1)),
            _ => None,
        };
        if let Some((dx, dy)) = step {
            if graph.node(&self.selected).is_none()
                || !graph.node(&self.selected).is_some_and(|n| n.visible)
            {
                if let Some((_, id)) = self.selected.split_once(':') {
                    let worker_key = format!("worker:{id}");
                    if graph.node(&worker_key).is_some_and(|n| n.visible) {
                        self.selected = worker_key;
                    } else if let Some(vis) = graph
                        .nodes
                        .iter()
                        .find(|n| n.visible && n.key == ORCHESTRATOR)
                    {
                        self.selected = vis.key.clone();
                    } else if let Some(first_vis) = graph.nodes.iter().find(|n| n.visible) {
                        self.selected = first_vis.key.clone();
                    } else {
                        self.selected = ORCHESTRATOR.into();
                    }
                } else if let Some(vis) = graph
                    .nodes
                    .iter()
                    .find(|n| n.visible && n.key == ORCHESTRATOR)
                {
                    self.selected = vis.key.clone();
                } else if let Some(first_vis) = graph.nodes.iter().find(|n| n.visible) {
                    self.selected = first_vis.key.clone();
                } else {
                    self.selected = ORCHESTRATOR.into();
                }
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
            KeyCode::Char('p') => self.command("/plan"),
            KeyCode::Char('u') => self.open_unblock(None),
            KeyCode::Char('z') | KeyCode::Char(' ') => self.toggle_selected_collapse(),
            KeyCode::Char('g') => self.graph_grouping = self.graph_grouping.next(),
            KeyCode::Char('f') => self.filter = self.filter.next(),
            KeyCode::Char('d') => {
                let key = self.selected.clone();
                self.open_diff(&key);
            }
            KeyCode::Char('c') => {
                let target = self.selected.clone();
                let input = self.get_comment_draft(&target);
                self.editor = Some(Editor { target, input });
            }
            KeyCode::Enter => {
                self.details = Some(Details::new(self.selected.clone()));
                self.fetch_task();
            }
            _ => {}
        }
        true
    }

    fn scroll_graph_node(&mut self, graph: &Graph, down: bool, amount: u16) {
        if let Some(node) = graph.node(&self.selected) {
            let height = self.graph_viewport.map_or(10, |(_, height)| height);
            let max = node
                .y
                .saturating_add(node.height)
                .saturating_sub(height)
                .max(node.y);
            self.offset.1 = if down {
                self.offset.1.saturating_add(amount).clamp(node.y, max)
            } else {
                self.offset.1.saturating_sub(amount).clamp(node.y, max)
            };
        }
    }

    pub fn select_node(&mut self, key: String) {
        self.selected = graph::task_node_key(&key);
    }

    pub fn toggle_selected_collapse(&mut self) {
        let task_id = if let Some((_, id)) = self.selected.split_once(':') {
            id.to_string()
        } else {
            return;
        };
        if self.graph_collapsed.contains(&task_id) {
            self.graph_collapsed.remove(&task_id);
        } else {
            self.graph_collapsed.insert(task_id);
        }
    }

    pub fn set_graph_filter(&mut self, query: Option<String>) {
        self.graph_filter = query;
        let graph = Graph::new_with_viewport(
            &self.view,
            self.graph_filter.as_deref(),
            &self.graph_collapsed,
            self.graph_grouping,
            self.graph_viewport,
        );
        if !graph
            .nodes
            .iter()
            .any(|n| n.visible && n.key == self.selected)
        {
            if let Some((_, id)) = self.selected.split_once(':') {
                let worker_key = format!("worker:{id}");
                if graph.node(&worker_key).is_some_and(|n| n.visible) {
                    self.selected = worker_key;
                } else if let Some(vis) = graph
                    .nodes
                    .iter()
                    .find(|n| n.visible && n.key == ORCHESTRATOR)
                {
                    self.selected = vis.key.clone();
                } else if let Some(first_vis) = graph.nodes.iter().find(|n| n.visible) {
                    self.selected = first_vis.key.clone();
                } else {
                    self.selected = ORCHESTRATOR.into();
                }
            } else if let Some(vis) = graph
                .nodes
                .iter()
                .find(|n| n.visible && n.key == ORCHESTRATOR)
            {
                self.selected = vis.key.clone();
            } else if let Some(first_vis) = graph.nodes.iter().find(|n| n.visible) {
                self.selected = first_vis.key.clone();
            } else {
                self.selected = ORCHESTRATOR.into();
            }
        }
    }

    pub fn open_conversation_picker(&mut self) {
        let mut items = Vec::new();

        if let Ok(Value::Array(list)) = self.client.call(&Request::Conversations) {
            for val in list {
                let project = val["project"].as_str().unwrap_or_default();
                let project_basename = std::path::Path::new(project)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(project);
                if !project.is_empty()
                    && project != self.project
                    && project_basename != self.project
                {
                    continue;
                }
                let id = val["id"].as_str().unwrap_or_default().to_string();
                if id.is_empty() {
                    continue;
                }
                let status = val["status"].as_str().unwrap_or("idle");
                let total = val["tasks_total"].as_u64().unwrap_or(0);
                let done = val["tasks_done"].as_u64().unwrap_or(0);
                let active_mark = if id == self.conversation {
                    " (current)"
                } else {
                    ""
                };
                let title = if total > 0 {
                    format!("{id}{active_mark} · {status} · {done}/{total} tasks")
                } else {
                    format!("{id}{active_mark} · {status}")
                };
                items.push(palette::ConversationItem {
                    id,
                    project: project.to_string(),
                    title,
                });
            }
        }

        for (index, project) in self.projects.iter().enumerate() {
            if index == self.active_project {
                continue;
            }
            for conversation in &project.conversations {
                let id = conversation["id"].as_str().unwrap_or_default().to_string();
                if id.is_empty() {
                    continue;
                }
                let status = conversation["status"].as_str().unwrap_or("idle");
                items.push(palette::ConversationItem {
                    title: format!("{} · {id} · {status}", project.name),
                    project: project.path.to_string_lossy().into_owned(),
                    id,
                });
            }
        }
        for id in self.conversation_states.keys() {
            if !items.iter().any(|item| &item.id == id) {
                let active_mark = if id == &self.conversation {
                    " (current)"
                } else {
                    ""
                };
                items.push(palette::ConversationItem {
                    id: id.clone(),
                    project: self.project.clone(),
                    title: format!("{id}{active_mark} · saved state"),
                });
            }
        }
        if !items.iter().any(|item| item.id == self.conversation) && !self.conversation.is_empty() {
            items.insert(
                0,
                palette::ConversationItem {
                    id: self.conversation.clone(),
                    project: self.project.clone(),
                    title: format!("{} (current) · active", self.conversation),
                },
            );
        }

        let mut picker = Picker::new(Kind::Conversations);
        picker.conversations = items;
        self.picker = Some(picker);
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
                    self.save_comment_draft(&editor.target, editor.input);
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

        if details.searching {
            match key.code {
                KeyCode::Esc => {
                    details.searching = false;
                    details.query.clear();
                    details.scroll = 0;
                }
                KeyCode::Enter => {
                    details.searching = false;
                }
                KeyCode::Backspace => {
                    details.query.pop();
                    details.scroll = 0;
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    details.query.push(c);
                    details.scroll = 0;
                }
                _ => {}
            }
            return;
        }

        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        let comment = details.selected_comment(&self.view);
        let on_comments = details.tab == COMMENTS;
        let on_checks = details.tab == CHECKS;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                details.save_current_tab();
                self.details_scroll_memory
                    .extend(details.scroll_memory.clone());
                self.details_query_memory
                    .extend(details.query_memory.clone());
                self.details = None;
                return;
            }
            KeyCode::Char('/') => {
                details.searching = true;
                details.query.clear();
                details.scroll = 0;
                return;
            }
            KeyCode::Char('v') => {
                details.raw = !details.raw;
                return;
            }
            KeyCode::Enter if details.tab == 0 => {
                details.expand_prompts = !details.expand_prompts;
                return;
            }
            KeyCode::Char(c @ '1'..='5') => {
                if let Some(tab) = details.nth_tab((c as usize) - ('1' as usize)) {
                    details.switch_tab(tab);
                }
            }
            KeyCode::Char('?') => self.show_help(Help::Details),
            KeyCode::Char('c') => {
                // On the Checks tab a comment belongs to the selected check.
                let target = match (details.selected_check(), details.task_id()) {
                    (Some(check), Some(task)) => format!("check:{task}:{check}"),
                    _ => details.key.clone(),
                };
                let input = self.get_comment_draft(&target);
                self.editor = Some(Editor { target, input });
            }
            KeyCode::Down if on_checks => {
                details.check = (details.check + 1).min(details.check_count().saturating_sub(1));
                details.reveal_check = true;
            }
            KeyCode::Up if on_checks => {
                details.check = details.check.saturating_sub(1);
                details.reveal_check = true;
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
            KeyCode::Char('d') if control => details.scroll += 10,
            KeyCode::Char('u') if control => {
                details.follow = false;
                details.scroll = details.scroll.saturating_sub(10);
            }
            KeyCode::Char('d') => {
                let key = details.key.clone();
                self.open_diff(&key);
                return;
            }
            KeyCode::Right | KeyCode::Tab => {
                details.switch_tab(details.neighbour_tab(1));
            }
            KeyCode::Left | KeyCode::BackTab => {
                details.switch_tab(details.neighbour_tab(-1));
            }
            KeyCode::Down => details.scroll += 1,
            KeyCode::Up => {
                details.follow = false;
                details.scroll = details.scroll.saturating_sub(1);
            }
            KeyCode::PageDown => details.scroll += 20,
            KeyCode::PageUp => {
                details.follow = false;
                details.scroll = details.scroll.saturating_sub(20);
            }
            KeyCode::Home => {
                details.follow = false;
                details.scroll = 0;
            }
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
        if let Some(editor) = self.config_editor.as_mut().filter(|e| e.visible) {
            editor.paste(text);
            return;
        }
        if self.help.is_some() || self.trust_prompt.is_some() {
            return;
        }
        if text.len() > editor::MAX_PASTE_BYTES {
            self.notice = Some("Pasted text truncated to 64 KB".into());
        }
        if let Some(picker) = &mut self.picker {
            let clean_text = sanitize_single_line_paste(text);
            picker.query.push_str(&clean_text);
            picker.selected = 0;
        } else if let Some(editor) = &mut self.editor {
            let clean_text = sanitize_paste(text);
            editor.input.insert_str(&clean_text);
        } else if self.focus == Focus::Chat
            && self.plan_view.is_none()
            && self.diff.is_none()
            && self.details.is_none()
        {
            let clean_text = sanitize_paste(text);
            self.input.insert_str(&clean_text);
        }
    }

    pub fn draw(&mut self, frame: &mut Frame) {
        self.settle_focus();
        self.terminal_area = frame.area();
        self.graph_area = None;
        if draw_min_term_guard(frame, frame.area()) {
            return;
        }
        if let Some(editor) = self.config_editor.as_ref().filter(|e| e.visible) {
            editor.draw(frame);
            return;
        }
        if let Some(pv) = &mut self.plan_view {
            let theme = Theme::from_env();
            pv.draw(frame, frame.area(), &self.view, &theme);
        } else if let Some(diff) = &mut self.diff {
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
                        .title(if let Some(rest) = editor.target.strip_prefix("unblock:") {
                            let task = rest.rsplit(':').next().unwrap_or_default();
                            let action = rest.split(':').next().unwrap_or_default();
                            format!(
                                " {} {task} · optional text · Enter send · Alt+Enter newline · Esc cancel ",
                                action.replace('_', " ")
                            )
                        } else if let Some(plan) = editor.target.strip_prefix("reject:") {
                            format!(
                                " reject plan {plan} · optional reason · Enter reject · Alt+Enter newline · Esc cancel "
                            )
                        } else {
                            format!(
                                " comment on {} · Enter send · Alt+Enter newline · Esc cancel ",
                                target_name(&editor.target)
                            )
                        }),
                ),
                box_area,
            );
            if self.help.is_none()
                && self.picker.is_none()
                && self.trust_prompt.is_none()
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
            picker.draw_with_cursor(
                frame,
                &self.view,
                self.help.is_none() && self.trust_prompt.is_none(),
            );
        }
        if let Some(help) = self.help {
            let max_scroll = help.max_scroll(frame.area());
            self.help_scroll = self.help_scroll.min(max_scroll);
            help.draw_scroll(frame, self.help_scroll);
        }
        if let Some(conf) = &self.confirmation
            && conf.status == ConfirmationStatus::Pending
            && self.picker.is_none()
            && self.help.is_none()
            && self.trust_prompt.is_none()
        {
            let area = frame.area();
            let height = 5u16;
            if area.height >= height + 2 && area.width >= 20 {
                let box_area = Rect::new(
                    area.x + 2,
                    area.bottom().saturating_sub(height + 2),
                    area.width.saturating_sub(4),
                    height,
                );
                frame.render_widget(Clear, box_area);
                let lines = vec![
                    Line::from(vec![
                        Span::styled(
                            " Confirmation: ",
                            Style::default()
                                .fg(Color::Yellow)
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::raw(&conf.title),
                    ]),
                    Line::from(vec![Span::styled(
                        format!(" Scope: {}", conf.scope_hash),
                        Style::default().fg(Color::DarkGray),
                    )]),
                    Line::from(vec![Span::styled(
                        " [Enter / y] Confirm   [Esc / n] Reject",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )]),
                ];
                frame.render_widget(
                    Paragraph::new(lines).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .border_style(Style::default().fg(Color::Yellow))
                            .title(" Confirmation required "),
                    ),
                    box_area,
                );
            }
        }
        if let Some(path) = &self.project_trust {
            let area = frame.area();
            let width = area.width.saturating_sub(4).clamp(12, 76);
            let height = 8u16.min(area.height);
            let boxed = Rect::new(
                area.x + area.width.saturating_sub(width) / 2,
                area.y + area.height.saturating_sub(height) / 2,
                width,
                height,
            );
            frame.render_widget(Clear, boxed);
            let inner = usize::from(width.saturating_sub(4).max(1));
            let lines = vec![
                Line::styled(
                    truncate("Open this directory as a project?", inner),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                ),
                Line::raw(""),
                Line::styled(
                    truncate(&path.display().to_string(), inner),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Line::raw(""),
                Line::styled(
                    truncate(
                        "Its agents can read files, run commands and edit code there.",
                        inner,
                    ),
                    dim(),
                ),
                Line::raw(""),
                Line::styled(
                    truncate("[Enter / y] Trust and open   [Esc / n] Cancel", inner),
                    Style::default().fg(Color::Green),
                ),
            ];
            frame.render_widget(
                Paragraph::new(lines).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Yellow))
                        .title(" trust ")
                        .padding(ratatui::widgets::Padding::horizontal(1)),
                ),
                boxed,
            );
        }
        if let Some(modal) = &mut self.unblock {
            modal.draw(frame);
        }
        if let Some(dir) = &self.trust_prompt {
            let area = frame.area();
            let width = area.width.saturating_sub(4).clamp(12, 76);
            let height = 9u16.min(area.height);
            let box_area = Rect::new(
                area.x + area.width.saturating_sub(width) / 2,
                area.y + area.height.saturating_sub(height) / 2,
                width,
                height,
            );
            frame.render_widget(Clear, box_area);
            let inner_w = usize::from(width.saturating_sub(4).max(1));
            let lines = vec![
                Line::from(vec![Span::styled(
                    truncate("Do you trust the contents of this directory?", inner_w),
                    Style::default()
                        .fg(Color::Yellow)
                        .add_modifier(Modifier::BOLD),
                )]),
                Line::raw(""),
                Line::from(vec![Span::styled(
                    format!("  {}", truncate(dir, inner_w.saturating_sub(2))),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )]),
                Line::raw(""),
                Line::from(vec![Span::styled(
                    truncate(
                        "Agents can read files, run commands, and edit code in this folder.",
                        inner_w,
                    ),
                    dim(),
                )]),
                Line::raw(""),
                Line::from(vec![Span::styled(
                    truncate(
                        "[Enter / y] Trust this directory   [Esc / n / q] Quit",
                        inner_w,
                    ),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )]),
            ];
            frame.render_widget(
                Paragraph::new(lines).block(
                    Block::default()
                        .borders(Borders::ALL)
                        .border_style(Style::default().fg(Color::Yellow))
                        .title(" Workspace trust required "),
                ),
                box_area,
            );
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

        frame.render_widget(Paragraph::new(self.header_line(header.width)), header);
        let panel_focus = match self.focus {
            Focus::Chat => PanelFocus::Chat,
            Focus::Graph => PanelFocus::Graph,
        };

        let theme = Theme::from_env();
        if self.view.graph_hidden() && self.focus == Focus::Chat {
            self.draw_chat(frame, body, &theme);
            if self.view.activity.is_some() {
                let progress = self.view.progress();
                let width = progress
                    .iter()
                    .map(|line| line.width().saturating_add(4))
                    .max()
                    .unwrap_or(0)
                    .clamp(16, 48) as u16;
                let width = width.min(body.width);
                let height = progress.len().min(6) as u16 + 2;
                if body.width >= width + 2 && body.height >= height + 2 {
                    let overlay = Rect::new(
                        body.right().saturating_sub(width + 1),
                        body.y + 1,
                        width,
                        height,
                    );
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
                                .title(" now ")
                                .padding(ratatui::widgets::Padding::horizontal(1)),
                        ),
                        overlay,
                    );
                }
            }
        } else if body.width >= SIDE_BY_SIDE_BREAKPOINT {
            let chat_width = (body.width / 3).max(34);
            let [chat, sep_col, team] = Layout::horizontal([
                Constraint::Length(chat_width),
                Constraint::Length(1),
                Constraint::Min(1),
            ])
            .areas(body);
            self.draw_chat(frame, chat, &theme);
            let separator = theme.glyphs().vertical;
            let buf = frame.buffer_mut();
            for y in sep_col.top()..sep_col.bottom() {
                buf[(sep_col.x, y)]
                    .set_symbol(separator)
                    .set_style(theme.separator(false));
            }
            self.draw_graph(frame, team);
        } else if self.focus == Focus::Chat {
            self.draw_chat(frame, body, &theme);
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

    fn draw_chat(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        // Leave a column for the insertion cursor after a full line.
        let input_width = area.width.saturating_sub(3) as usize;
        self.input_width = input_width.max(1);
        let input_lines = visual_text_lines(&self.input.text, self.input_width);
        let max_input_height = area.height.saturating_sub(1).max(1);
        let input_height = (input_lines.len().clamp(1, 6) as u16 + 2).min(max_input_height);
        let [transcript, input] =
            Layout::vertical([Constraint::Min(1), Constraint::Length(input_height)]).areas(area);
        self.draw_transcript(frame, transcript, theme);

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
            && self.trust_prompt.is_none()
            && self.diff.is_none()
            && self.details.is_none();

        let fill = if focused {
            theme.raised()
        } else {
            theme.card()
        };
        let accent = if focused {
            theme.accent()
        } else {
            theme.faint()
        };
        let bar = theme.glyphs().bar;

        let buf = frame.buffer_mut();
        for y in input.top()..input.bottom() {
            for x in input.left()..input.right() {
                buf[(x, y)].set_style(fill);
            }
            if input.width > 0 {
                buf[(input.x, y)]
                    .set_symbol(bar)
                    .set_style(fill.patch(accent));
            }
        }

        if input.height > 0 && input.width > 2 {
            let title = " message ";
            let title_style = fill.patch(if focused { theme.accent() } else { theme.dim() });
            buf.set_string(input.x + 1, input.y, title, title_style);
        }

        let inner = Rect::new(
            input.x + 1,
            input.y + 1,
            input.width.saturating_sub(2),
            input.height.saturating_sub(1),
        );
        frame.render_widget(
            Paragraph::new(
                shown_lines
                    .iter()
                    .map(|line| Line::raw(line.clone()))
                    .collect::<Vec<_>>(),
            )
            .style(fill.patch(theme.text())),
            inner,
        );
        if focused && input.width >= 3 && input.height >= 2 {
            let min_x = input.x + 1;
            let max_x = input.right().saturating_sub(2).max(min_x);
            let min_y = input.y + 1;
            let max_y = input.bottom().saturating_sub(1).max(min_y);
            let cursor_x = (input.x + 1 + cursor_col).clamp(min_x, max_x);
            let visible_row = cursor_row
                .saturating_sub(self.input_shown)
                .min(visible_box_lines.saturating_sub(1));
            let cursor_y = (input.y + 1 + visible_row as u16).clamp(min_y, max_y);
            frame.set_cursor_position((cursor_x, cursor_y));
        }
    }

    fn draw_transcript(&mut self, frame: &mut Frame, area: Rect, theme: &Theme) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let width = area.width.saturating_sub(1);
        let glyphs = theme.glyphs();
        let mut cards: Vec<Row> = vec![];
        let mut top = None;

        for (index, card_entry) in self.view.cards.iter().enumerate() {
            let is_found = self.found == Some(index);
            match card_entry {
                TranscriptCard::Message(msg) => {
                    let is_user = msg.sender == "You" || msg.sender == "user";
                    let mut fill = if is_user {
                        theme.raised()
                    } else {
                        theme.card()
                    };
                    if is_found {
                        top = Some(cards.len());
                        fill = fill.add_modifier(Modifier::REVERSED);
                    }
                    let accent = theme.person(&msg.sender);
                    let head = vec![
                        Span::styled(
                            theme.clean(&msg.sender),
                            theme.person(&msg.sender).patch(theme.title()),
                        ),
                        Span::styled(format!(" {} ", glyphs.to), theme.faint()),
                        Span::styled(theme.clean(&msg.target), theme.person(&msg.target)),
                    ];
                    let right = format!(
                        "{} {} {}",
                        theme.clean(&msg.state),
                        glyphs.dot,
                        theme.clean(&msg.id)
                    );
                    cards.extend(card(
                        theme,
                        CardProps {
                            fill,
                            accent,
                            head,
                            right,
                            body: &msg.text,
                            max_body: usize::MAX,
                            width,
                        },
                    ));
                }
                TranscriptCard::Plan { plan, tasks, text } => {
                    let mut fill = theme.card();
                    if is_found {
                        top = Some(cards.len());
                        fill = fill.add_modifier(Modifier::REVERSED);
                    }
                    cards.extend(card(
                        theme,
                        CardProps {
                            fill,
                            accent: theme.accent(),
                            head: vec![Span::styled(
                                format!("Plan {}", theme.clean(plan)),
                                theme.accent().patch(theme.title()),
                            )],
                            right: format!("{tasks} tasks"),
                            body: text,
                            max_body: usize::MAX,
                            width,
                        },
                    ));
                }
                TranscriptCard::System {
                    title,
                    meta,
                    text,
                    tone,
                } => {
                    let mut fill = theme.card();
                    if is_found {
                        top = Some(cards.len());
                        fill = fill.add_modifier(Modifier::REVERSED);
                    }
                    let accent = match tone {
                        Tone::Active => theme.accent(),
                        Tone::Done => theme.status("completed"),
                        Tone::Failed => theme.error(),
                        Tone::Waiting => theme.warning(),
                        Tone::Queued => theme.dim(),
                    };
                    cards.extend(card(
                        theme,
                        CardProps {
                            fill,
                            accent,
                            head: vec![Span::styled(
                                theme.clean(title),
                                accent.patch(theme.title()),
                            )],
                            right: theme.clean(meta),
                            body: text,
                            max_body: usize::MAX,
                            width,
                        },
                    ));
                }
                TranscriptCard::Comment {
                    id,
                    target,
                    text,
                    status,
                } => {
                    let mut fill = theme.card();
                    if is_found {
                        top = Some(cards.len());
                        fill = fill.add_modifier(Modifier::REVERSED);
                    }
                    let state = self
                        .view
                        .comments
                        .get(id)
                        .map_or(status.as_str(), |_| self.view.comment_state(*id));
                    let body = match self.view.comment_error(*id) {
                        Some(error) => format!("{text}\nError: {error}"),
                        None => text.clone(),
                    };
                    cards.extend(card(
                        theme,
                        CardProps {
                            fill,
                            accent: theme.person("user"),
                            head: vec![Span::styled(
                                format!("Comment #{} on {}", id, theme.clean(target)),
                                theme.accent().patch(theme.title()),
                            )],
                            right: theme.clean(state),
                            body: &body,
                            max_body: usize::MAX,
                            width,
                        },
                    ));
                }
                TranscriptCard::Confirmation(conf) => {
                    let mut fill = theme.card();
                    if is_found {
                        top = Some(cards.len());
                        fill = fill.add_modifier(Modifier::REVERSED);
                    }
                    let (accent, status_str) = match conf.status {
                        ConfirmationStatus::Pending => (theme.warning(), "[awaiting confirmation]"),
                        ConfirmationStatus::Confirmed => (theme.status("completed"), "[confirmed]"),
                        ConfirmationStatus::Stale => (theme.error(), "[stale: stale_scope]"),
                        ConfirmationStatus::Rejected => (theme.dim(), "[rejected]"),
                    };
                    let title_span = vec![
                        Span::styled("Confirmation: ", theme.faint()),
                        Span::styled(theme.clean(&conf.title), accent.patch(theme.title())),
                    ];
                    let mut body = conf.detail.clone();
                    if let Some(plan) = &conf.plan_id {
                        body.push_str(&format!("\nPlan: {plan}"));
                    }
                    if let Some(task) = &conf.task_id {
                        body.push_str(&format!("\nTask: {task}"));
                    }
                    if let Some(candidate) = &conf.candidate {
                        body.push_str(&format!("\nDiff candidate: {candidate}"));
                    }
                    if !conf.scope_hash.is_empty() {
                        body.push_str(&format!("\nScope (hash): {}", conf.scope_hash));
                    }
                    cards.extend(card(
                        theme,
                        CardProps {
                            fill,
                            accent,
                            head: title_span,
                            right: status_str.to_string(),
                            body: &body,
                            max_body: usize::MAX,
                            width,
                        },
                    ));
                }
            }
        }

        // The orchestrator's state, at the end of the conversation: sending,
        // then thinking, with what it does and for how long.
        let waiting_for_turn = !self.view.idle
            && matches!(
                self.view.stage(),
                view::Stage::Interview | view::Stage::Planning
            );
        if self.sending || self.view.orchestrator_activity().is_some() || waiting_for_turn {
            let (what, doing) = if self.sending {
                ("sending…", String::new())
            } else {
                (
                    "thinking…",
                    self.view
                        .orchestrator_activity()
                        .unwrap_or_else(|| "waiting for the orchestrator to start".into()),
                )
            };
            let elapsed = self
                .view
                .agents
                .get(view::ORCHESTRATOR)
                .and_then(|agent| agent.started_ms.filter(|_| agent.running))
                .map(|started| {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |d| d.as_millis() as i64);
                    view::duration(now.saturating_sub(started).max(0) as u64)
                })
                .unwrap_or_default();
            cards.extend(card(
                theme,
                CardProps {
                    fill: theme.card(),
                    accent: theme.person("orchestrator"),
                    head: vec![
                        Span::styled(
                            format!("{} ", theme.spinner(self.tick)),
                            theme.status("working"),
                        ),
                        Span::styled(
                            format!("orchestrator {what}"),
                            theme.person("orchestrator").patch(theme.title()),
                        ),
                    ],
                    right: elapsed,
                    body: &doing,
                    max_body: 2,
                    width,
                },
            ));
        }

        if cards.is_empty() && !self.view.transcript.is_empty() {
            for (index, entry) in self.view.transcript.iter().enumerate() {
                let style = if self.found == Some(index) {
                    top = Some(cards.len());
                    entry.style.add_modifier(Modifier::REVERSED)
                } else {
                    entry.style
                };
                for line in wrap(&entry.text, width as usize) {
                    cards.push(Row::new(
                        theme.card(),
                        vec![Span::styled(theme.clean(&line), style)],
                    ));
                }
            }
        }

        if cards.is_empty() {
            cards.extend(card(
                theme,
                CardProps {
                    fill: theme.card(),
                    accent: theme.accent(),
                    head: vec![Span::styled(
                        "No messages in this conversation.",
                        theme.title(),
                    )],
                    right: String::new(),
                    body: "Type a message in the composer below or press Ctrl+P to open the palette.",
                    max_body: usize::MAX,
                    width,
                },
            ));
        }

        let height = area.height as usize;
        if let Some(top_idx) = top.take().filter(|_| self.scroll == usize::MAX) {
            self.scroll = cards.len().saturating_sub(top_idx + height);
        }
        self.scroll = self.scroll.min(cards.len().saturating_sub(height));
        let end = cards.len().saturating_sub(self.scroll);
        let start = end.saturating_sub(height);

        let buf = frame.buffer_mut();
        let body_area = Rect::new(area.x, area.y, area.width.saturating_sub(1), area.height);
        draw_rows(buf, body_area, &cards[start..end]);

        let scroll_area = Rect::new(area.right().saturating_sub(1), area.y, 1, area.height);
        scrollbar(buf, theme, scroll_area, cards.len(), start, height);
    }

    fn draw_graph(&mut self, frame: &mut Frame, area: Rect) {
        self.selected = graph::task_node_key(&self.selected);
        let focused = self.focus == Focus::Graph;
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(if focused {
                Style::default().fg(Color::Cyan)
            } else {
                dim()
            })
            .title(format!(
                " team · filter: {} (f) · groups: {} (g) · [V] ver [D] dep [M] msg ",
                self.filter.name(),
                self.graph_grouping.name()
            ));
        let frame_inner = block.inner(area);
        frame.render_widget(block, area);
        // Padding between the border and the graph; skipped when the panel is tiny.
        let inner = padded(frame_inner);

        self.graph_viewport = Some((inner.width, inner.height));
        self.graph_area = Some(inner);
        let graph = Graph::new_with_viewport(
            &self.view,
            self.graph_filter.as_deref(),
            &self.graph_collapsed,
            self.graph_grouping,
            self.graph_viewport,
        );
        if graph.node(&self.selected).is_none()
            || !graph.node(&self.selected).is_some_and(|n| n.visible)
        {
            if let Some((_, id)) = self.selected.split_once(':') {
                let worker_key = format!("worker:{id}");
                if graph.node(&worker_key).is_some_and(|n| n.visible) {
                    self.selected = worker_key;
                } else if let Some(vis) = graph
                    .nodes
                    .iter()
                    .find(|n| n.visible && n.key == ORCHESTRATOR)
                {
                    self.selected = vis.key.clone();
                } else if let Some(first_vis) = graph.nodes.iter().find(|n| n.visible) {
                    self.selected = first_vis.key.clone();
                } else {
                    self.selected = ORCHESTRATOR.into();
                }
            } else if let Some(vis) = graph
                .nodes
                .iter()
                .find(|n| n.visible && n.key == ORCHESTRATOR)
            {
                self.selected = vis.key.clone();
            } else if let Some(first_vis) = graph.nodes.iter().find(|n| n.visible) {
                self.selected = first_vis.key.clone();
            } else {
                self.selected = ORCHESTRATOR.into();
            }
        }
        // Reset scroll offset if the entire graph fits into the panel
        if graph.width <= inner.width {
            self.offset.0 = 0;
        }
        if graph.height <= inner.height {
            self.offset.1 = 0;
        }
        // Scroll only as far as needed to keep the selected node in view.
        if let Some(node) = graph.node(&self.selected) {
            let (left, top) = &mut self.offset;
            if node.x < *left {
                *left = node.x.saturating_sub(3);
            } else if node.x + graph::WIDTH + 3 > *left + inner.width {
                *left = (node.x + graph::WIDTH + 3).saturating_sub(inner.width);
            }
            if node.height > inner.height {
                // A tall card can be read with PgUp/PgDn or the mouse wheel.
                let last = node
                    .y
                    .saturating_add(node.height)
                    .saturating_sub(inner.height);
                if *top < node.y || *top > last {
                    *top = node.y;
                }
            } else if node.y < *top {
                *top = node.y;
            } else if node.y.saturating_add(node.height) > top.saturating_add(inner.height) {
                *top = node
                    .y
                    .saturating_add(node.height)
                    .saturating_sub(inner.height);
            }
        }
        let mut canvas =
            Canvas::new_window(self.offset.0, self.offset.1, inner.width, inner.height);
        graph.render_with(
            &self.view,
            &mut canvas,
            self.filter,
            &self.selected,
            &Theme::from_env(),
            self.tick,
        );
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
        let visible_nodes: Vec<&graph::Node> = graph.nodes.iter().filter(|n| n.visible).collect();
        let Some(overlay) = graph::compute_progress_overlay_with_nodes(
            inner,
            graph.node(&self.selected),
            &visible_nodes,
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
                    .title(" now ")
                    .padding(ratatui::widgets::Padding::horizontal(1)),
            ),
            overlay,
        );
    }

    pub fn mouse(&mut self, mouse: event::MouseEvent) -> bool {
        if self.config_editor.as_ref().is_some_and(|e| e.visible) {
            return true;
        }
        self.settle_focus();
        if self.trust_prompt.is_some() || self.project_trust.is_some() {
            return true;
        }
        if let Some(modal) = &mut self.unblock {
            if mouse.kind == MouseEventKind::Down(MouseButton::Left) {
                match modal.click(mouse.column, mouse.row) {
                    unblock::Outcome::Choose(choice) => {
                        if let Some(state) = self.unblock.take() {
                            self.unblock_choice(state, choice);
                        }
                    }
                    unblock::Outcome::Close | unblock::Outcome::None => {}
                }
            }
            return true;
        }
        match mouse.kind {
            MouseEventKind::ScrollUp => {
                if self.help.is_some() {
                    self.help_scroll = self.help_scroll.saturating_sub(3);
                } else if let Some(pv) = &mut self.plan_view {
                    pv.key(
                        event::KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
                        &self.view,
                    );
                } else if let Some(diff) = &mut self.diff {
                    diff.key(KeyCode::Up);
                } else if let Some(details) = &mut self.details {
                    details.scroll = details.scroll.saturating_sub(3);
                } else if self.focus == Focus::Graph {
                    let graph = Graph::new_with_viewport(
                        &self.view,
                        self.graph_filter.as_deref(),
                        &self.graph_collapsed,
                        self.graph_grouping,
                        self.graph_viewport,
                    );
                    self.scroll_graph_node(&graph, false, 3);
                } else {
                    self.scroll += 3;
                }
            }
            MouseEventKind::ScrollDown => {
                if self.help.is_some() {
                    self.help_scroll = (self.help_scroll + 3).min(self.help_max_scroll());
                } else if let Some(pv) = &mut self.plan_view {
                    pv.key(
                        event::KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
                        &self.view,
                    );
                } else if let Some(diff) = &mut self.diff {
                    diff.key(KeyCode::Down);
                } else if let Some(details) = &mut self.details {
                    details.scroll += 3;
                } else if self.focus == Focus::Graph {
                    let graph = Graph::new_with_viewport(
                        &self.view,
                        self.graph_filter.as_deref(),
                        &self.graph_collapsed,
                        self.graph_grouping,
                        self.graph_viewport,
                    );
                    self.scroll_graph_node(&graph, true, 3);
                } else {
                    self.scroll = self.scroll.saturating_sub(3);
                }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                let col = mouse.column;
                let row = mouse.row;
                let area = self.terminal_area;
                if self.plan_view.is_some() {
                    let action = self
                        .plan_view
                        .as_mut()
                        .map(|pv| pv.mouse_click(col, row, &self.view))
                        .unwrap_or(PlanAction::None);
                    self.handle_plan_action(action);
                    return true;
                }
                // Behind a dialog the header is not there to click.
                if self.diff.is_some()
                    || self.details.is_some()
                    || self.picker.is_some()
                    || self.editor.is_some()
                    || self.help.is_some()
                {
                    return true;
                }
                if let Some(inner) = self.graph_area
                    && inner.contains((col, row).into())
                {
                    let graph = Graph::new_with_viewport(
                        &self.view,
                        self.graph_filter.as_deref(),
                        &self.graph_collapsed,
                        self.graph_grouping,
                        self.graph_viewport,
                    );
                    let x = col.saturating_sub(inner.x).saturating_add(self.offset.0);
                    let y = row.saturating_sub(inner.y).saturating_add(self.offset.1);
                    if let Some(node) = graph.nodes.iter().find(|node| {
                        node.visible
                            && x >= node.x
                            && x < node.x.saturating_add(graph::WIDTH)
                            && y >= node.y
                            && y < node.y.saturating_add(node.height)
                    }) {
                        self.select_node(node.key.clone());
                    }
                    self.focus = Focus::Graph;
                    return true;
                }
                if row == area.y && area.width > 0 {
                    let header_line = self.header_line(area.width);
                    let controls = self.view.controls();
                    let header_text: String = header_line
                        .spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect();
                    // Find bracketed token at column
                    let mut cur_col = 0usize;
                    let target_col = usize::from(col.saturating_sub(area.x));
                    for (idx, ch) in header_text.char_indices() {
                        let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(1);
                        if target_col >= cur_col && target_col < cur_col + w {
                            if let Some(open) = header_text[..=idx].rfind('[')
                                && let Some(close_rel) = header_text[idx..].find(']')
                            {
                                let token = &header_text[open..=idx + close_rel];
                                match token {
                                    "[Mode: autonomous]" | "[auto]" => {
                                        self.command("/mode supervised");
                                    }
                                    "[Mode: supervised]" | "[sup]" => {
                                        self.command("/mode autonomous");
                                    }
                                    "[/pause]" | "[‖]" if controls.pause => {
                                        self.command("/pause")
                                    }
                                    "[/continue]" | "[▶]" if controls.resume_paused => {
                                        self.command("/continue")
                                    }
                                    "[/resume]" | "[↻]" if controls.resume_plan => {
                                        self.command("/resume")
                                    }
                                    "[/cancel]" | "[✕]" if controls.cancel => {
                                        self.command("/cancel")
                                    }
                                    "[Plan]" => self.command("/plan"),
                                    "[Unblock]" => self.command("/unblock"),
                                    "[Chat]" => self.focus = Focus::Chat,
                                    "[Team]" => self.focus = Focus::Graph,
                                    _ => {}
                                }
                            }
                            break;
                        }
                        cur_col += w;
                    }
                } else if row > area.y
                    && row + 1 < area.bottom()
                    && !self.view.graph_hidden()
                    && area.width >= SIDE_BY_SIDE_BREAKPOINT
                {
                    let chat_width = (area.width / 3).max(34);
                    if col < area.x + chat_width {
                        self.focus = Focus::Chat;
                    } else if col > area.x + chat_width && self.focus == Focus::Chat {
                        self.focus = Focus::Graph;
                    }
                }
            }
            _ => {}
        }
        true
    }
}

/// Whether to draw in ASCII: asked for with `ANTISLOP_ASCII`, or the
/// locale is not UTF-8.
/// Shrinks `area` by one cell on every side so content sits centred with a margin.
/// Tiny areas are returned unchanged.
fn padded(area: Rect) -> Rect {
    if area.width > 4 && area.height > 4 {
        Rect::new(area.x + 1, area.y + 1, area.width - 2, area.height - 2)
    } else {
        area
    }
}

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
        '•' | '⏺' | '✻' | '✦' => '*',
        '⎿' => '+',
        '›' => '>',
        '●' => '*',
        '▌' => '|',
        '█' => '#',
        '✕' => 'x',
        '‖' => '=',
        '◆' => 'O',
        '◇' => 'P',
        '≡' => 'A',
        '▸' | '◂' => '>',
        '◈' => 'V',
        '⠋' | '⠙' | '⠹' | '⠸' | '⠼' | '⠴' | '⠦' | '⠧' | '⠇' | '⠏' => '*',
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

/// `~` and `~/…` mean the user's home; other paths are used as typed.
fn expand_home(path: &str) -> std::path::PathBuf {
    let path = path.trim();
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => std::path::PathBuf::from(home).join(rest),
        (None, Some(home)) if path == "~" => std::path::PathBuf::from(home),
        _ => std::path::PathBuf::from(path),
    }
}

/// Asks the daemon to unblock a task, with a freshly read token so the
/// request is about the state the user is looking at. Returns what the
/// daemon says happened.
fn recover(
    client: &Client,
    conversation: &str,
    plan: &str,
    task: &str,
    action: crate::lean::RecoveryAction,
    message: Option<String>,
) -> std::result::Result<String, String> {
    let stored = client
        .call(&Request::Task {
            plan: plan.to_owned(),
            id: task.to_owned(),
        })
        .map_err(|e| e.to_string())?;
    let token = stored["freshness_token"]
        .as_str()
        .ok_or("the daemon gave no freshness token")?
        .to_owned();
    let reply = client
        .call(&Request::Recover(crate::lean::TaskRecoveryRequest {
            conversation: conversation.to_owned(),
            plan: plan.to_owned(),
            task: task.to_owned(),
            action,
            consent_files: None,
            freshness_token: token,
            message,
        }))
        .map_err(|e| e.to_string())?;
    Ok(reply["notice"].as_str().unwrap_or("done").to_owned())
}

pub fn run(options: &Options, client: Client) -> Result<()> {
    let conversation = options.conversation(&client)?;
    let (updates, received) = mpsc::channel();
    let mut app = App {
        config_editor: None,
        config_drafts: Default::default(),
        client: client.clone(),
        project: options
            .project
            .file_name()
            .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
        conversation: String::new(),
        generation: 0,
        stop_poller: Arc::new(AtomicBool::new(false)),
        tick: 0,
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
        plan_view: None,
        editor: None,
        picker: None,
        help: None,
        help_scroll: 0,
        terminal_area: Rect::default(),
        history: vec![],
        history_path: None,
        history_index: None,
        history_draft: String::new(),
        history_draft_cursor: 0,
        history_draft_preferred_col: None,
        history_item_cursors: std::collections::HashMap::new(),
        sending: false,
        sending_comment: false,
        pending_send: None,
        pending_comment: None,
        send_request_id: 0,
        comment_request_id: 0,
        diff_request_id: 0,
        task_request_id: 0,
        failed_send: None,
        failed_comment: None,
        conversation_drafts: std::collections::HashMap::new(),
        comment_drafts: std::collections::HashMap::new(),
        comment_draft_cursors: std::collections::HashMap::new(),
        conversation_states: std::collections::HashMap::new(),
        graph_filter: None,
        graph_collapsed: std::collections::HashSet::new(),
        graph_grouping: graph::GroupingMode::default(),
        confirmation: None,
        details_scroll_memory: std::collections::HashMap::new(),
        details_query_memory: std::collections::HashMap::new(),
        graph_viewport: None,
        graph_area: None,
        unblock: None,
        projects: Vec::new(),
        active_project: 0,
        project_trust: None,
        trust_prompt: if options.trust {
            None
        } else {
            match client.call(&Request::TrustStatus) {
                Ok(val) if val["trusted"] == true => None,
                _ => Some(options.project.to_string_lossy().into_owned()),
            }
        },
    };
    app.load_history(&options.state);
    app.register_project(workspace::ProjectConn::new(
        options.project.clone(),
        client.clone(),
    ));
    app.open(conversation);

    // https://no-color.org: nothing depends on color alone (design §17).
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
    let ascii = ascii_terminal();
    let mut terminal = ratatui::init();
    let _ = crossterm::execute!(std::io::stdout(), EnableBracketedPaste, EnableMouseCapture);
    let result = (|| -> Result<()> {
        loop {
            while let Ok(update) = received.try_recv() {
                app.apply_update(update);
            }
            app.tick = app.tick.wrapping_add(1);
            if let Some(plan) = &mut app.plan_view {
                plan.tick = app.tick;
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
                Event::Mouse(mouse) => {
                    if !app.mouse(mouse) {
                        return Ok(());
                    }
                }
                Event::Paste(text) => app.paste(&text),
                _ => {}
            }
        }
    })();
    app.stop_poller.store(true, Ordering::SeqCst);
    let _ = crossterm::execute!(
        std::io::stdout(),
        DisableBracketedPaste,
        DisableMouseCapture
    );
    ratatui::restore();
    result
}

#[cfg(test)]
pub mod tests {
    use super::view::render_header;
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
    fn ac_modal_04_esc_stays_open_and_explicit_quit_detaches() {
        let mut app = test_app();
        assert!(app.input.is_empty());
        assert!(app.key(key_press(KeyCode::Esc)));
        assert!(!app.key(key_press(KeyCode::Char('q'))));
        assert!(!app.key(event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )));
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
        let backend = TestBackend::new(11, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
        let norm = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(norm.contains("Terminal too small (min. 12x5 required)"));
    }

    #[test]
    fn ac_resp_01_app_draw_main_compact_switch_at_84_cols() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = test_app();
        app.focus = Focus::Chat;
        let backend = TestBackend::new(84, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(text.contains("[Chat]"));
        assert!(text.contains("Team"));
        assert!(text.contains("(Tab: switch)"));

        // Switch to Focus::Graph
        app.focus = Focus::Graph;
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = buffer.content.iter().map(|c| c.symbol()).collect();
        assert!(text.contains("Chat"));
        assert!(text.contains("[Team]"));
        assert!(text.contains("(Tab: switch)"));
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
        assert!(!text.contains("[Chat]"));
        assert!(!text.contains("(Tab: switch)"));
        assert!(text.contains("antislop"));
        assert!(text.contains("ready"));
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
        assert!(text.contains("No messages in this conversation"));
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
        assert!(text1.contains("send message to orchestrator"));
        assert!(text1.contains("newline in draft"));
        assert!(text1.contains("line in draft"));
        assert!(text1.contains("scroll conversation"));
        assert!(text1.contains("accept/resume"));
        assert!(text1.contains("delete to start / end of line"));

        // 2. Navigate/scroll help down and verify modal isolation
        let bottom = app.help_max_scroll();
        for _ in 0..bottom {
            app.key(event::KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        assert_eq!(
            app.help_scroll, bottom,
            "Help should scroll down to max_scroll"
        );
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

        assert!(text2.contains("cancel running work"));
        assert!(text2.contains("pause new steps / continue"));
        assert!(text2.contains("/mode autonomous"));
        assert!(text2.contains("/mode supervised"));
        assert!(text2.contains("search conversation"));
        assert!(text2.contains("command palette"));
        assert!(text2.contains("back/clear draft"));

        // 4. Verify End followed by Down, PageDown, Up (clamping and no overflow panic)
        let max_scroll = app.help_max_scroll();
        assert!(max_scroll >= 8);

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
        assert_eq!(app.help_max_scroll(), max_scroll);

        // 6. Graph help at 60x12: verify long filter relation description is fully accessible without clipping
        app.show_help(Help::Graph);
        for _ in 0..2 {
            app.key(event::KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        terminal
            .draw(|frame| {
                app.draw(frame);
            })
            .unwrap();
        let buffer_graph = terminal.backend().buffer();
        let text_graph: String = buffer_graph.content.iter().map(|c| c.symbol()).collect();
        assert!(text_graph.contains("relation filter:"));
        assert!(text_graph.contains("all"));
        assert!(text_graph.contains("dependencies"));
        assert!(text_graph.contains("verification"));
        assert!(text_graph.contains("messages"));
        assert!(text_graph.contains("supervision"));

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
        app.view.activity = Some("compiling tests".into());
        app.view.tasks = vec![view::TaskRow {
            id: "t1".into(),
            status: "working".into(),
            ..view::TaskRow::default()
        }];
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
        // Progress overlay renders with "now" title and doesn't obscure orchestrator
        assert!(text.contains("now"));
        assert!(text.contains("orchestrator") || text.contains(ORCHESTRATOR));

        // Verify orchestrator node cells remain intact in the buffer
        let mut orchestrator_visible = false;
        for y in 0..30 {
            let row: String = (0..100).map(|x| buffer[(x, y)].symbol()).collect();
            if row.contains("orchestrator") || row.contains(ORCHESTRATOR) {
                orchestrator_visible = true;
                break;
            }
        }
        assert!(orchestrator_visible, "Selected node must remain visible");

        // 2. Compact render with tiny inner area where all overlay placements collide
        // When compute_progress_overlay returns None, App::draw hides the overlay
        let mut app_compact = test_app();
        app_compact.view.activity = Some("very long activity line".into());
        app_compact.view.tasks = app.view.tasks.clone();
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
        assert!(tiny_text.contains("orchestrator") || tiny_text.contains(ORCHESTRATOR));
    }

    #[test]
    fn the_plan_opens_from_every_panel_and_rejecting_asks_for_a_reason() {
        let mut app = test_app();
        app.view.plan = Some("p1".into());
        app.view.tasks = vec![view::TaskRow {
            id: "t1".into(),
            status: "proposed".into(),
            ..view::TaskRow::default()
        }];
        app.key(key_ctrl('l'));
        assert!(
            app.plan_view.is_some(),
            "Ctrl+L opens the plan from the chat"
        );
        app.plan_view = None;
        // Even with the cursor in the draft, the shortcut does not type.
        app.input.insert_char('x');
        app.key(key_ctrl('l'));
        assert!(app.plan_view.is_some());
        assert_eq!(app.input.text, "x");

        app.handle_plan_action(PlanAction::Reject);
        let editor = app.editor.as_ref().expect("a reason editor opens");
        assert_eq!(editor.target, "reject:p1");
        // An empty reason may be submitted; the plan view closes.
        app.submit_comment();
        assert!(app.plan_view.is_none());
        assert!(app.editor.is_none());

        // Nothing to reject once the plan is decided.
        let mut decided = test_app();
        decided.view.plan = Some("p1".into());
        decided.handle_plan_action(PlanAction::Reject);
        assert!(decided.editor.is_none());
    }

    #[test]
    fn the_narrow_header_keeps_a_clickable_plan_button() {
        for width in [50, 70, 94] {
            let line = render_header(
                "p",
                "conversation-id",
                Some(false),
                "ready",
                PanelFocus::Chat,
                width,
            );
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(text.contains("[Plan]"), "{width}: {text}");
        }
        let wide = render_header(
            "p",
            "conversation-id",
            Some(false),
            "ready",
            PanelFocus::Chat,
            120,
        );
        let text: String = wide.spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text.matches("[Plan]").count(), 1);
    }

    #[test]
    fn enter_folds_and_unfolds_prompts_on_overview_only() {
        let mut app = test_app();
        app.view.agents.insert(
            "worker:t1".into(),
            view::Agent {
                provider: "codex".into(),
                feed: vec![session::Block::Prompt(
                    (0..20).map(|n| format!("p{n}\n")).collect::<String>(),
                )],
                ..view::Agent::default()
            },
        );
        app.details = Some(details::Details::new("worker:t1".into()));
        app.details.as_mut().unwrap().switch_tab(1);
        app.key(key_press(KeyCode::Enter));
        assert!(!app.details.as_ref().unwrap().expand_prompts, "Messages");
        app.details.as_mut().unwrap().switch_tab(0);
        app.key(key_press(KeyCode::Enter));
        let details = app.details.as_mut().unwrap();
        assert!(details.expand_prompts);
        let shown: String = details
            .lines(&app.view)
            .into_iter()
            .map(|(_, l)| l)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(shown.contains("p19"));
        app.key(key_press(KeyCode::Enter));
        assert!(!app.details.as_ref().unwrap().expand_prompts);
    }

    #[test]
    fn the_session_follows_new_entries_until_the_user_scrolls_up() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut app = test_app();
        let feed = |count: usize| {
            (0..count)
                .map(|n| session::Block::Text(format!("entry {n}")))
                .collect::<Vec<_>>()
        };
        let agent = |count| view::Agent {
            provider: "codex".into(),
            feed: feed(count),
            ..view::Agent::default()
        };
        app.view.agents.insert("worker:t1".into(), agent(40));
        let mut details = details::Details::new("worker:t1".into());
        details.switch_tab(0);
        app.details = Some(details);
        let mut term = Terminal::new(TestBackend::new(60, 12)).unwrap();
        let screen = |term: &Terminal<TestBackend>| -> String {
            term.backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect()
        };

        term.draw(|f| app.draw(f)).unwrap();
        assert_eq!(
            app.details.as_ref().unwrap().scroll,
            0,
            "Overview opens at the top"
        );
        assert!(!screen(&term).contains("entry 39"));
        app.details.as_mut().unwrap().switch_tab(1);
        app.details.as_mut().unwrap().switch_tab(0);
        term.draw(|f| app.draw(f)).unwrap();
        assert_eq!(app.details.as_ref().unwrap().scroll, 0);
        app.key(key_press(KeyCode::End));
        term.draw(|f| app.draw(f)).unwrap();
        assert!(screen(&term).contains("entry 39"));

        // New entries scroll into view by themselves.
        app.view.agents.insert("worker:t1".into(), agent(55));
        term.draw(|f| app.draw(f)).unwrap();
        assert!(screen(&term).contains("entry 54"));

        // Scrolling up stops following; the screen stays where the user is.
        app.key(key_press(KeyCode::PageUp));
        term.draw(|f| app.draw(f)).unwrap();
        assert!(!app.details.as_ref().unwrap().follow);
        app.view.agents.insert("worker:t1".into(), agent(70));
        term.draw(|f| app.draw(f)).unwrap();
        assert!(!screen(&term).contains("entry 69"));

        // End goes back to the newest entry and follows again.
        app.key(key_press(KeyCode::End));
        term.draw(|f| app.draw(f)).unwrap();
        assert!(app.details.as_ref().unwrap().follow);
        assert!(screen(&term).contains("entry 69"));
        app.view.agents.insert("worker:t1".into(), agent(80));
        term.draw(|f| app.draw(f)).unwrap();
        assert!(screen(&term).contains("entry 79"));
    }

    #[test]
    fn the_chat_shows_that_the_orchestrator_is_sending_then_thinking() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let mut app = test_app();
        app.view
            .cards
            .push(view::TranscriptCard::Message(view::MessageEntry {
                id: "m1".into(),
                sender: "You".into(),
                target: "orchestrator".into(),
                text: "build it".into(),
                state: "sent".into(),
                kind: "user".into(),
            }));
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let screen = |app: &mut App, term: &mut Terminal<TestBackend>| -> String {
            term.draw(|f| app.draw(f)).unwrap();
            term.backend()
                .buffer()
                .content
                .iter()
                .map(|c| c.symbol())
                .collect()
        };
        assert!(!screen(&mut app, &mut term).contains("orchestrator sending"));

        app.sending = true;
        assert!(screen(&mut app, &mut term).contains("orchestrator sending…"));

        app.sending = false;
        app.view.activity = Some("orchestrator: exec ls".into());
        let thinking = screen(&mut app, &mut term);
        assert!(thinking.contains("orchestrator thinking…"));
        assert!(thinking.contains("exec ls"));
        // The spinner moves with the tick.
        app.tick = 1;
        assert_ne!(thinking, screen(&mut app, &mut term));

        // Between the daemon taking the message and the turn starting.
        app.view.activity = None;
        app.view.idle = false;
        assert!(screen(&mut app, &mut term).contains("waiting for the orchestrator to start"));
        app.view.idle = true;
        assert!(!screen(&mut app, &mut term).contains("orchestrator thinking"));
    }

    #[test]
    fn a_stopped_step_can_be_unblocked_from_a_key_a_command_and_the_header() {
        let mut app = test_app();
        app.view.plan = Some("p1".into());
        let task = |id: &str, status: &str| view::TaskRow {
            id: id.into(),
            title: format!("Task {id}"),
            status: status.into(),
            writes: true,
            reason: Some("the verifier repeated itself".into()),
            ..view::TaskRow::default()
        };
        app.view.tasks = vec![task("t1", "approved")];
        app.command("/unblock");
        assert!(app.unblock.is_none());
        assert!(
            app.notice
                .as_deref()
                .unwrap()
                .contains("nothing to unblock")
        );

        app.view.tasks = vec![
            task("t1", "approved"),
            task("t2", "needs_user"),
            task("t3", "skipped"),
        ];
        // The first stopped step that is not just skipped comes first.
        app.command("/unblock");
        assert_eq!(app.unblock.as_ref().unwrap().task, "t2");
        app.key(key_press(KeyCode::Esc));
        assert!(app.unblock.is_none());
        app.command("/unblock t3");
        assert_eq!(app.unblock.as_ref().unwrap().task, "t3");
        app.key(key_press(KeyCode::Esc));

        // The graph key acts on the selected step.
        app.view.tasks.push(task("t4", "failed"));
        app.focus = Focus::Graph;
        app.selected = "verifier:t4".into();
        // Stage Blocked keeps the graph visible: nothing is running.
        app.key(key_press(KeyCode::Char('u')));
        assert_eq!(app.unblock.as_ref().unwrap().task, "t4");

        // A choice that takes a hint opens the editor for it.
        app.key(key_press(KeyCode::Char('1')));
        assert!(app.unblock.is_none());
        assert_eq!(
            app.editor.as_ref().unwrap().target,
            "unblock:retry_with_guidance:p1:t4"
        );
        app.editor = None;

        // The header offers it whenever a step is stuck.
        assert!(app.header_status().ends_with("[Unblock]"));
        app.view.tasks.retain(|t| t.status == "approved");
        assert!(!app.header_status().contains("[Unblock]"));
    }

    fn header_text(app: &App, width: u16) -> String {
        app.header_line(width)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect()
    }

    fn click_header(app: &mut App, width: u16, token: &str) {
        let text = header_text(app, width);
        let at = text
            .find(token)
            .unwrap_or_else(|| panic!("{token} in {text}"));
        let col = unicode_width::UnicodeWidthStr::width(&text[..at]) as u16 + 1;
        app.terminal_area = Rect::new(0, 0, width, 24);
        app.mouse(event::MouseEvent {
            kind: MouseEventKind::Down(event::MouseButton::Left),
            column: col,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });
    }

    #[test]
    fn the_header_buttons_exist_on_narrow_terminals_and_know_what_they_can_do() {
        let mut app = test_app();
        app.view.idle = true;
        for width in [45, 60, 94, 95, 140] {
            let text = header_text(&app, width);
            let (pause, resume, cancel) = if width >= 95 {
                ("[/pause]", "[/resume]", "[/cancel]")
            } else {
                ("[‖]", "[↻]", "[✕]")
            };
            for token in [pause, resume, cancel] {
                assert!(text.contains(token), "{width}: {text}");
            }
        }
        // Nothing runs: pause and cancel are dim, and a click does nothing.
        let controls = app.view.controls();
        assert!(!controls.pause && !controls.cancel && !controls.resume_plan);
        // While work runs they light up; paused work offers to continue.
        app.view.idle = false;
        assert!(app.view.controls().pause && app.view.controls().cancel);
        app.view.paused = true;
        assert!(app.view.controls().resume_paused && !app.view.controls().pause);
        assert!(header_text(&app, 60).contains("[▶]"));
        assert!(!header_text(&app, 60).contains("[‖]"));
        // Compact mode token at the narrowest widths.
        app.view.autonomous = Some(true);
        assert!(header_text(&app, 40).contains("[auto]"));
    }

    #[test]
    fn panels_and_buttons_in_the_header_can_be_clicked() {
        let mut app = test_app();
        app.view.idle = false;
        app.view.cards.push(view::TranscriptCard::System {
            title: "x".into(),
            meta: String::new(),
            text: "x".into(),
            tone: Tone::Queued,
        });
        app.view.tasks = vec![view::TaskRow {
            id: "t1".into(),
            status: "working".into(),
            ..view::TaskRow::default()
        }];
        // Every panel is a button, not only the focused one.
        click_header(&mut app, 80, "[Team]");
        assert_eq!(app.focus, Focus::Graph);
        click_header(&mut app, 80, "[Chat]");
        assert_eq!(app.focus, Focus::Chat);
        // The mode does not flip until the daemon says so.
        app.view.autonomous = Some(false);
        app.command("/mode");
        assert_eq!(app.view.autonomous, Some(false));
        app.key(key_ctrl('t'));
        assert_eq!(app.view.autonomous, Some(false));

        // With a dialog open the header ignores clicks.
        app.focus = Focus::Chat;
        app.editor = Some(Editor {
            target: "plan:p1".into(),
            input: InputBuffer::new(),
        });
        click_header(&mut app, 80, "[Team]");
        assert_eq!(app.focus, Focus::Chat);
    }
}
