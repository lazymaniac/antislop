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
        details.load_task_documents(reply.documents);
        self.schedule_output_document_fetch();
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
        details.load_task_documents(reply.documents);
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
                {
                    let mut cached = DiffView::new(task.clone());
                    cached.load(reply.clone());
                    cached.select_file(self.workbench_file_index);
                    self.workbench_diffs.insert(task.clone(), cached);
                    if let Some(diff) = self.diff.as_mut().filter(|diff| diff.task == task) {
                        diff.load(reply);
                        diff.select_file(self.workbench_file_index);
                    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::net::UnixListener;
    use std::sync::mpsc;

    #[test]
    fn task_refresh_cancels_removed_result_before_the_next_chunk() {
        let state = tempfile::tempdir().unwrap();
        let listener = UnixListener::bind(crate::lean::daemon::socket(state.path())).unwrap();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream);
            let first = crate::transport::read::<serde_json::Value>(&mut reader)
                .unwrap()
                .unwrap();
            assert_eq!(first["request"]["id"], "A");
            assert_eq!(first["request"]["offset"], 0);
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            crate::transport::write(
                reader.get_mut(),
                &json!({"data":{"text":"partial A","next_offset":9,"done":false},"error":null}),
            )
            .unwrap();
            assert!(
                crate::transport::read::<serde_json::Value>(&mut reader)
                    .unwrap()
                    .is_none()
            );
            drop(reader);

            let (stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream);
            let replacement = crate::transport::read::<serde_json::Value>(&mut reader)
                .unwrap()
                .unwrap();
            assert_eq!(replacement["request"]["id"], "B");
            assert_eq!(replacement["request"]["offset"], 0);
            crate::transport::write(
                reader.get_mut(),
                &json!({"data":{"text":"complete B","next_offset":9,"done":true},"error":null}),
            )
            .unwrap();
        });

        let client = Client::at(state.path());
        let (mut app, updates) = crate::lean::tui::App::test_app_with_client(client);
        let mut details = crate::lean::tui::details::Details::new(ORCHESTRATOR.into());
        details.tab = crate::lean::tui::details::RESULT;
        details.load_task(
            json!({"_result_documents":[{"id":"A","path":"a.md","source_commit":"12345678"}]}),
        );
        app.details = Some(details);
        app.schedule_output_document_fetch();
        started_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();

        app.load_active_task(TaskSnapshotReply {
            conversation: app.conversation.clone(),
            plan: app.view.plan.clone(),
            generation: app.generation,
            request_id: app.task_request_id,
            key: ORCHESTRATOR.into(),
            round: 1,
            task: json!({"round":1,"_result_documents":[{"id":"B","path":"b.md","source_commit":"87654321"}]}),
            documents: Default::default(),
        });
        assert_eq!(
            app.details
                .as_ref()
                .unwrap()
                .selected_result_document
                .as_deref(),
            Some("result_document:B")
        );
        release_tx.send(()).unwrap();
        let update = updates
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(
            matches!(update, Update::ResultDocument { id, result: Ok(_), .. } if id == "result_document:B")
        );
        server.join().unwrap();
        assert_eq!(
            std::fs::read_dir(state.path().join("inspection-cache"))
                .unwrap()
                .count(),
            1
        );
    }
}
