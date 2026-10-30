//! Terminal frontend for one conversation: the chat with the orchestrator
//! beside the team graph. It never waits on the daemon in its draw loop: a
//! poller thread long-polls events and every action runs on its own thread,
//! so typing, scrolling and sending stay instant.

mod buffer;
pub mod clipboard;
pub mod config;
mod contracts;
mod dashboard;
pub mod details;
pub mod diffview;
pub mod document;
pub mod editor;
mod events;
pub mod graph;
mod history;
mod input;
mod inspection;
pub mod interview;
mod layout;
pub mod ledger;
mod native_view;
mod navigation;
pub mod palette;
mod paste;
mod planhistory;
pub mod planview;
mod render;
pub mod sanitize;
pub mod selection;
mod shell;
mod state;
pub mod taskinfo;
pub mod theme;
pub mod unblock;
pub mod view;
pub mod workspace;

pub use contracts::{ConversationState, Editor, Focus, Update};
pub use details::Details;
pub use diffview::DiffView;
pub use editor::{InputBuffer, sanitize_paste, sanitize_single_line_paste, visual_text_lines};
pub use graph::{Filter, GroupingMode};
pub use palette::{Help, Kind, Picker};
pub use planview::{PlanAction, PlanView};
pub use sanitize::{sanitize, sanitize_line};
pub use shell::run;
pub use state::App;
#[cfg(test)]
use state::TranscriptLayout;
use state::TranscriptLayoutEntry;
pub use theme::{
    CardProps, Glyphs, Row, Theme, Tone, card, draw_rows, scrollbar, section, split_row, tone,
};
pub use view::{
    ConfirmationAction, ConfirmationCard, ConfirmationStatus, MessageEntry, TranscriptCard,
};

use super::daemon::{Client, Request};
use super::{projects, trust};
use crate::Result;
use crossterm::event::{self, KeyCode, KeyModifiers, MouseButton, MouseEventKind};
use details::{CHECKS, COMMENTS, RESULT};
use graph::{Canvas, Graph};
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
use view::{
    ORCHESTRATOR, PanelFocus, SIDE_BY_SIDE_BREAKPOINT, View, dim, draw_min_term_guard, target_name,
    truncate, wrap,
};

