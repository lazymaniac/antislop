use super::*;

impl Core {
    /// Stores one check the verifier reported and tells frontends, when it
    /// changed what they show.
    pub(in crate::lean) fn record_check(
        &self,
        conversation: &str,
        task: &Task,
        update: &tasks::CheckUpdate,
    ) {
        if self.store.assert_task_inputs(task).is_err() {
            return;
        }
        let state = if update.status.is_some() {
            "settled"
        } else {
            "checking"
        };
        let changed = self.store.set_check(
            &task.id,
            task.round,
            &update.id,
            state,
            update.status.as_deref(),
            &update.note,
        );
        if changed.unwrap_or(false) {
            self.publish_check(conversation, task, update, state);
        }
    }

    fn publish_check(
        &self,
        conversation: &str,
        task: &Task,
        update: &tasks::CheckUpdate,
        state: &str,
    ) {
        let _ = self.store.append_event(
            conversation,
            None,
            Some(&task.plan_id),
            "check",
            &json!({"id": task.spec["id"], "task": task.id, "round": task.round,
                    "check": update.id, "state": state, "status": update.status}),
        );
    }

    /// The task's state for frontends; `reason` says why it stopped.
    /// `feedback` is the card's bounded result for the announced round:
    /// `null` when that round has produced none yet.
    pub(in crate::lean) fn announce_task(
        &self,
        conversation: &str,
        task: &Task,
        reason: Option<&str>,
    ) {
        let scope_hash = task_scope_hash(task);
        // The event's reason is the card's bounded line; the whole reason
        // is already in the task's report or in the conversation, so a cut
        // here never loses evidence the user or the orchestrator can read.
        let reason = reason.map(|reason| tasks::bound(reason, tasks::DETAIL_LIMIT));
        let _ = self.store.append_event(
            conversation,
            None,
            Some(&task.plan_id),
            "task",
            &json!({"id": task.spec["id"], "task": task.id, "title": task.spec["title"],
                    "status": task.status, "round": task.round, "reason": reason,
                    "scope_hash": scope_hash, "feedback": task.feedback}),
        );
    }

    pub(in crate::lean) fn set_status(
        &self,
        conversation: &str,
        task: &mut Task,
        status: &str,
    ) -> Result<()> {
        self.stop(conversation, task, status, None)
    }

    /// Sets a status the task stops in, saying why (design §8.4).
    pub(in crate::lean) fn stop(
        &self,
        conversation: &str,
        task: &mut Task,
        status: &str,
        reason: Option<&str>,
    ) -> Result<()> {
        task.status = status.into();
        self.store.update_task(task)?;
        self.announce_task(conversation, task, reason);
        Ok(())
    }

