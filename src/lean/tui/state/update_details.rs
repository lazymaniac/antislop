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
        details.load_documents(reply.documents);
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
        details.load_documents(reply.documents);
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
                    if let Some((_, key, _)) = self
                        .default_diff_request
                        .as_ref()
                        .filter(|(conversation, _, request)| {
                            conversation == &self.conversation && *request == request_id
                        })
                        .cloned()
                    {
                        self.default_diff_request = None;
                        if let Err(error) = &reply {
                            self.notice = Some(format!("Could not load task changes: {error}"));
                        }
                        let mut diff = DiffView::new(task);
                        diff.load(reply);
                        if diff.has_changes()
                            && self.navigation.target == WorkbenchTarget::Agent(key.clone())
                            && self.navigation.screen == Screen::Workbench
                            && self.navigation.presentation().terminal_only_default
                            && !self.navigation.presentation().native_input
                        {
                            self.navigation.update_presentation(|presentation| {
                                presentation.terminal_only_default = false;
                                presentation.tab = WorkbenchTab::Diff;
                                presentation.split = true;
                                presentation.focus = NavigationFocus::WorkbenchContent;
                            });
                            self.navigation.focus = NavigationFocus::WorkbenchContent;
                            if let Some(details) = &mut self.details {
                                details.prepare_workbench_details(
                                    Some(WorkbenchTab::Summary),
                                    WorkbenchTab::Diff,
                                );
                            }
                            self.diff = Some(diff.clone());
                        }
                        self.diff_states
                            .insert((self.conversation.clone(), key), diff);
                    } else if let Some(diff) = self.diff.as_mut().filter(|diff| diff.task == task) {
                        diff.load(reply);
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
    fn default_diff_opens_only_for_real_changes_and_never_overrides_a_chosen_tab() {
        for (files, manual) in [(false, false), (true, false), (true, true)] {
            let mut app = App::test_app();
            app.view.plan = Some("p1".into());
            app.view.tasks.push(view::TaskRow {
                id: "t1".into(),
                change_kind: crate::lean::tasks::ChangeKind::Code,
                ..Default::default()
            });
            app.open_details("worker:t1".into());
            assert!(app.default_diff_request.is_some());
            assert!(app.diff.is_none());
            let request_id = app.diff_request_id;
            if manual {
                app.workbench_select_tab(WorkbenchTab::Results);
            }
            let file = json!({"path":"a.rs","old_path":null,"status":"modified","binary":false,"added":1,"removed":0,"lines":[]});
            app.apply_diff(Update::Diff {
                conversation: app.conversation.clone(),
                plan: app.view.plan.clone(),
                generation: app.generation,
                request_id,
                task: "t1".into(),
                reply: Ok(json!({"files":if files {vec![file]} else {vec![]}})),
            });
            assert_eq!(app.diff.is_some(), files && !manual);
            assert_eq!(
                app.navigation.presentation().tab,
                if manual {
                    WorkbenchTab::Results
                } else if files {
                    WorkbenchTab::Diff
                } else {
                    WorkbenchTab::Summary
                }
            );
            if !files {
                assert!(app.navigation.presentation().terminal_only_default);
            }
        }
        for key in [
            "worker:t1",
            "reviewer:t1",
            ORCHESTRATOR,
            crate::lean::PLANNER,
        ] {
            let mut app = App::test_app();
            app.view.plan = Some("p1".into());
            app.view.tasks.push(view::TaskRow {
                id: "t1".into(),
                ..Default::default()
            });
            app.open_details(key.into());
            assert!(app.default_diff_request.is_none());
            assert!(app.diff.is_none());
            assert!(app.navigation.presentation().terminal_only_default);
        }
    }

    #[test]
    fn diff_refresh_preserves_file_selection_and_ignores_stale_replies() {
        let mut app = App::test_app();
        app.view.plan = Some("p1".into());
        app.diff = Some(DiffView::new("t1".into()));
        app.diff_request_id = 7;
        let files = ["src/first.rs", "src/second.rs"].map(|path| {
            json!({
                "path": path, "old_path": null, "status": "modified", "binary": false,
                "added": 1, "removed": 0,
                "lines": [{"kind": "add", "old": null, "new": 1, "text": "new line"}]
            })
        });
        let reply = json!({"files": files});
        let update = |plan: &str, request_id, task: &str, reply| Update::Diff {
            conversation: "test-conv".into(),
            plan: Some(plan.into()),
            generation: 0,
            request_id,
            task: task.into(),
            reply: Ok(reply),
        };
        app.apply_update(update("p1", 7, "t1", reply.clone()));
        let diff = app.diff.as_mut().unwrap();
        diff.key(KeyCode::Char(']'));
        assert_eq!(diff.target().as_deref(), Some("diff:t1:src/second.rs:1"));

        app.apply_update(update("p1", 7, "t1", reply.clone()));
        for (plan, request_id, task) in [("p0", 7, "t1"), ("p1", 6, "t1"), ("p1", 7, "t2")] {
            app.apply_update(update(plan, request_id, task, json!({"files": []})));
        }
        assert_eq!(
            app.diff.as_ref().unwrap().target().as_deref(),
            Some("diff:t1:src/second.rs:1")
        );

        let mut reduced = reply;
        reduced["files"].as_array_mut().unwrap().pop();
        app.apply_update(update("p1", 7, "t1", reduced));
        assert_eq!(
            app.diff.as_ref().unwrap().target().as_deref(),
            Some("diff:t1:src/first.rs:1")
        );
    }

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