impl App {
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
                    ledger_view: self.ledger_view.clone(),
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
        self.plan_history = None;
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
            self.ledger_view = state.ledger_view;
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
            self.ledger_view = None;
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
            .map(|details| {
                if details.fetch_in_flight {
                    return false;
                }
                if details
                    .fetch_retry_after
                    .is_some_and(|deadline| std::time::Instant::now() < deadline)
                {
                    return false;
                }
                let fetched = details.fetched_round;
                let now = details
                    .selected_round
                    .or_else(|| self.task_row_round(&details.key))
                    .or(details.fetched_round)
                    .unwrap_or(0);
                // Checks the verifier settles meanwhile change the Checks tab.
                let checks = details.tab == CHECKS
                    && details.fetched_checks
                        != details
                            .task_id()
                            .and_then(|id| self.view.task_check_events.get(id))
                            .copied()
                            .unwrap_or(0);
                let latest = self
                    .task_row_round(&details.key)
                    .or_else(|| {
                        self.view.agents.get(&details.key).map(|agent| {
                            if agent.running {
                                u64::from(agent.turns)
                            } else {
                                u64::from(agent.turns.saturating_sub(1))
                            }
                        })
                    })
                    .unwrap_or(0);
                let rounds_changed = latest != details.latest_observed;
                let session = matches!(details.tab, 0 | 1 | details::REVIEW | details::VERIFIER)
                    && details.selected_round.is_none()
                    && details.fetched_events != self.view.after;
                fetched != Some(now) || checks || session || rounds_changed
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
            self.fetch_run();
            return;
        };
        let Some(key) = self.details.as_ref().map(|details| details.key.clone()) else {
            return;
        };
        let latest_observed = self.task_row_round(&key).unwrap_or(0);
        let selected_round = self
            .details
            .as_ref()
            .and_then(|details| details.selected_round);
        let round = selected_round
            .or_else(|| self.task_row_round(&key))
            .unwrap_or(0);
        self.task_request_id += 1;
        let request_id = self.task_request_id;
        if let Some(details) = self
            .details
            .as_mut()
            .filter(|details| details.task_id() == Some(id.as_str()))
        {
            details.fetch_in_flight = true;
            details.latest_observed = latest_observed;
            details.fetched_events = self.view.after;
            details.fetched_round = Some(round);
            details.fetched_generation = Some(self.generation);
            details.fetched_plan = Some(plan.clone());
            details.fetched_checks = self.view.task_check_events.get(&id).copied().unwrap_or(0);
        }
        let history_id = self
            .details
            .as_ref()
            .and_then(|details| details.task_uuid.clone())
            .unwrap_or_else(|| id.clone());
        let checks_only = self
            .details
            .as_ref()
            .is_some_and(|details| details.tab == CHECKS);
        let plan_id = plan;
        let conversation = self.conversation.clone();
        let generation = self.generation;
        let client = self.client.clone();
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            let _ = updates.send(
                match inspection::task_snapshot(
                    &client,
                    &conversation,
                    &plan_id,
                    &history_id,
                    selected_round,
                    checks_only,
                ) {
                    Ok(task) => Update::Task {
                        conversation,
                        plan: Some(plan_id),
                        generation,
                        request_id,
                        key,
                        round,
                        task: task.task,
                        documents: task.documents,
                    },
                    Err(error) => Update::TaskFailed {
                        conversation,
                        generation,
                        plan: plan_id,
                        request_id,
                        key,
                        error: error.to_string(),
                    },
                },
            );
        });
    }

    fn fetch_run(&mut self) {
        let Some(details) = self
            .details
            .as_mut()
            .filter(|details| details.task_id().is_none() && details.key != "conversation")
        else {
            return;
        };
        self.task_request_id += 1;
        let request_id = self.task_request_id;
        let key = details.key.clone();
        let selected = details.selected_round;
        let plan = self.view.plan.clone();
        details.fetch_in_flight = true;
        details.fetched_generation = Some(self.generation);
        details.fetched_plan = plan.clone();
        details.latest_observed = self.view.agents.get(&key).map_or(0, |agent| {
            if agent.running {
                u64::from(agent.turns)
            } else {
                u64::from(agent.turns.saturating_sub(1))
            }
        });
        details.fetched_events = self.view.after;
        let conversation = self.conversation.clone();
        let generation = self.generation;
        let client = self.client.clone();
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            let update = match inspection::run_snapshot(&client, &conversation, &key, selected) {
                Ok(task) => Update::Task {
                    conversation,
                    plan,
                    generation,
                    request_id,
                    key,
                    round: task.task["round"].as_u64().unwrap_or(0),
                    task: task.task,
                    documents: task.documents,
                },
                Err(error) => Update::TaskFailed {
                    conversation,
                    plan: plan.unwrap_or_default(),
                    generation,
                    request_id,
                    key,
                    error: error.to_string(),
                },
            };
            let _ = updates.send(update);
        });
    }

    pub fn open_details(&mut self, key: String) {
        if let Some(details) = &mut self.details {
            details.switch_key(key);
        } else {
            let mut details = Details::new(key);
            details.conversation = self.conversation.clone();
            details.drafts = std::rc::Rc::clone(&self.agent_drafts);
            details.scroll_memory = self.details_scroll_memory.clone();
            details.query_memory = self.details_query_memory.clone();
            details.restore_current_tab();
            self.details = Some(details);
        }
        if let Some(details) = &mut self.details {
            details.task_uuid = details
                .task_id()
                .and_then(|id| self.view.tasks.iter().find(|task| task.id == id))
                .and_then(|task| task.uuid.clone());
            // A task opens on whoever's turn it is: the verifier while it
            // checks, the worker otherwise.
            if let Some(row) = details
                .task_id()
                .and_then(|id| self.view.tasks.iter().find(|task| task.id == id))
            {
                let tab = if row.status == crate::lean::tasks::status::VERIFYING {
                    details::VERIFIER
                } else {
                    0
                };
                details.switch_tab(tab);
            }
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

    fn submit_agent_message(&mut self) {
        let Some(details) = self
            .details
            .as_ref()
            .filter(|details| details.selected_round.is_none())
        else {
            return;
        };
        let (conversation, recipient) = details.composer_key();
        let plan = self.view.plan.clone();
        let expected_run = details
            .task
            .as_ref()
            .and_then(|task| task["_latest_run"].as_str())
            .map(str::to_owned);
        let expected_round = details
            .task
            .as_ref()
            .and_then(|task| task["round"].as_u64())
            .map(|round| round as u32);
        let addressed = if let Some((role, _)) = recipient.split_once(':') {
            let Some(uuid) = details.task.as_ref().and_then(|task| task["id"].as_str()) else {
                return;
            };
            format!("{role}:{uuid}")
        } else {
            recipient.clone()
        };
        let mut drafts = self.agent_drafts.borrow_mut();
        let composer = drafts
            .entry((conversation.clone(), recipient.clone()))
            .or_default();
        if composer.sending || composer.input.text.trim().is_empty() {
            return;
        }
        let text = composer.input.text.clone();
        if composer.sent_text.as_ref() != Some(&text) {
            composer.operation = None;
        }
        composer.sent_text = Some(text.clone());
        let operation = composer
            .operation
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();
        composer.sending = true;
        composer.delivery_receipts.clear();
        composer.error = None;
        composer.address = Some(addressed.clone());
        let client = self.client.clone();
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            let result = client
                .call(&Request::SendToAgent {
                    message: super::store::AgentMessage {
                        conversation: conversation.clone(),
                        plan,
                        recipient: addressed,
                        operation_id: operation.clone(),
                        expected_round,
                        expected_run,
                        text: text.clone(),
                    },
                })
                .map_err(|error| error.to_string());
            let _ = updates.send(Update::AgentMessage {
                conversation,
                recipient,
                operation,
                text,
                result,
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
        let round = self
            .details
            .as_ref()
            .filter(|details| details.task_id().is_some())
            .and_then(|details| {
                details.selected_round.or_else(|| {
                    details
                        .task
                        .as_ref()
                        .and_then(|task| task["round"].as_u64())
                })
            })
            .map(|round| round as u32);
        let run = self
            .details
            .as_ref()
            .filter(|details| details.task_id().is_none())
            .and_then(|details| details.task.as_ref())
            .and_then(|task| task["_run"].as_str())
            .map(str::to_owned);
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
            } else if round.is_none() && run.is_none() {
                client.call(&Request::CommentScoped {
                    conversation: conversation.clone(),
                    plan: plan.clone(),
                    target: editor.target.clone(),
                    text,
                })
            } else {
                client.call(&Request::CommentAt {
                    conversation: conversation.clone(),
                    plan: plan.clone(),
                    target: editor.target.clone(),
                    text,
                    round,
                    run,
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

    fn return_to_main(&mut self) {
        if let Some(ledger) = &mut self.ledger_view {
            ledger.visible = false;
        }
        if let Some(details) = self.details.take() {
            self.details_scroll_memory
                .insert((details.key.clone(), details.tab), details.scroll);
            self.details_query_memory
                .insert((details.key, details.tab), details.query);
        }
        self.diff = None;
        self.plan_view = None;
        self.plan_history = None;
        self.help = None;
        self.picker = None;
        self.confirmation = None;
        self.unblock = None;
        if let Some(editor) = self.editor.take() {
            let key = Self::comment_key(
                &self.conversation,
                self.view.plan.as_deref(),
                &editor.target,
            );
            self.comment_draft_cursors.insert(
                key.clone(),
                (editor.input.cursor, editor.input.preferred_col),
            );
            self.comment_drafts.insert(key, editor.input.text);
        }
        if let Some(editor) = &mut self.config_editor {
            editor.visible = false;
        }
    }

    pub fn apply_update(&mut self, update: Update) {
        self.apply_update_inner(update);
        if let Some(view) = self.ledger_view.as_mut()
            && !view.pending
            && let Some(run) = view.review_run.as_ref()
            && self.view.requirements_approval_run.as_deref() == Some(run)
        {
            let run = view.review_run.take().expect("queued review");
            self.return_to_main();
            self.ledger_view.as_mut().unwrap().visible = true;
            self.ledger_action(ledger::Action::Review(run));
        }
        self.text_selection
            .invalidate_context(&self.selection_context());
    }

    /// Runs a `/command` or sends a message.
    fn dispatch(&mut self, text: &str) {
        let text = text.to_owned();
        let conversation = self.conversation.clone();
        match text.as_str() {
            "" => {}
            "/requirements" => self.open_ledger(),
            "/interview" | "/question" => {
                self.sync_interview();
                if let Some(form) = self
                    .interview_forms
                    .get_mut(&self.conversation)
                    .filter(|form| !form.sent)
                {
                    form.visible = true;
                } else {
                    self.notice = Some("No question is waiting for an answer.".into());
                }
            }
            "/plans" => {
                self.plan_history = Some(planhistory::Browser::new());
                self.load_plan_history(0);
            }
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
            "/accept" if self.view.workflow_phase.as_deref() == Some("interview") => {
                self.dispatch("/planning");
            }
            "/planning" => {
                if self.view.workflow_phase.as_deref() != Some("interview") {
                    self.notice = Some("✗ this session is not in interview".into());
                } else if self.view.requirements_summary.is_empty()
                    || self.sending
                    || !self.input.text.is_empty()
                {
                    self.notice = Some(
                        "✗ send your answers and wait for the requirements summary first".into(),
                    );
                } else {
                    self.request(
                        Request::Send { conversation, text },
                        Some("requirements approved; planning started"),
                    );
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

    /// Opens actions for the named or selected step, falling back to a
    /// stopped step or an active task that can be restarted.
    pub fn open_unblock(&mut self, named: Option<String>) {
        if named.as_deref() == Some(ORCHESTRATOR)
            || (named.is_none() && self.selected == ORCHESTRATOR)
            || self.view.plan.is_none()
        {
            self.unblock = Some(unblock::Unblock::orchestrator());
            return;
        }
        let Some(plan) = self.view.plan.clone() else {
            return;
        };
        let selected = self.selected.split_once(':').map(|(_, id)| id.to_owned());
        let wanted = named.or(selected);
        let task = wanted
            .as_deref()
            .and_then(|id| self.view.tasks.iter().find(|t| t.id == id))
            .filter(|t| {
                unblock::is_blocked(&t.status)
                    || matches!(t.status.as_str(), "queued" | "working" | "verifying")
            })
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
            })
            .or_else(|| {
                self.view
                    .tasks
                    .iter()
                    .find(|t| matches!(t.status.as_str(), "working" | "verifying" | "queued"))
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

    pub(super) fn new_conversation(&self) {
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

    pub fn command(&mut self, command: &str) {
        self.dispatch(command);
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
        let branch = self.view.session_branch.as_ref().or_else(|| {
            self.projects
                .get(self.active_project)
                .and_then(|project| project.branch.as_ref())
        });
        let label = match branch {
            Some(branch) => format!("{label} · branch: {branch}"),
            None => label,
        };
        let mut line = view::render_header_with(
            &label,
            &self.conversation,
            self.view.autonomous,
            self.header_status().trim_end_matches(" · "),
            panel_focus,
            width,
            self.view.controls(),
        );
        if let Some(notice) = &self.notice {
            line.spans.insert(
                0,
                Span::styled(
                    format!(" {} · ", sanitize(notice)),
                    Theme::from_env().title(),
                ),
            );
        }
        line
    }

    /// The header's status text: the stage, then what is going on.
    fn header_status(&self) -> String {
        let status = if self.view.paused { "paused" } else { "" };
        let stage = self.view.stage();
        match stage {
            view::Stage::Fresh => format!("1/3 interview · {status}"),
            view::Stage::Interview => format!("1/3 interview · {status}"),
            view::Stage::Planning => format!("2/3 planning · {status}"),
            view::Stage::Review => format!("2/3 plan review · {status}"),
            view::Stage::Executing => format!("3/3 implementation · {status} [Task actions]"),
            // The way out is one click away whenever something is stuck.
            view::Stage::Blocked => format!("{} · {status} [Unblock]", stage.label()),
            _ => format!("{} · {status}", stage.label()),
        }
    }

    pub fn key(&mut self, key: event::KeyEvent) -> bool {
        let keep_running = self.key_inner(key);
        self.text_selection
            .invalidate_context(&self.selection_context());
        if self.selection_blocked() {
            self.text_selection = Default::default();
        }
        keep_running
    }

    fn sync_interview(&mut self) {
        if let Some((event, question)) = &self.view.interview_question {
            let same = self
                .interview_forms
                .get(&self.conversation)
                .is_some_and(|form| form.event == *event);
            if !same {
                self.interview_forms.insert(
                    self.conversation.clone(),
                    interview::Form::new(*event, question.clone()),
                );
            }
        }
    }

    fn interview_visible(&self) -> bool {
        (self.view.workflow_phase.as_deref() == Some("interview") || self.view.user_decision)
            && self
                .view
                .interview_question
                .as_ref()
                .is_some_and(|(event, _)| {
                    self.interview_forms
                        .get(&self.conversation)
                        .is_some_and(|form| form.event == *event && form.visible && !form.sent)
                })
    }

    fn send_interview_answer(&mut self, text: String) {
        let Some(form) = self.interview_forms.get_mut(&self.conversation) else {
            return;
        };
        if form.sending || form.sent {
            return;
        }
        form.sending = true;
        form.error = None;
        let question = form.event;
        let client = self.client.clone();
        let updates = self.updates.clone();
        let conversation = self.conversation.clone();
        std::thread::spawn(move || {
            let result = client
                .call(&Request::Send {
                    conversation: conversation.clone(),
                    text,
                })
                .map(|_| ())
                .map_err(|error| error.to_string());
            let _ = updates.send(Update::InterviewResult {
                conversation,
                question,
                result,
            });
        });
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

    fn selection_context(&self) -> String {
        use std::hash::{Hash, Hasher};
        let details = self.details.as_ref().map(|details| {
            let mut query = std::collections::hash_map::DefaultHasher::new();
            details.query.hash(&mut query);
            (&details.key, details.tab, query.finish())
        });
        format!(
            "{}:{}:{}:{:?}:{:?}",
            self.conversation,
            self.generation,
            self.view.after,
            details,
            self.plan_view.as_ref().map(|p| &p.plan_id)
        )
    }

    fn selection_blocked(&self) -> bool {
        if self.plan_history.is_some() {
            return true;
        }
        if self.interview_visible() {
            return true;
        }
        if self.ledger_view.as_ref().is_some_and(|v| v.visible) {
            return true;
        }
        self.help.is_some()
            || self.picker.is_some()
            || self.editor.is_some()
            || self.diff.is_some()
            || self.unblock.is_some()
            || self.trust_prompt.is_some()
            || self.project_trust.is_some()
            || self
                .confirmation
                .as_ref()
                .is_some_and(|c| c.status == ConfirmationStatus::Pending)
            || self.config_editor.as_ref().is_some_and(|e| e.visible)
    }

    fn bar_height(area: Rect) -> u16 {
        if area.width >= SIDE_BY_SIDE_BREAKPOINT && area.height >= 30 {
            3
        } else {
            2
        }
    }

    fn main_areas(area: Rect) -> [Rect; 3] {
        Layout::vertical([
            Constraint::Min(0),
            Constraint::Length(u16::from(area.height >= 6)),
            Constraint::Length(Self::bar_height(area)),
        ])
        .areas(area)
    }

    fn mouse_scroll(&mut self, down: bool) {
        let code = if down { KeyCode::Down } else { KeyCode::Up };
        if self.help.is_some() {
            self.help_scroll = if down {
                (self.help_scroll + 3).min(self.help_max_scroll())
            } else {
                self.help_scroll.saturating_sub(3)
            };
        } else if let Some(pv) = &mut self.plan_view {
            pv.key(event::KeyEvent::new(code, KeyModifiers::NONE), &self.view);
        } else if let Some(diff) = &mut self.diff {
            diff.key(code);
        } else if let Some(details) = &mut self.details {
            details.pending_g = false;
            if !down {
                details.follow = false;
            }
            details.scroll = if down {
                details.scroll.saturating_add(3)
            } else {
                details.scroll.saturating_sub(3)
            };
        } else if self.focus == Focus::Graph {
            let graph = Graph::new_with_viewport(
                &self.view,
                self.graph_filter.as_deref(),
                &self.graph_collapsed,
                self.graph_grouping,
                self.graph_viewport,
            );
            self.scroll_graph_node(&graph, down, 3);
        } else {
            self.scroll = if down {
                self.scroll.saturating_sub(3)
            } else {
                self.scroll + 3
            };
        }
    }

    pub fn mouse(&mut self, mut mouse: event::MouseEvent) -> bool {
        if self.native_focused() {
            if let Some(details) = &self.details {
                let area = details.native_areas[details.active_panel];
                if area.contains((mouse.column, mouse.row).into()) {
                    mouse.column -= area.x;
                    mouse.row -= area.y;
                    self.native_send(crate::lean::terminal::Input::Mouse { event: mouse });
                }
            }
            return true;
        }
        if self.plan_history.is_some() {
            return true;
        }
        self.sync_interview();
        if let Some(view) = self.ledger_view.as_mut().filter(|v| v.visible) {
            let action = view.mouse(mouse);
            self.ledger_action(action);
            return true;
        }
        if self.selection_blocked() {
            self.text_selection = Default::default();
        }
        self.text_selection
            .invalidate_context(&self.selection_context());
        if self.config_editor.as_ref().is_some_and(|e| e.visible) {
            return true;
        }
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
        if self.interview_visible() {
            return true;
        }
        if !self.selection_blocked() {
            self.text_selection.mouse(mouse);
        }
        match mouse.kind {
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                self.mouse_scroll(mouse.kind == MouseEventKind::ScrollDown);
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
                if self
                    .phase_action_area
                    .is_some_and(|area| area.contains((col, row).into()))
                {
                    if let Some(command) = self.phase_action_command {
                        self.command(command);
                    }
                    return true;
                }
                let [body, _, bar] = Self::main_areas(area);
                if row == bar.y + (Self::bar_height(area) - 1) / 2
                    && col >= bar.right().saturating_sub(3)
                    && bar.contains((col, row).into())
                {
                    self.show_help(self.help_here());
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
                            && x < node.x.saturating_add(node.width)
                            && y >= node.y
                            && y < node.y.saturating_add(node.height)
                    }) {
                        self.select_node(node.key.clone());
                    }
                    self.focus = Focus::Graph;
                    return true;
                }
                if row == bar.y + (Self::bar_height(area) - 1) / 2
                    && bar.contains((col, row).into())
                {
                    let header_line = self.header_line(bar.width.saturating_sub(4));
                    let controls = self.view.controls();
                    let header_text: String = header_line
                        .spans
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect();
                    // Find bracketed token at column
                    let mut cur_col = 0usize;
                    let target_col = usize::from(col.saturating_sub(bar.x));
                    for (idx, grapheme) in
                        unicode_segmentation::UnicodeSegmentation::grapheme_indices(
                            header_text.as_str(),
                            true,
                        )
                    {
                        let w = unicode_width::UnicodeWidthStr::width(grapheme);
                        if target_col >= cur_col && target_col < cur_col + w {
                            if let Some(open) = header_text[..idx + grapheme.len()].rfind('[')
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
                                    "[Unblock]" | "[Task actions]" => self.command("/unblock"),
                                    "[Chat]" => self.focus = Focus::Chat,
                                    "[Team]" => self.focus = Focus::Graph,
                                    _ => {}
                                }
                            }
                            break;
                        }
                        cur_col += w;
                    }
                } else if body.contains((col, row).into()) && area.width >= SIDE_BY_SIDE_BREAKPOINT
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
        if cell.fg == theme::ayu::DIM || cell.fg == theme::ayu::FAINT {
            cell.modifier.insert(Modifier::DIM);
        }
        cell.set_fg(Color::Reset).set_bg(Color::Reset);
    }
}

/// Whether the terminal advertises 24-bit color; `ANTISLOP_TRUECOLOR=1` forces it.
pub fn truecolor_terminal() -> bool {
    let set = |name: &str| std::env::var_os(name).is_some_and(|value| !value.is_empty());
    set("ANTISLOP_TRUECOLOR")
        || std::env::var("COLORTERM")
            .is_ok_and(|value| matches!(value.as_str(), "truecolor" | "24bit"))
}

/// The xterm-256 color closest to an RGB value: the 6x6x6 cube or the gray ramp.
pub fn nearest_indexed(r: u8, g: u8, b: u8) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let level = |v: u8| {
        (0..6)
            .min_by_key(|&i| (i32::from(LEVELS[i]) - i32::from(v)).abs())
            .unwrap_or(0)
    };
    let distance = |a: (u8, u8, u8)| {
        let d = |x: u8, y: u8| (i32::from(x) - i32::from(y)).pow(2);
        d(a.0, r) + d(a.1, g) + d(a.2, b)
    };
    let (ri, gi, bi) = (level(r), level(g), level(b));
    let cube = (LEVELS[ri], LEVELS[gi], LEVELS[bi]);
    let average = ((u32::from(r) + u32::from(g) + u32::from(b)) / 3) as i32;
    let step = ((average - 8 + 5) / 10).clamp(0, 23);
    let gray = (8 + 10 * step) as u8;
    if distance((gray, gray, gray)) < distance(cube) {
        232 + step as u8
    } else {
        (16 + 36 * ri + 6 * gi + bi) as u8
    }
}

/// Maps every RGB color to its nearest xterm-256 index for terminals without true color.
pub fn to_indexed_colors(buffer: &mut ratatui::buffer::Buffer) {
    let map = |color: Color| match color {
        Color::Rgb(r, g, b) => Color::Indexed(nearest_indexed(r, g, b)),
        other => other,
    };
    for cell in &mut buffer.content {
        cell.fg = map(cell.fg);
        cell.bg = map(cell.bg);
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
    if task == ORCHESTRATOR && action == crate::lean::RecoveryAction::RestartAgent {
        return client
            .call(&Request::RestartOrchestrator {
                conversation: conversation.into(),
            })
            .map(|reply| {
                reply
                    .as_str()
                    .unwrap_or("restarting the orchestrator")
                    .to_owned()
            })
            .map_err(|error| error.to_string());
    }
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

#[cfg(test)]
pub mod tests {
    use super::view::render_header;
    use super::*;
    use ratatui::buffer::Buffer;

    #[test]
    fn ascii_replaces_box_drawing_marks_and_letters() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 12, 1));
        buffer.set_string(0, 0, "╭─✓ ąębć 漢", Style::default());
        in_ascii(&mut buffer);
        let text: String = buffer.content.iter().map(|cell| cell.symbol()).collect();
        assert_eq!(text, "+-+ aebc ?  ");
    }

    #[test]
    fn rgb_falls_back_to_the_nearest_256_color() {
        assert_eq!(nearest_indexed(0, 0, 0), 16);
        assert_eq!(nearest_indexed(255, 255, 255), 231);
        assert_eq!(nearest_indexed(0xFF, 0x00, 0x00), 196);
        // Ayu's near-black background lands on the gray ramp, not the cube.
        assert!((232..=235).contains(&nearest_indexed(0x0B, 0x0E, 0x14)));
        let mut buffer = Buffer::empty(Rect::new(0, 0, 1, 1));
        buffer.set_string(
            0,
            0,
            "a",
            Style::default().fg(Color::Rgb(255, 0, 0)).bg(Color::Red),
        );
        to_indexed_colors(&mut buffer);
        assert_eq!(buffer.content[0].fg, Color::Indexed(196));
        assert_eq!(buffer.content[0].bg, Color::Red);
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
    fn help_and_picker_keep_ownership_when_routing_keys() {
        let mut app = test_app();
        app.input = InputBuffer::with_text("chat draft 👩‍💻");
        app.picker = Some(Picker::new(Kind::History));
        app.show_help(Help::Picker);

        assert!(app.key(key_press(KeyCode::Char('x'))));
        assert_eq!(app.picker.as_ref().unwrap().query, "");
        assert_eq!(app.input.text, "chat draft 👩‍💻");

        assert!(app.key(key_press(KeyCode::Esc)));
        assert!(app.help.is_none());
        assert!(app.picker.is_some());
        assert!(app.key(key_press(KeyCode::Char('x'))));
        assert_eq!(app.picker.as_ref().unwrap().query, "x");
        assert_eq!(app.input.text, "chat draft 👩‍💻");
    }

    #[test]
    fn native_escape_accepts_legacy_and_enhanced_ctrl_bracket() {
        for ch in [']', '5'] {
            let mut app = test_app();
            app.input = InputBuffer::with_text("Unsent draft 👩‍💻");
            let mut details = Details::new("worker:t1".into());
            details.native_focus = true;
            app.details = Some(details);
            assert!(app.native_focused());

            app.key(key_ctrl(ch));

            assert!(!app.native_focused());
            assert!(app.details.is_some());
            assert_eq!(app.input.text, "Unsent draft 👩‍💻");
        }
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
    fn chat_history_escape_restores_grapheme_safe_draft_cursor() {
        let mut app = test_app();
        let draft = "e\u{301}👩‍💻 draft";
        let cursor = "e\u{301}👩‍💻".len();
        app.input = InputBuffer::with_text(draft);
        app.input.cursor = cursor;
        app.history = vec!["previous message".into()];

        app.key(key_press(KeyCode::Up));
        assert_eq!(app.input.text, "previous message");
        app.key(key_press(KeyCode::Esc));

        assert_eq!(app.input.text, draft);
        assert_eq!(app.input.cursor, cursor);
        app.input.backspace();
        assert_eq!(app.input.text, "e\u{301} draft");
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
    fn execution_decision_opens_the_question_form_without_discarding_chat() {
        let mut app = test_app();
        app.input = InputBuffer::with_text("Unsent chat draft 👩‍💻");
        app.view.workflow_phase = Some("implementation".into());
        app.view.user_decision = true;
        let question = serde_json::from_value(serde_json::json!({"id":"metadata","context":"Scope decision","question":"Allow metadata?","options":[{"label":"Allow"},{"label":"Reject"}]})).unwrap();
        app.view.interview_question = Some((42, question));
        app.sync_interview();
        assert!(app.interview_visible());
        assert_eq!(app.input.text, "Unsent chat draft 👩‍💻");
        app.view.user_decision = false;
        assert!(!app.interview_visible());
    }

    #[test]
    fn interview_draft_survives_background_failure_and_conversation_switches() {
        let mut app = test_app();
        app.conversation = "interview-a".into();
        app.input = InputBuffer::with_text("Unsent chat draft");
        let question = serde_json::from_value(serde_json::json!({"id":"scope","context":"Scope affects timing.","question":"Which scope?","options":[{"label":"Small"},{"label":"Full"}]})).unwrap();
        app.view.workflow_phase = Some("interview".into());
        app.view.interview_question = Some((42, question));
        app.sync_interview();
        app.interview_forms
            .get_mut("interview-a")
            .unwrap()
            .paste("My own answer 👩‍💻");
        app.help = Some(Help::Chat);
        app.key(event::KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        app.paste("must stay out of every draft");
        assert_eq!(
            app.interview_forms["interview-a"].input.text,
            "My own answer 👩‍💻"
        );
        app.help = None;
        app.interview_forms.get_mut("interview-a").unwrap().sending = true;
        app.open("interview-b".into());
        app.stop_poller.store(true, Ordering::SeqCst);
        app.apply_update(Update::InterviewResult {
            conversation: "interview-a".into(),
            question: 42,
            result: Err("socket closed".into()),
        });
        assert_eq!(
            app.interview_forms["interview-a"].error.as_deref(),
            Some("socket closed")
        );
        assert!(!app.interview_forms["interview-a"].sending);
        app.open("interview-a".into());
        app.stop_poller.store(true, Ordering::SeqCst);
        assert_eq!(app.input.text, "Unsent chat draft");
        assert_eq!(
            app.interview_forms["interview-a"].input.text,
            "My own answer 👩‍💻"
        );
        app.apply_update(Update::InterviewResult {
            conversation: "interview-a".into(),
            question: 41,
            result: Ok(()),
        });
        assert!(!app.interview_forms["interview-a"].sent);
    }

    fn requirements_event(app: &mut App, seq: u64, run: &str, resolved: bool) {
        let mut events = vec![
            serde_json::json!({"seq":seq,"kind":"ledger_approval_required","payload":{"run":run}}),
        ];
        if resolved {
            events.push(serde_json::json!({"seq":seq+1,"kind":"ledger_proposal_rejected","payload":{"run":run}}));
        }
        app.apply_update(Update::Events {
            conversation: app.conversation.clone(),
            generation: app.generation,
            events,
            idle: true,
            autonomous: false,
        });
    }
    fn review_app() -> App {
        let mut app = test_app();
        app.input = InputBuffer::with_text("Unsent chat 👩‍💻");
        let mut ledger = ledger::LedgerView::new();
        ledger.preview = Some(serde_json::json!({"proposal_id":"p1","diff_hash":"hash"}));
        ledger.proposed_change = Some(crate::lean::ledger::LedgerChange {
            conversation: app.conversation.clone(),
            run_id: "r1".into(),
            operation_id: "op1".into(),
            expected_spec_revision: 0,
            reason: "fix".into(),
            entries: vec![],
        });
        ledger.request_id = 12;
        app.ledger_view = Some(ledger);
        app
    }
    #[test]
    fn requirements_decisions_return_to_main_only_after_success() {
        for kind in ["applied", "rejected"] {
            let mut app = review_app();
            app.details = Some(Details::new("worker:t1".into()));
            app.plan_view = Some(PlanView::new("plan".into()));
            app.editor = Some(Editor {
                target: "orchestrator".into(),
                input: InputBuffer::with_text("Comment draft"),
            });
            app.ledger_view.as_mut().unwrap().rejection =
                InputBuffer::with_text("Correction draft");
            app.apply_update(Update::Ledger {
                conversation: app.conversation.clone(),
                request_id: 12,
                kind: kind.into(),
                reply: Err("raw failure".into()),
            });
            assert!(app.ledger_view.as_ref().unwrap().visible);
            assert!(app.ledger_view.as_ref().unwrap().preview.is_some());
            assert_eq!(
                app.ledger_view.as_ref().unwrap().rejection.text,
                "Correction draft"
            );
            app.apply_update(Update::Ledger {
                conversation: app.conversation.clone(),
                request_id: 12,
                kind: kind.into(),
                reply: Ok(serde_json::json!({"spec_revision":1})),
            });
            assert!(!app.ledger_view.as_ref().unwrap().visible);
            assert!(app.details.is_none() && app.plan_view.is_none() && app.editor.is_none());
            assert_eq!(app.input.text, "Unsent chat 👩‍💻");
            assert!(
                app.comment_drafts
                    .values()
                    .any(|text| text == "Comment draft")
            );
        }
    }
    #[test]
    fn replay_opens_only_pending_reviews_and_respects_dismissal() {
        let mut app = test_app();
        requirements_event(&mut app, 1, "r1", true);
        assert!(app.ledger_view.is_none());
        requirements_event(&mut app, 3, "r1", false);
        assert!(app.ledger_view.as_ref().unwrap().pending);
        let mut view = ledger::LedgerView::new();
        view.approval_event = 3;
        view.visible = false;
        app.ledger_view = Some(view);
        requirements_event(&mut app, 3, "r1", false);
        assert!(!app.ledger_view.as_ref().unwrap().visible);
        requirements_event(&mut app, 4, "r1", false);
        assert!(app.ledger_view.as_ref().unwrap().visible);
    }
    #[test]
    fn queued_review_waits_for_write_and_late_reply_cannot_replace_it() {
        let mut app = review_app();
        app.ledger_view.as_mut().unwrap().pending = true;
        app.ledger_view.as_mut().unwrap().pending_kind = "applied".into();
        requirements_event(&mut app, 2, "r2", false);
        assert_eq!(
            app.ledger_view.as_ref().unwrap().review_run.as_deref(),
            Some("r2")
        );
        app.apply_update(Update::Ledger {
            conversation: app.conversation.clone(),
            request_id: 12,
            kind: "applied".into(),
            reply: Ok(serde_json::json!({})),
        });
        let view = app.ledger_view.as_ref().unwrap();
        assert!(view.visible && view.pending);
        assert_eq!(view.pending_run.as_deref(), Some("r2"));
        app.apply_update(Update::Ledger {
            conversation: app.conversation.clone(),
            request_id: 12,
            kind: "preview".into(),
            reply: Ok(serde_json::json!({"proposal":{"proposal_id":"old"}})),
        });
        assert!(app.ledger_view.as_ref().unwrap().preview.is_none());
    }
    #[test]
    fn queued_review_survives_waiting_for_conversation_history() {
        let mut app = review_app();
        app.ledger_view.as_mut().unwrap().review_run = Some("r2".into());
        app.apply_update(Update::Ledger {
            conversation: app.conversation.clone(),
            request_id: 11,
            kind: "preview".into(),
            reply: Err("late".into()),
        });
        assert_eq!(
            app.ledger_view.as_ref().unwrap().review_run.as_deref(),
            Some("r2")
        );
        requirements_event(&mut app, 2, "r2", false);
        assert!(app.ledger_view.as_ref().unwrap().pending);
        assert_eq!(
            app.ledger_view.as_ref().unwrap().pending_run.as_deref(),
            Some("r2")
        );
    }
    #[test]
    fn review_escape_closes_details_and_preserves_search_and_chat() {
        let mut app = review_app();
        app.ledger_view.as_mut().unwrap().query = InputBuffer::with_text("filter");
        let mut details = Details::new("worker:t1".into());
        details.native_focus = true;
        app.details = Some(details);
        assert!(!app.native_focused());
        app.paste("must not edit chat");
        assert_eq!(app.input.text, "Unsent chat 👩‍💻");
        app.key(key_press(KeyCode::Esc));
        assert!(app.details.is_none());
        assert!(!app.ledger_view.as_ref().unwrap().visible);
        assert_eq!(app.ledger_view.as_ref().unwrap().query.text, "filter");
        assert_eq!(app.input.text, "Unsent chat 👩‍💻");
    }
    #[test]
    fn requirements_approval_automatically_opens_comparison() {
        let mut app = test_app();
        app.input = InputBuffer::with_text("Draft 👩‍💻");
        app.apply_update(Update::Events {
            conversation: app.conversation.clone(),
            generation: app.generation,
            events: vec![serde_json::json!({
                "seq": 1, "kind": "ledger_approval_required", "payload": {"run": "r1"}
            })],
            idle: true,
            autonomous: false,
        });
        let ledger = app.ledger_view.as_ref().unwrap();
        assert!(ledger.visible);
        assert!(ledger.pending);
        assert_eq!(ledger.pending_kind, "preview");
        assert_eq!(app.input.text, "Draft 👩‍💻");
    }

    #[test]
    fn requirements_button_opens_ledger_and_preserves_chat_draft() {
        use ratatui::{Terminal, backend::TestBackend};
        let mut app = test_app();
        app.view.requirements_approval_run = Some("run1".into());
        app.input = InputBuffer::with_text("My comment 👩‍💻");
        let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let area = app
            .phase_action_area
            .expect("requirements action above input");
        app.mouse(event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: area.x,
            row: area.y,
            modifiers: KeyModifiers::NONE,
        });
        assert!(app.ledger_view.as_ref().is_some_and(|view| view.visible));
        assert_eq!(app.input.text, "My comment 👩‍💻");
    }

    #[test]
    fn interview_shortcut_guides_approval_and_preserves_unsent_answers() {
        let mut app = test_app();
        app.view.workflow_phase = Some("interview".into());
        app.key(event::KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL,
        ));
        assert!(
            app.notice
                .as_deref()
                .unwrap()
                .contains("requirements summary")
        );
        app.view.requirements_summary = "Goal and criteria".into();
        app.input = InputBuffer::with_text("Correction 👩‍💻");
        app.key(event::KeyEvent::new(
            KeyCode::Char('a'),
            KeyModifiers::CONTROL,
        ));
        assert_eq!(app.input.text, "Correction 👩‍💻");
        assert_eq!(app.input.cursor, 0);
        app.command("/planning");
        assert!(app.notice.as_deref().unwrap().contains("send your answers"));
        assert_eq!(app.input.text, "Correction 👩‍💻");
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
    fn comment_drafts_restore_grapheme_cursor_per_conversation() {
        let mut app = test_app();
        app.conversation = "conversation-one".into();
        let mut input = InputBuffer::with_text("e\u{301}👩‍💻");
        input.cursor = 1;
        input.preferred_col = Some(4);
        app.save_comment_draft("task", input);

        app.conversation = "conversation-two".into();
        assert!(app.get_comment_draft("task").is_empty());
        app.conversation = "conversation-one".into();
        let restored = app.get_comment_draft("task");
        assert_eq!(restored.text, "e\u{301}👩‍💻");
        assert_eq!(restored.cursor, "e\u{301}".len());
        assert_eq!(restored.preferred_col, Some(4));
    }

    #[test]
    fn failed_background_comment_keeps_its_grapheme_cursor() {
        let mut app = test_app();
        app.open("conversation-a".into());
        app.sending_comment = true;
        app.comment_request_id = 17;
        app.editor = Some(Editor {
            target: "worker:t1".into(),
            input: InputBuffer::with_text("active draft"),
        });
        app.open("conversation-b".into());

        let mut failed_input = InputBuffer::with_text("👩‍💻 note");
        failed_input.cursor = "👩‍💻".len();
        failed_input.preferred_col = Some(4);
        app.apply_update(Update::CommentResult {
            conversation: "conversation-a".into(),
            plan: None,
            generation: 1,
            request_id: 17,
            editor: Editor {
                target: "worker:t3".into(),
                input: failed_input,
            },
            result: Err("connection interrupted".into()),
        });

        app.open("conversation-a".into());
        let draft = app.get_comment_draft("worker:t3");
        assert_eq!(draft.text, "👩‍💻 note");
        assert_eq!(draft.cursor, "👩‍💻".len());
        assert_eq!(draft.preferred_col, Some(4));
        assert_eq!(app.editor.as_ref().unwrap().input.text, "active draft");
    }

    #[test]
    fn render_coordination_guards_below_minimum_and_draws_at_minimum() {
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

        let mut app = test_app();
        let mut terminal = Terminal::new(TestBackend::new(12, 5)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(!text.contains("Terminal too small"));
        assert_eq!(app.terminal_area, Rect::new(0, 0, 12, 5));
    }

    #[test]
    fn interview_form_is_drawn_inside_the_chat_panel_beside_the_canvas() {
        use ratatui::{Terminal, backend::TestBackend};

        let mut app = test_app();
        app.conversation = "interview-a".into();
        let question = serde_json::from_value(serde_json::json!({"id":"scope","context":"Scope affects timing.","question":"Which scope?","options":[{"label":"Small"},{"label":"Full"}]})).unwrap();
        app.view.workflow_phase = Some("interview".into());
        app.view.interview_question = Some((42, question));
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();

        let chat = app.chat_area;
        let graph = app.graph_area.expect("canvas stays visible");
        assert!(chat.width > 0 && chat.right() <= graph.x);
        let buffer = terminal.backend().buffer();
        let row = |y: u16, x0: u16, x1: u16| -> String {
            (x0..x1).map(|x| buffer[(x, y)].symbol()).collect()
        };
        let title_row = (0..24)
            .find(|&y| row(y, 0, 120).contains("Question · choose an answer"))
            .expect("form is drawn");
        let line = row(title_row, 0, 120);
        let col = line[..line.find("Question").unwrap()].chars().count() as u16;
        assert!(col < chat.right(), "form title must sit in the chat panel");
        assert!(
            (0..24).any(|y| row(y, graph.x, graph.right()).contains("orchestrator")),
            "canvas must keep rendering next to the form"
        );
    }

    #[test]
    fn agent_canvas_remains_available_in_every_workflow_phase() {
        use ratatui::{Terminal, backend::TestBackend};

        for phase in ["interview", "planning", "review", "executing", "done"] {
            for width in [84, 120] {
                let mut app = test_app();
                app.view.workflow_phase = Some(phase.into());
                app.input.insert_str("unsent draft");
                let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
                terminal.draw(|frame| app.draw(frame)).unwrap();
                if width >= SIDE_BY_SIDE_BREAKPOINT {
                    assert!(app.graph_area.is_some(), "{phase}");
                    assert!(app.text_area.is_some(), "{phase}");
                }
                app.key(event::KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
                terminal.draw(|frame| app.draw(frame)).unwrap();
                assert_eq!(app.focus, Focus::Graph, "{phase}");
                assert!(app.graph_area.is_some(), "{phase}");
                assert_eq!(app.input.text, "unsent draft");
            }
        }
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
        assert!(text.contains("test"));
        assert!(!text.contains("antislop"));
        assert!(text.contains("1/3 interview"));
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
        let mut text2: String = buffer2.content.iter().map(|c| c.symbol()).collect();
        // Additional help entries may move a shortcut off the last page.
        for _ in 0..bottom {
            app.key(event::KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
            terminal.draw(|frame| app.draw(frame)).unwrap();
            text2.extend(
                terminal
                    .backend()
                    .buffer()
                    .content
                    .iter()
                    .map(|cell| cell.symbol()),
            );
        }

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
        assert!(app.help_max_scroll() < max_scroll);
        assert_eq!(
            app.help_scroll,
            app.help_max_scroll(),
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
        // The overlay is only hidden when the viewport is too small for it
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
    fn transcript_cache_reuses_rows_and_matches_fresh_layout() {
        use ratatui::{Terminal, backend::TestBackend};
        let mut app = test_app();
        for i in 0..40 {
            app.view.cards.push(TranscriptCard::Message(MessageEntry {
                id: i.to_string(),
                sender: "You".into(),
                target: "orchestrator".into(),
                text: format!("message {i}: {}", "👩‍💻 e\u{301} tekst ".repeat(20)),
                state: "sent".into(),
                kind: "user".into(),
            }));
        }
        let mut term = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let theme = Theme::new(true, false);
        let draw = |app: &mut App, term: &mut Terminal<TestBackend>, theme: &Theme| {
            term.draw(|frame| app.draw_transcript(frame, frame.area(), theme))
                .unwrap();
            term.backend().buffer().clone()
        };
        draw(&mut app, &mut term, &theme);
        let original = app.transcript_layout.entries[0].rows.as_ptr();
        draw(&mut app, &mut term, &theme);
        assert_eq!(original, app.transcript_layout.entries[0].rows.as_ptr());
        if let TranscriptCard::Message(msg) = &mut app.view.cards[39] {
            msg.text.push_str(" updated");
            msg.state = "answered".into();
        }
        draw(&mut app, &mut term, &theme);
        assert_eq!(original, app.transcript_layout.entries[0].rows.as_ptr());
        for (scroll, found, theme) in [
            (0, None, theme),
            (15, None, theme),
            (usize::MAX, Some(20), theme),
            (usize::MAX, None, Theme::new(false, true)),
        ] {
            app.scroll = scroll;
            app.found = found;
            let cached = draw(&mut app, &mut term, &theme);
            let settled_scroll = app.scroll;
            app.transcript_layout = TranscriptLayout::default();
            app.scroll = settled_scroll;
            assert_eq!(cached, draw(&mut app, &mut term, &theme));
        }
        term.resize(Rect::new(0, 0, 40, 16)).unwrap();
        let cached = draw(&mut app, &mut term, &theme);
        app.transcript_layout = TranscriptLayout::default();
        assert_eq!(cached, draw(&mut app, &mut term, &theme));
        app.view.cards.truncate(1);
        draw(&mut app, &mut term, &theme);
        assert_eq!(app.transcript_layout.entries.len(), 1);
        app.view.cards.push(TranscriptCard::Comment {
            id: 7,
            target: "plan".into(),
            text: "please fix".into(),
            status: "open".into(),
        });
        app.view.comments.insert(
            7,
            view::Comment {
                id: 7,
                plan_id: None,
                target: "plan".into(),
                text: "please fix".into(),
                status: "open".into(),
            },
        );
        app.scroll = 0;
        draw(&mut app, &mut term, &theme);
        app.view.comment_progress.insert(
            7,
            view::CommentProgress {
                state: "failed".into(),
                error: Some("retry needed".into()),
            },
        );
        let cached = draw(&mut app, &mut term, &theme);
        let text: String = cached.content.iter().map(|cell| cell.symbol()).collect();
        assert!(text.contains("retry needed"));
        app.transcript_layout = TranscriptLayout::default();
        assert_eq!(cached, draw(&mut app, &mut term, &theme));
        app.conversation = "other".into();
        app.view.cards.clear();
        draw(&mut app, &mut term, &theme);
        assert!(app.transcript_layout.entries.is_empty());
    }

    #[test]
    fn transcript_zero_area_preserves_viewport_state() {
        use ratatui::{Terminal, backend::TestBackend};
        let mut app = test_app();
        app.scroll = 7;
        app.text_area = Some(Rect::new(1, 2, 3, 4));
        let area = Rect::new(0, 0, 0, 10);
        let mut term = Terminal::new(TestBackend::new(10, 10)).unwrap();
        term.draw(|frame| app.draw_transcript(frame, area, &Theme::new(true, false)))
            .unwrap();
        assert_eq!(app.scroll, 7);
        assert_eq!(app.text_area, Some(Rect::new(1, 2, 3, 4)));
    }

    #[test]
    fn transcript_narrow_area_renders_legacy_rows_safely() {
        use ratatui::{Terminal, backend::TestBackend};
        let mut app = test_app();
        app.view.transcript.push(view::Entry {
            style: Style::default(),
            text: "legacy transcript".into(),
        });
        let area = Rect::new(0, 0, 2, 1);
        let mut term = Terminal::new(TestBackend::new(2, 1)).unwrap();
        term.draw(|frame| app.draw_transcript(frame, area, &Theme::new(false, true)))
            .unwrap();
        assert_eq!(app.text_area, Some(Rect::new(0, 0, 1, 1)));
        assert_eq!(term.backend().buffer().content.len(), 2);
    }

    #[test]
    fn transcript_workflow_fallback_replaces_empty_message() {
        use ratatui::{Terminal, backend::TestBackend};
        let mut app = test_app();
        app.view.workflow_phase = Some("interview".into());
        let mut term = Terminal::new(TestBackend::new(70, 20)).unwrap();
        term.draw(|frame| app.draw_transcript(frame, frame.area(), &Theme::new(true, false)))
            .unwrap();
        let text: String = term
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("Phase 1/3: Interview and requirements"));
        assert!(!text.contains("No messages in this conversation"));
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
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
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
        // Agent updates show in the graph's "now" box only, not above the input.
        assert!(thinking.contains(" now "));
        assert_eq!(thinking.matches("orchestrator: exec ls").count(), 1);

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
        app.selected = "worker:t1".into();
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

    #[test]
    fn the_orchestrator_actions_open_without_a_plan_and_with_an_existing_plan() {
        let mut app = test_app();
        app.focus = Focus::Graph;
        app.selected = ORCHESTRATOR.into();
        for plan in [None, Some("p1".into())] {
            app.view.plan = plan;
            app.key(key_press(KeyCode::Char('u')));
            let modal = app.unblock.take().unwrap();
            assert_eq!(modal.task, ORCHESTRATOR);
            assert_eq!(modal.choices.len(), 1);
            assert_eq!(
                modal.choices[0].action,
                crate::lean::RecoveryAction::RestartAgent
            );
            assert!(!modal.choices[0].asks_text);
        }
    }

    #[test]
    fn an_active_step_can_be_restarted_from_the_actions_menu() {
        let mut app = test_app();
        app.view.plan = Some("p1".into());
        app.view.tasks = vec![view::TaskRow {
            id: "t1".into(),
            status: "working".into(),
            ..Default::default()
        }];
        app.selected = "worker:t1".into();
        app.focus = Focus::Graph;
        app.key(key_press(KeyCode::Char('u')));
        assert_eq!(app.unblock.as_ref().unwrap().task, "t1");
        app.key(key_press(KeyCode::Enter));
        assert_eq!(
            app.editor.as_ref().unwrap().target,
            "unblock:restart_agent:p1:t1"
        );
    }

    fn header_text(app: &App, width: u16) -> String {
        app.header_line(width)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect()
    }

    fn click_header(app: &mut App, width: u16, token: &str) {
        let text = header_text(app, width.saturating_sub(4));
        let at = text
            .find(token)
            .unwrap_or_else(|| panic!("{token} in {text}"));
        let col = unicode_width::UnicodeWidthStr::width(&text[..at]) as u16 + 1;
        app.terminal_area = Rect::new(0, 0, width, 24);
        app.mouse(event::MouseEvent {
            kind: MouseEventKind::Down(event::MouseButton::Left),
            column: col,
            row: App::main_areas(app.terminal_area)[2].y,
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
    fn footer_has_one_status_row_and_help_preserves_the_draft() {
        let mut app = test_app();
        app.paste("draft");
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 50)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let row_text = |buffer: &ratatui::buffer::Buffer, y| -> String {
            (0..120).map(|x| buffer[(x, y)].symbol()).collect()
        };
        let buffer = terminal.backend().buffer();
        assert!(row_text(buffer, 47).trim().is_empty());
        assert!(row_text(buffer, 48).ends_with("[?]"));
        assert!(row_text(buffer, 49).trim().is_empty());
        app.mouse(event::MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 118,
            row: 48,
            modifiers: KeyModifiers::NONE,
        });
        assert_eq!(app.help, Some(Help::Chat));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        app.key(key_press(KeyCode::Esc));
        assert!(app.help.is_none());
        assert_eq!(app.input.text, "draft");
    }

    #[test]
    fn footer_clicks_follow_graphemes_and_the_rendered_row() {
        let mut app = test_app();
        app.project = "Zażółć 👩‍💻 e\u{301}".into();
        for (width, height) in [(80, 24), (180, 50)] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal.draw(|frame| app.draw(frame)).unwrap();
            let bar = App::main_areas(app.terminal_area)[2];
            let row = bar.y + (App::bar_height(app.terminal_area) - 1) / 2;
            let text = header_text(&app, width - 4);
            // Clicking each displayed grapheme must be safe, including wide emoji.
            let project_width = unicode_width::UnicodeWidthStr::width(app.project.as_str());
            for column in 0..project_width as u16 + 1 {
                app.mouse(event::MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column,
                    row,
                    modifiers: KeyModifiers::NONE,
                });
            }
            if width < SIDE_BY_SIDE_BREAKPOINT {
                let at = text.find("[Team]").unwrap();
                app.mouse(event::MouseEvent {
                    kind: MouseEventKind::Down(MouseButton::Left),
                    column: unicode_width::UnicodeWidthStr::width(&text[..at]) as u16 + 1,
                    row,
                    modifiers: KeyModifiers::NONE,
                });
                assert_eq!(app.focus, Focus::Graph);
                app.focus = Focus::Chat;
            }
            app.mouse(event::MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: width - 2,
                row,
                modifiers: KeyModifiers::NONE,
            });
            assert_eq!(app.help, Some(Help::Chat));
            app.key(key_press(KeyCode::Esc));
        }
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
    #[test]
    fn copy_shortcut_queues_text_for_the_rendering_thread() {
        let mut app = test_app();
        let area = Rect::new(0, 0, 8, 1);
        let mut buffer = ratatui::buffer::Buffer::empty(area);
        buffer.set_string(0, 0, "safe", Style::default());
        app.text_selection
            .draw_scrolled(&mut buffer, Some(area), app.selection_context(), 0);
        for (kind, column) in [
            (MouseEventKind::Down(MouseButton::Left), 0),
            (MouseEventKind::Drag(MouseButton::Left), 3),
        ] {
            app.text_selection.mouse(event::MouseEvent {
                kind,
                column,
                row: 0,
                modifiers: KeyModifiers::NONE,
            });
        }
        assert!(app.key(key_ctrl('y')));
        let text = app
            .pending_clipboard
            .take()
            .expect("key queues text, never writes stdout");
        assert_eq!(text, "safe");
        let mut output = Vec::new();
        assert_eq!(
            clipboard::copy_text(&mut output, &text).unwrap(),
            clipboard::CopyMethod::Osc52
        );
        assert_eq!(output, b"\x1b]52;c;c2FmZQ==\x07");
        assert!(app.pending_clipboard.is_none());
    }
}
