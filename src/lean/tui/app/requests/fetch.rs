use super::super::super::*;
use std::sync::{Condvar, Mutex};

#[derive(Clone, Debug, PartialEq, Eq)]
struct ResultFetchIdentity {
    conversation: String,
    generation: u64,
    key: String,
    id: String,
}

#[derive(Clone)]
struct ResultFetchRequest {
    identity: ResultFetchIdentity,
    selection: u64,
    metadata: serde_json::Value,
    client: Client,
    updates: UpdateSender,
}

#[derive(Default)]
struct ResultFetchState {
    desired: Option<ResultFetchRequest>,
    revision: u64,
    closed: bool,
    running: Option<ResultFetchIdentity>,
    completed: Option<ResultFetchIdentity>,
}

#[derive(Default)]
struct ResultFetchShared {
    state: Mutex<ResultFetchState>,
    changed: Condvar,
}

/// One worker per TUI app serializes document transfers and coalesces rapid
/// selection changes to the latest requested artifact.
#[derive(Default)]
pub(in crate::lean::tui) struct ResultFetchCoordinator {
    shared: Arc<ResultFetchShared>,
}

impl ResultFetchCoordinator {
    pub(in crate::lean::tui) fn new() -> Self {
        let shared = Arc::new(ResultFetchShared::default());
        let worker = Arc::clone(&shared);
        std::thread::spawn(move || result_fetch_worker(worker));
        Self { shared }
    }

    fn select(&self, request: Option<ResultFetchRequest>) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let previous_identity = state
            .desired
            .as_ref()
            .map(|request| request.identity.clone());
        let previous_selection = state.desired.as_ref().map(|request| request.selection);
        let next_identity = request.as_ref().map(|request| &request.identity);
        let selected_completed =
            next_identity.is_some_and(|identity| state.completed.as_ref() == Some(identity));
        if !selected_completed
            && (previous_identity.as_ref() != next_identity
                || request.as_ref().is_some_and(|next| {
                    previous_selection.is_some_and(|selection| {
                        selection != next.selection
                            && state.running.as_ref() != Some(&next.identity)
                    })
                }))
        {
            state.revision = state.revision.wrapping_add(1);
        }
        state.desired = request;
        self.shared.changed.notify_one();
    }

    pub(in crate::lean::tui) fn cancel_document_fetch(&self) {
        self.select(None);
    }
}

impl Drop for ResultFetchCoordinator {
    fn drop(&mut self) {
        let mut state = self
            .shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.closed = true;
        state.desired = None;
        self.shared.changed.notify_one();
    }
}

