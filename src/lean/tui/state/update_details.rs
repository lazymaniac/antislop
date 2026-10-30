use super::super::*;

struct TaskSnapshotReply {
    conversation: String,
    plan: Option<String>,
    generation: u64,
    request_id: u64,
    key: String,
    round: u64,
    task: Value,
    documents: std::collections::HashMap<String, document::Document>,
}

impl App {
    pub(super) fn apply_task_failed(&mut self, update: Update) {
        let Update::TaskFailed {
            conversation,
            generation,
            plan,
            request_id,
            key,
            error,
        } = update
        else {
            unreachable!("update routed to wrong handler");
        };
        if conversation == self.conversation {
            self.fail_active_task(generation, &plan, request_id, &key, &error);
        } else {
            self.fail_background_task(&conversation, generation, &plan, request_id, &key, &error);
        }
    }

    fn fail_active_task(
        &mut self,
        generation: u64,
        plan: &str,
        request_id: u64,
        key: &str,
        error: &str,
    ) {
        if self.view.plan.as_deref().unwrap_or_default() != plan
            || request_id != self.task_request_id
        {
            return;
        }
        let Some(details) = self.details.as_mut().filter(|details| {
            details.key == key
                && details.fetched_generation == Some(generation)
                && details.fetched_plan.as_deref().unwrap_or_default() == plan
        }) else {
            return;
        };
        if details.fetch_failed(error) {
            self.notice = Some(format!("✗ {error}"));
        }
    }

    fn fail_background_task(
        &mut self,
        conversation: &str,
        generation: u64,
        plan: &str,
        request_id: u64,
        key: &str,
        error: &str,
    ) {
        let Some(state) = self
            .conversation_states
            .get_mut(conversation)
            .filter(|state| state.task_request_id == request_id)
        else {
            return;
        };
        if let Some(details) = state.details.as_mut().filter(|details| {
            details.key == key
                && details.fetched_generation == Some(generation)
                && details.fetched_plan.as_deref().unwrap_or_default() == plan
        }) {
            details.fetch_failed(error);
        }
    }

    pub(super) fn apply_task(&mut self, update: Update) {
        let Update::Task {
            conversation,
            plan,
            generation,
            request_id,
            key,
            round,
            task,
            documents,
        } = update
        else {
            unreachable!("update routed to wrong handler");
        };
        self.apply_task_snapshot(TaskSnapshotReply {
            conversation,
            plan,
            generation,
            request_id,
            key,
            round,
            task,
            documents,
        });
    }

    fn apply_task_snapshot(&mut self, reply: TaskSnapshotReply) {
        self.clear_task_fetch(&reply);
        if self.task_round_moved(&reply) {
            return;
        }
        if reply.conversation == self.conversation {
            if self.task_reply_matches_current_round(&reply) {
                self.load_active_task(reply);
            }
        } else {
            self.load_background_task(reply);
        }
        self.refetch_stale_details();
    }

    fn clear_task_fetch(&mut self, reply: &TaskSnapshotReply) {
        if reply.conversation == self.conversation && reply.request_id == self.task_request_id {
            if let Some(details) = self
                .details
                .as_mut()
                .filter(|details| details.key == reply.key)
            {
                details.fetch_in_flight = false;
            }
        } else if let Some(state) = self
            .conversation_states
            .get_mut(&reply.conversation)
            .filter(|state| state.task_request_id == reply.request_id)
            && let Some(details) = state
                .details
                .as_mut()
                .filter(|details| details.key == reply.key)
        {
            details.fetch_in_flight = false;
        }
    }

    fn task_round_moved(&mut self, reply: &TaskSnapshotReply) -> bool {
        let Some(actual) = reply.task["round"]
            .as_u64()
            .filter(|actual| *actual != reply.round)
        else {
            return false;
        };
        if reply.conversation == self.conversation
            && reply.request_id == self.task_request_id
            && self
                .task_row_round(&reply.key)
                .is_some_and(|current| current != reply.round && actual == current)
        {
            if let Some(details) = self.details.as_mut() {
                details.fetched_round = None;
            }
            self.refetch_stale_details();
        }
        true
    }

    fn task_reply_matches_current_round(&self, reply: &TaskSnapshotReply) -> bool {
        match self.task_row_round(&reply.key) {
            Some(current) => {
                self.details
                    .as_ref()
                    .and_then(|details| details.selected_round)
                    .unwrap_or(current)
                    == reply.round
            }
            None => true,
        }
    }

    fn load_active_task(&mut self, reply: TaskSnapshotReply) {
        if reply.request_id != self.task_request_id
            || self.view.plan.as_ref() != reply.plan.as_ref()
        {
            return;
        }
        let Some(details) = self.details.as_mut().filter(|details| {
            details.key == reply.key
                && details
                    .fetched_generation
                    .is_none_or(|issued| issued == reply.generation)
                && details
                    .fetched_plan
                    .as_ref()
                    .is_none_or(|issued| Some(issued) == reply.plan.as_ref())
        }) else {
            return;
        };
        details.load_task(reply.task);
        details.documents = reply.documents;
    }

    fn load_background_task(&mut self, reply: TaskSnapshotReply) {
        let Some(state) = self
            .conversation_states
            .get_mut(&reply.conversation)
            .filter(|state| state.task_request_id == reply.request_id)
        else {
            return;
        };
        let Some(details) = state.details.as_mut().filter(|details| {
            details.key == reply.key
                && details.fetched_round == Some(reply.round)
                && details
                    .fetched_generation
                    .is_none_or(|issued| issued == reply.generation)
                && details
                    .fetched_plan
                    .as_ref()
                    .is_none_or(|issued| Some(issued) == reply.plan.as_ref())
        }) else {
            return;
        };
        details.load_task(reply.task);
        details.documents = reply.documents;
    }

    pub(super) fn apply_diff(&mut self, update: Update) {
        match update {
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
            _ => unreachable!("update routed to wrong handler"),
        }
    }
}
