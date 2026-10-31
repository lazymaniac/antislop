use super::super::super::*;
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
}

impl App {
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
        self.restore_details(&key);
        self.select_details_tab();
        self.fetch_task();
    }

    fn restore_details(&mut self, key: &str) {
        if let Some(details) = &mut self.details {
            details.switch_key(key.to_owned());
        } else {
            let mut details = Details::new(key.to_owned());
            details.conversation = self.conversation.clone();
            details.drafts = std::rc::Rc::clone(&self.agent_drafts);
            details.scroll_memory = self.details_scroll_memory.clone();
            details.query_memory = self.details_query_memory.clone();
            details.restore_current_tab();
            self.details = Some(details);
        }
    }

    fn select_details_tab(&mut self) {
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
    }
}

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
    );
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
    );
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