fn result_fetch_worker(shared: Arc<ResultFetchShared>) {
    let mut observed_revision = 0;
    loop {
        let (request, revision) = {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            while !state.closed {
                let selected_completed = state
                    .desired
                    .as_ref()
                    .is_some_and(|desired| state.completed.as_ref() == Some(&desired.identity));
                if selected_completed {
                    observed_revision = state.revision;
                }
                if state.desired.is_some()
                    && state.revision != observed_revision
                    && !selected_completed
                {
                    break;
                }
                state = shared
                    .changed
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
            if state.closed {
                return;
            }
            let Some(request) = state.desired.clone() else {
                continue;
            };
            observed_revision = state.revision;
            state.running = Some(request.identity.clone());
            (request, state.revision)
        };
        let identity = request.identity.clone();
        let is_current = || {
            shared
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .desired
                .as_ref()
                .is_some_and(|desired| desired.identity == identity)
        };
        let result = inspection::result_document(
            &request.client,
            &request.identity.conversation,
            &request.metadata,
            is_current,
        )
        .map_err(|error| error.to_string());
        let mut state = shared
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state.running.as_ref() == Some(&identity) {
            state.running = None;
        }
        let current = state
            .desired
            .as_ref()
            .filter(|desired| desired.identity == identity)
            .map(|desired| (desired.selection, desired.updates.clone()));
        if let Some((selection, updates)) = current {
            observed_revision = state.revision;
            let update = match result {
                Ok(Some(document)) => {
                    state.completed = Some(identity.clone());
                    Some(Update::ResultDocument {
                        conversation: identity.conversation.clone(),
                        generation: identity.generation,
                        key: identity.key.clone(),
                        selection,
                        id: identity.id.clone(),
                        result: Ok(document),
                    })
                }
                Ok(None) => None,
                Err(error) => Some(Update::ResultDocument {
                    conversation: identity.conversation.clone(),
                    generation: identity.generation,
                    key: identity.key.clone(),
                    selection,
                    id: identity.id.clone(),
                    result: Err(error),
                }),
            };
            if let Some(update) = update {
                drop(state);
                let _ = updates.send(update);
                continue;
            }
        }
        // A changed request is picked up immediately; an unchanged request
        // waits for another selection or a retry after an error.
        if state.revision == revision {
            observed_revision = revision;
        }
    }
}
use crate::lean::tui::UpdateSender;

struct TaskFetch {
    conversation: String,
    plan: String,
    generation: u64,
    request_id: u64,
    key: String,
    round: u64,
    history_id: String,
    selected_round: Option<u64>,
    checks_only: bool,
    dialogue: bool,
    client: Client,
    updates: UpdateSender,
}

struct RunFetch {
    client: Client,
    updates: UpdateSender,
    conversation: String,
    plan: Option<String>,
    generation: u64,
    request_id: u64,
    key: String,
    selected: Option<u64>,
    dialogue: bool,
}

impl App {
    pub(in crate::lean::tui) fn schedule_output_document_fetch(&mut self) {
        let Some(details) = self
            .details
            .as_mut()
            .filter(|details| details.tab == details::RESULT)
        else {
            self.result_fetch.cancel_document_fetch();
            if let Some(details) = self.details.as_mut() {
                details.result_loading = None;
            }
            return;
        };
        let Some(id) = details
            .selected_result_document
            .clone()
            .filter(|id| id.starts_with("result_document:"))
        else {
            self.result_fetch.cancel_document_fetch();
            if let Some(details) = self.details.as_mut() {
                details.result_loading = None;
            }
            return;
        };
        if details.documents.contains_key(&id) {
            self.result_fetch.cancel_document_fetch();
            return;
        }
        if details
            .result_loading
            .as_ref()
            .is_some_and(|(key, selection, generation)| {
                key == &id
                    && *selection == details.result_selection_generation
                    && *generation == self.generation
            })
            || details
                .result_error
                .as_ref()
                .is_some_and(|(key, _)| key == &id)
        {
            return;
        }
        let Some(metadata) = details.result_metadata.get(&id).cloned() else {
            self.result_fetch.cancel_document_fetch();
            return;
        };
        let selection = details.result_selection_generation;
        details.result_loading = Some((id.clone(), selection, self.generation));
        let (key, conversation, generation) = (
            details.key.clone(),
            self.conversation.clone(),
            self.generation,
        );
        self.result_fetch.select(Some(ResultFetchRequest {
            identity: ResultFetchIdentity {
                conversation,
                generation,
                key,
                id,
            },
            selection,
            metadata,
            client: self.client.clone(),
            updates: self.updates.clone(),
        }));
    }

    /// Fetches the stored task behind the open details. The request carries
    /// the task id, the plan and the task's round, and replies are matched
    /// against all of them plus this conversation's generation and the
    /// request id, so a late answer can never paste one round's data into
    /// another round's or another plan's details.
    pub(in crate::lean::tui) fn fetch_task(&mut self) {
        let Some(fetch) = self.prepare_task_fetch() else {
            self.fetch_run();
            return;
        };
        std::thread::spawn(move || run_task_fetch(fetch));
    }

    fn prepare_task_fetch(&mut self) -> Option<TaskFetch> {
        let (plan, id, key, selected_round, round) = self.task_fetch_identity()?;
        self.task_request_id += 1;
        let request_id = self.task_request_id;
        let latest = self.task_row_round(&key).unwrap_or(0);
        self.mark_task_fetch(&id, &plan, round, latest);
        Some(TaskFetch {
            conversation: self.conversation.clone(),
            plan,
            generation: self.generation,
            request_id,
            key,
            round,
            history_id: self
                .details
                .as_ref()
                .and_then(|d| d.task_uuid.clone())
                .unwrap_or_else(|| id.clone()),
            selected_round,
            dialogue: self.navigation.presentation().tab == WorkbenchTab::Conversation,
            checks_only: self
                .details
                .as_ref()
                .is_some_and(|details| details.tab == CHECKS),
            client: self.client.clone(),
            updates: self.updates.clone(),
        })
    }

    fn task_fetch_identity(&self) -> Option<(String, String, String, Option<u64>, u64)> {
        let plan = self.view.plan.clone()?;
        let details = self.details.as_ref()?;
        let id = details.task_id()?.to_owned();
        let key = details.key.clone();
        let selected = details.selected_round;
        let round = selected.or_else(|| self.task_row_round(&key)).unwrap_or(0);
        Some((plan, id, key, selected, round))
    }

    fn mark_task_fetch(&mut self, id: &str, plan: &str, round: u64, latest: u64) {
        let checks = self.view.task_check_events.get(id).copied().unwrap_or(0);
        if let Some(details) = self
            .details
            .as_mut()
            .filter(|details| details.task_id() == Some(id))
        {
            details.fetch_in_flight = true;
            details.latest_observed = latest;
            details.fetched_events = self.view.after;
            details.fetched_round = Some(round);
            details.fetched_generation = Some(self.generation);
            details.fetched_plan = Some(plan.to_owned());
            details.fetched_checks = checks;
        }
    }

    pub(in crate::lean::tui) fn fetch_run(&mut self) {
        let Some(fetch) = self.prepare_run_fetch() else {
            return;
        };
        std::thread::spawn(move || run_run_fetch(fetch));
    }

    fn prepare_run_fetch(&mut self) -> Option<RunFetch> {
        let details = self
            .details
            .as_ref()
            .filter(|details| details.task_id().is_none() && details.key != "conversation")?;
        self.task_request_id += 1;
        let request_id = self.task_request_id;
        let key = details.key.clone();
        let selected = details.selected_round;
        let plan = self.view.plan.clone();
        let fetch = RunFetch {
            dialogue: self.navigation.presentation().tab == WorkbenchTab::Conversation,
            client: self.client.clone(),
            updates: self.updates.clone(),
            conversation: self.conversation.clone(),
            plan,
            generation: self.generation,
            request_id,
            key,
            selected,
        };
        let latest = self.latest_round_for_key(&fetch.key);
        if let Some(details) = self.details.as_mut() {
            mark_run_fetch(details, &fetch, latest, self.view.after);
        }
        Some(fetch)
    }

    fn latest_round_for_key(&self, key: &str) -> u64 {
        self.view.agents.get(key).map_or(0, |agent| {
            if agent.running {
                u64::from(agent.turns)
            } else {
                u64::from(agent.turns.saturating_sub(1))
            }
        })
    }

    pub fn open_details(&mut self, key: String) {
        self.default_diff_request = None;
        if key != "conversation" {
            let work = crate::lean::tui::work_units::WorkUnits::new(&self.view);
            if let Some(unit) = work.for_stage(&key) {
                self.navigation.remembered_stages.insert(
                    unit.id.clone(),
                    crate::lean::tui::work_units::StageId(key.clone()),
                );
            }
        }
        let target = if key == "conversation" {
            WorkbenchTarget::Session
        } else {
            WorkbenchTarget::Agent(key.clone())
        };
        let target_initialized = self.navigation.presentations.contains_key(&target);
        self.remember_workbench();
        let previous_focus = self.navigation.focus;
        let previous_round = self
            .details
            .as_ref()
            .and_then(|details| details.selected_round);
        if self.details.is_some() && self.navigation.screen == Screen::Workbench {
            self.navigation.update_presentation(|presentation| {
                presentation.focus = previous_focus;
                presentation.round = previous_round;
            });
        }
        self.navigation.screen = Screen::Workbench;
        self.navigation.target = target;
        let writer = key.starts_with("worker:")
            && self
                .view
                .tasks
                .iter()
                .find(|task| key.split_once(':').is_some_and(|(_, id)| id == task.id))
                .is_some_and(|task| task.change_kind.has_delivery());
        let changes = writer
            && self
                .diff_states
                .get(&(self.conversation.clone(), key.clone()))
                .is_some_and(DiffView::has_changes);
        if !target_initialized {
            let terminal_only = key != "conversation" && !changes;
            let tab = super::super::super::workbench::default_tab(&key, changes);
            self.navigation.update_presentation(|presentation| {
                presentation.tab = tab;
                presentation.terminal_only_default = terminal_only;
                if terminal_only {
                    presentation.split = false;
                    presentation.focus = NavigationFocus::WorkbenchTerminal;
                } else {
                    presentation.focus = NavigationFocus::WorkbenchContent;
                }
            });
        }
        // Opening a target closes its composer; restore a navigation pane instead
        // of routing subsequent keys into an editor that is no longer visible.
        self.navigation.update_presentation(|presentation| {
            if !matches!(
                presentation.focus,
                NavigationFocus::WorkbenchList
                    | NavigationFocus::WorkbenchContent
                    | NavigationFocus::WorkbenchTerminal
            ) {
                presentation.focus = NavigationFocus::WorkbenchContent;
            }
        });
        self.navigation.focus = self.navigation.presentation().focus;
        self.result_fetch.cancel_document_fetch();
        self.restore_details(&key);
        self.resolve_details_task();
        if self.graph_expanded.is_none() && self.navigation.presentation().tab == WorkbenchTab::Diff
        {
            self.load_diff(&key);
        } else {
            self.diff = None;
        }
        if self.navigation.presentation().tab == WorkbenchTab::Results {
            self.schedule_output_document_fetch();
        }
        if !target_initialized && writer && !changes && self.view.plan.is_some() {
            // Probe off-thread, leaving the diff closed while the task has no
            // known file changes. A late reply must not replace a chosen tab.
            self.load_diff(&key);
            self.default_diff_request =
                Some((self.conversation.clone(), key.clone(), self.diff_request_id));
            self.diff = None;
        }
        self.fetch_task();
    }

    pub(in crate::lean::tui) fn fetch_workbench_sessions(&mut self) {
        if let Some((conversation, generation, _)) = &self.workbench_sessions_inflight {
            self.workbench_sessions_pending |=
                conversation != &self.conversation || *generation != self.generation;
            return;
        }
        let Some(request_id) = self.workbench_sessions_request_id.checked_add(1) else {
            self.notice = Some("Could not load CLI session owners: request id exhausted".into());
            return;
        };
        self.workbench_sessions_request_id = request_id;
        self.workbench_sessions_pending = false;
        self.workbench_sessions_fetch_after = self.view.after;
        self.workbench_sessions_inflight =
            Some((self.conversation.clone(), self.generation, request_id));
        let client = self.client.clone();
        let updates = self.updates.clone();
        let conversation = self.conversation.clone();
        let generation = self.generation;
        std::thread::spawn(move || {
            let result = client
                .call(&crate::lean::daemon::Request::Session(
                    crate::lean::daemon::SessionRequest::Sessions {
                        conversation: conversation.clone(),
                    },
                ))
                .and_then(|value| serde_json::from_value(value).map_err(crate::Error::from));
            let _ = updates.send(Update::WorkbenchSessions {
                conversation,
                generation,
                request_id,
                result: result.map_err(|error| error.to_string()),
            });
        });
    }

    fn restore_details(&mut self, key: &str) {
        self.remember_workbench();
        if let Some(saved) = self
            .detail_states
            .remove(&(self.conversation.clone(), key.to_owned()))
        {
            self.details = Some(saved);
            if let Some(details) = &mut self.details {
                details.fetch_in_flight = false;
                details.editing = false;
                details.native_focus = false;
                details.result_loading = None;
                details.prepare_workbench_details(None, self.navigation.presentation().tab);
            }
            return;
        }
        let presentation = self.navigation.presentation();
        if let Some(details) = &mut self.details {
            details.switch_key(key.to_owned());
            details.selected_round = presentation.round;
            details.prepare_workbench_details(None, presentation.tab);
        } else {
            let mut details = Details::new(key.to_owned());
            details.conversation = self.conversation.clone();
            details.drafts = std::rc::Rc::clone(&self.agent_drafts);
            details.scroll_memory = self.details_scroll_memory.clone();
            details.query_memory = self.details_query_memory.clone();
            details.workbench_memory = [
                WorkbenchTab::Summary,
                WorkbenchTab::Conversation,
                WorkbenchTab::Results,
                WorkbenchTab::Checks,
                WorkbenchTab::Comments,
                WorkbenchTab::Diff,
            ]
            .into_iter()
            .filter_map(|tab| {
                // These reserved high slots are disjoint from legacy Details indices.
                let slot = usize::MAX - tab as usize;
                let scroll = self.details_scroll_memory.get(&(key.to_owned(), slot))?;
                let query = self.details_query_memory.get(&(key.to_owned(), slot))?;
                Some(((key.to_owned(), tab), (*scroll, query.clone())))
            })
            .collect();
            details.selected_round = presentation.round;
            details.restore_current_tab();
            details.prepare_workbench_details(None, presentation.tab);
            self.details = Some(details);
        }
    }

    fn resolve_details_task(&mut self) {
        if let Some(details) = &mut self.details {
            details.task_uuid = details
                .task_id()
                .and_then(|id| self.view.tasks.iter().find(|task| task.id == id))
                .and_then(|task| task.uuid.clone());
            if details.task_id().is_some() {
                details.prepare_workbench_details(None, self.navigation.presentation().tab);
            }
        }
    }
}

#[cfg(test)]
#[path = "fetch_tests.rs"]
mod result_fetch_tests;

fn mark_run_fetch(details: &mut Details, fetch: &RunFetch, latest: u64, events: u64) {
    details.fetch_in_flight = true;
    details.fetched_generation = Some(fetch.generation);
    details.fetched_plan = fetch.plan.clone();
    details.latest_observed = latest;
    details.fetched_events = events;
}

fn run_task_fetch(fetch: TaskFetch) {
    let result = inspection::task_snapshot(
        &fetch.client,
        &fetch.conversation,
        &fetch.plan,
        &fetch.history_id,
        fetch.selected_round,
        fetch.checks_only,
    )
    .and_then(|mut snapshot| {
        if fetch.dialogue {
            inspection::load_prompts(
                &fetch.client,
                &fetch.conversation,
                &fetch.key,
                &mut snapshot.task,
            )?;
        }
        Ok(snapshot)
    });
    let update = task_fetch_update(fetch, result);
    let _ = update.0.send(update.1);
}

fn task_fetch_update(
    fetch: TaskFetch,
    result: crate::Result<inspection::Snapshot>,
) -> (UpdateSender, Update) {
    let update = match result {
        Ok(task) => Update::Task {
            conversation: fetch.conversation,
            plan: Some(fetch.plan),
            generation: fetch.generation,
            request_id: fetch.request_id,
            key: fetch.key,
            round: fetch.round,
            task: task.task,
            documents: task.documents,
        },
        Err(error) => Update::TaskFailed {
            conversation: fetch.conversation,
            generation: fetch.generation,
            plan: fetch.plan,
            request_id: fetch.request_id,
            key: fetch.key,
            error: error.to_string(),
        },
    };
    (fetch.updates, update)
}

fn run_run_fetch(fetch: RunFetch) {
    let result = inspection::run_snapshot(
        &fetch.client,
        &fetch.conversation,
        &fetch.key,
        fetch.selected,
    )
    .and_then(|mut snapshot| {
        if fetch.dialogue {
            inspection::load_prompts(
                &fetch.client,
                &fetch.conversation,
                &fetch.key,
                &mut snapshot.task,
            )?;
        }
        Ok(snapshot)
    });
    let update = match result {
        Ok(task) => Update::Task {
            conversation: fetch.conversation,
            plan: fetch.plan,
            generation: fetch.generation,
            request_id: fetch.request_id,
            key: fetch.key,
            round: task.task["round"].as_u64().unwrap_or(0),
            task: task.task,
            documents: task.documents,
        },
        Err(error) => Update::TaskFailed {
            conversation: fetch.conversation,
            plan: fetch.plan.unwrap_or_default(),
            generation: fetch.generation,
            request_id: fetch.request_id,
            key: fetch.key,
            error: error.to_string(),
        },
    };
    let _ = fetch.updates.send(update);
}

impl App {
    pub(in crate::lean::tui) fn remember_workbench(&mut self) {
        if let Some(details) = &self.details {
            let mut saved = details.clone();
            let target = if details.key == "conversation" {
                WorkbenchTarget::Session
            } else {
                WorkbenchTarget::Agent(details.key.clone())
            };
            if let Some(presentation) = self.navigation.presentations.get(&target) {
                saved.workbench_memory.insert(
                    (details.key.clone(), presentation.tab),
                    (details.scroll, details.query.clone()),
                );
            }
            self.detail_states
                .insert((details.conversation.clone(), details.key.clone()), saved);
            if let Some(diff) = &self.diff {
                self.diff_states.insert(
                    (details.conversation.clone(), details.key.clone()),
                    diff.clone(),
                );
            }
        }
    }
}