    /// Dynamically schedules and runs every task whose dependencies are approved
    /// and whose write scope does not conflict with currently running writers.
    /// Tasks start immediately as soon as their dependencies finish rather than
    /// waiting for unrelated slow tasks in the same wave.
    pub(in crate::lean) fn run_plan(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        cancel: &AtomicBool,
    ) {
        std::thread::scope(|scope| {
            let (done_tx, done_rx) = std::sync::mpsc::channel::<String>();
            let mut in_flight_ids: std::collections::HashSet<String> =
                std::collections::HashSet::new();
            let mut in_flight_writers: HashMap<String, Vec<String>> = HashMap::new();

            loop {
                let graph_guard = self.graph_updates.lock().expect("graph update mutex");
                let Ok(mut tasks) = self.store.plan(plan_id) else {
                    break;
                };
                let state: HashMap<String, String> = tasks
                    .iter()
                    .map(|task| {
                        (
                            task.spec["id"].as_str().unwrap_or_default().to_owned(),
                            task.status.clone(),
                        )
                    })
                    .collect();
                let mut settled = false;
                for task in &mut tasks {
                    let spec_id = task.spec["id"].as_str().unwrap_or_default().to_owned();
                    if (task.status != status::QUEUED && task.status != status::SKIPPED)
                        || in_flight_ids.contains(&spec_id)
                    {
                        continue;
                    }
                    if task.status == status::SKIPPED {
                        let _ = self.set_status(conversation, task, status::QUEUED);
                        settled = true;
                        continue;
                    }
                    let depends: Vec<String> =
                        serde_json::from_value(task.spec["depends_on"].clone()).unwrap_or_default();
                    if cancel.load(Ordering::SeqCst) {
                        continue;
                    } else if depends.iter().all(|dependency| {
                        state.get(dependency).map(String::as_str) == Some(status::APPROVED)
                    }) {
                        if task.status == status::SKIPPED {
                            let _ = self.set_status(conversation, task, status::QUEUED);
                            settled = true;
                            continue;
                        }
                        // The plan's limit on tasks running at once; the rest
                        // stay queued until one finishes.
                        let config = self.config_snapshot();
                        if in_flight_ids.len() >= config.policy.max_parallel {
                            continue;
                        }
                        let writes = task.spec["writes"].as_bool().unwrap_or(false);
                        let task_scope: Vec<String> =
                            serde_json::from_value(task.spec["scope"].clone()).unwrap_or_default();
                        // In place, writers take turns in the user's directory
                        // whatever their scopes; the others wait queued.
                        let one_at_a_time = config.workspace.mode == WorkspaceMode::InPlace;
                        let write_conflict = writes
                            && (one_at_a_time && !in_flight_writers.is_empty()
                                || in_flight_writers.values().any(|active_scope| {
                                    tasks::scopes_overlap(&task_scope, active_scope)
                                }));
                        if !write_conflict {
                            in_flight_ids.insert(spec_id.clone());
                            if writes {
                                in_flight_writers.insert(spec_id.clone(), task_scope);
                            }
                            let mut task_clone = task.clone();
                            let tx = done_tx.clone();
                            let task_cancel =
                                Arc::new(AtomicBool::new(cancel.load(Ordering::SeqCst)));
                            self.task_cancels.lock().expect("task cancel mutex").insert(
                                task.id.clone(),
                                (conversation.into(), task_cancel.clone()),
                            );
                            scope.spawn(move || {
                                if let Err(error) =
                                    self.run_task(conversation, &mut task_clone, &task_cancel)
                                {
                                    let report = task_clone.report.take().unwrap_or_default();
                                    task_clone.report = Some(
                                        format!("{report}\n\n[runtime] {error}")
                                            .trim_start()
                                            .to_owned(),
                                    );
                                    let _ = self.stop(
                                        conversation,
                                        &mut task_clone,
                                        status::FAILED,
                                        Some(&error.to_string()),
                                    );
                                }
                                let _ =
                                    self.answer_waiting(conversation, &task_clone.plan_id, cancel);
                                let restart = {
                                    let mut active =
                                        self.task_cancels.lock().expect("task cancel mutex");
                                    active.remove(&task_clone.id);
                                    self.task_restarts
                                        .lock()
                                        .expect("task restart mutex")
                                        .remove(&task_clone.id)
                                };
                                if let Some(mut request) = restart {
                                    let result = self
                                        .task_freshness_token(
                                            conversation,
                                            &request.plan,
                                            &request.task,
                                        )
                                        .and_then(|token| {
                                            request.freshness_token = token;
                                            self.recover_task(request)
                                        });
                                    if let Err(error) = result {
                                        self.notice(
                                            conversation,
                                            &format!("agent restart failed: {error}"),
                                        );
                                    }
                                }
                                let _ = tx.send(spec_id);
                            });
                        }
                    }
                }
                drop(graph_guard);
                if settled {
                    continue;
                }
                if in_flight_ids.is_empty() {
                    break;
                }
                let Ok(finished_id) = done_rx.recv() else {
                    break;
                };
                in_flight_ids.remove(&finished_id);
                in_flight_writers.remove(&finished_id);
                while let Ok(more_id) = done_rx.try_recv() {
                    in_flight_ids.remove(&more_id);
                    in_flight_writers.remove(&more_id);
                }
            }
        });
    }
}
