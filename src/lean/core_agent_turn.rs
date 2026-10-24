use super::*;

impl Core {
    pub(in crate::lean) fn turn_inner(
        &self,
        setup: TurnSetup<'_>,
        cancel: &AtomicBool,
        prompt: impl FnOnce(bool) -> String,
        mail_only: bool,
    ) -> Result<Option<String>> {
        let lock = self.turn_lock(setup.conversation, &setup.role);
        let Some(_turn_guard) = mail::lock_cancellable(&lock, cancel) else {
            return Ok(None);
        };
        let TurnSetup {
            session_policy,
            conversation,
            plan_id,
            role,
            profile,
            cwd,
            add_dirs,
            access,
            mut upto_message,
        } = setup;
        if mail_only {
            if self.store.pending_mail(conversation, &role)?.is_empty() {
                return Ok(None);
            }
            upto_message = self.store.last_answered(conversation, &role)?;
            if matches!(role.as_str(), ORCHESTRATOR | COMMENT_ORCHESTRATOR) {
                upto_message = self.store.last_orchestration_answered(conversation)?;
            }
        }
        if role != ORCHESTRATOR && role != COMMENT_ORCHESTRATOR && role != PLAN_VERIFIER {
            // Cancelling a paused conversation ends what waits, too.
            while !cancel.load(Ordering::SeqCst) && self.store.paused(conversation)? {
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
        }
        let provider = profile
            .as_ref()
            .map_or("unconfigured", |profile| agent::name(profile.provider));
        let resume = self.session_for_turn(
            conversation,
            &role,
            provider,
            &session_policy,
            profile.as_ref().ok(),
            &cwd,
            &add_dirs,
            access,
        )?;
        let mut inbox = self.store.pending_mail(conversation, &role)?;
        let mut history_mail = std::collections::HashSet::new();
        if role == ORCHESTRATOR && !mail_only {
            let mut visible = Vec::with_capacity(inbox.len());
            for mail in inbox {
                if mail.kind == "agent_work"
                    && let Some(message) =
                        self.store.agent_mail_user_message(conversation, mail.id)?
                {
                    if message > upto_message {
                        continue;
                    }
                    history_mail.insert(mail.id);
                }
                visible.push(mail);
            }
            inbox = visible;
        }
        if mail_only && role == ORCHESTRATOR {
            inbox.retain(|mail| mail.kind != "agent_work");
        }
        if mail_only && inbox.is_empty() {
            return Ok(None);
        }
        let comments: Vec<u64> = if role == ORCHESTRATOR || role == COMMENT_ORCHESTRATOR {
            let answered = self.store.last_orchestration_answered(conversation)?;
            self.store
                .comments(conversation, "open")?
                .into_iter()
                .filter(|comment| comment.message > answered && comment.message <= upto_message)
                .map(|comment| comment.id)
                .collect()
        } else {
            Vec::new()
        };
        let turn = self.store.start_turn(
            conversation,
            &role,
            provider,
            upto_message,
            plan_id.as_deref(),
        )?;
        self.store.attribute_mail(
            conversation,
            &inbox.iter().map(|mail| mail.id).collect::<Vec<_>>(),
            &turn.id,
        )?;
        let persistence_error = std::cell::RefCell::new(None);
        let announce = |kind: &str, payload: serde_json::Value| {
            if let Err(error) = self.store.append_event(
                conversation,
                Some(&turn.id),
                turn.plan_id.as_deref(),
                kind,
                &payload,
            ) {
                persistence_error.borrow_mut().get_or_insert(error);
            }
        };
        let log = std::cell::RefCell::new(TurnLog::default());
        // A verifier reports its checks while it works; they are recorded as
        // they come, so the user follows them before the verdict exists.
        let live = role
            .strip_prefix("verifier:")
            .and_then(|task| self.store.task_by_id(task).ok().flatten());
        let progress = std::cell::RefCell::new(tasks::Progress::default());
        let record = |updates: Vec<tasks::CheckUpdate>| {
            if let Some(task) = &live {
                for update in updates {
                    self.record_check(conversation, task, &update);
                }
            }
        };
        let flush_text = |log: &std::cell::RefCell<TurnLog>| {
            let mut log = log.borrow_mut();
            let text = std::mem::take(&mut log.text);
            if text.is_empty() {
                return;
            }
            log.stored_text = true;
            announce("agent", json!({"type": "text", "delta": text}));
        };
        // What ran, for the history and its export (design §15).
        announce(
            "turn_started",
            json!({
                "role": role,
                "provider": provider,
                "model": profile.as_ref().ok().and_then(|profile| profile.model.clone()),
                "effort": profile.as_ref().ok().and_then(|profile| profile.native_effort.clone()),
                "comments": comments,
                "mail_only": mail_only,
                "execution_signature": profile.as_ref().ok().map(rounds::execution_signature).transpose()?,
            }),
        );
        let outcome = profile.and_then(|profile| {
            let profile_name = profile.profile.clone();
            let profile_effort = profile.effort;
            let spec = agent::Spec {
                provider: profile.provider,
                binary: profile.binary,
                args: profile.args,
                model: profile.model,
                effort: profile.native_effort,
                cwd,
                add_dirs,
                access,
                resume: resume.clone(),
            };
            let native = self.native.prepare_scoped(
                self,
                conversation,
                &role,
                &spec,
                &match &session_policy {
                    SessionPolicy::Role => String::new(),
                    SessionPolicy::TaskRound { .. } => format!(
                        "{session_policy:?}:{profile_name}:{}",
                        rounds::spec_signature(&profile_name, profile_effort, &spec)?
                    ),
                },
            )?;
            let identity = native
                .as_ref()
                .map(|session| session.identity.clone())
                .or_else(|| resume.clone());
            self.store.record_run_context(&store::RunContext {
                turn_id: turn.id.clone(),
                conversation: conversation.into(),
                role: role.clone(),
                provider: provider.into(),
                profile: profile_name,
                cwd: spec.cwd.clone(),
                add_dirs: spec.add_dirs.clone(),
                access: match access {
                    agent::Access::ReadOnly => store::RunAccess::ReadOnly,
                    agent::Access::Write => store::RunAccess::Write,
                },
                backend: native
                    .as_ref()
                    .map_or(store::RunBackend::Cli, |session| session.backend_kind()),
                provider_thread_id: (spec.provider == crate::config::ProviderKind::Codex)
                    .then(|| identity.clone())
                    .flatten(),
                provider_session_id: (spec.provider != crate::config::ProviderKind::Codex)
                    .then(|| identity.clone())
                    .flatten(),
                provider_turn_id: None,
            })?;
            // A resumed CLI may omit another session announcement. Persist
            // its known identity now so further turns in this round continue it.
            if let Some(identity) = &identity {
                self.store.set_session(&turn.id, identity)?;
            }
            let mut prompt_mail = inbox
                .iter()
                .filter(|mail| !history_mail.contains(&mail.id))
                .peekable();
            let mut text = if prompt_mail.peek().is_none() {
                String::new()
            } else {
                tasks::inbox_text_from(prompt_mail)
            };
            text.push_str(&prompt(resume.is_none()));
            if resume.is_none()
                && let SessionPolicy::TaskRound { task, round } = &session_policy
            {
                let handoff = self.round_handoff(task, *round, &spec.cwd)?;
                text.push_str(&tasks::round_handoff_prompt(
                    &handoff,
                    task_role(&role).is_some_and(|(_, verifier)| verifier),
                ));
            }
            if let Some((id, _)) = task_role(&role)
                && let Some(task) = self.store.task_by_id(id)?
                && !mail_only
            {
                require(
                    task.status != status::STALE,
                    "task was invalidated before its process started",
                )?;
                self.store.assert_task_inputs(&task)?;
            }
            for message in &inbox {
                if message.sender == "user" {
                    announce("agent", json!({"type":"user","text":message.text}));
                }
            }
            // What the agent was asked, for its session view.
            announce(
                "agent",
                serde_json::to_value(agent::Event::Prompt { text: text.clone() })
                    .unwrap_or_default(),
            );
            let mut on_event = |event| {
                match &event {
                    agent::Event::ProviderTurn { id } if native.is_some() => {
                        if let Err(error) =
                            self.store.bind_provider_turn(conversation, &turn.id, id)
                        {
                            persistence_error.borrow_mut().get_or_insert(error);
                        }
                    }
                    agent::Event::Session { id } => {
                        if let Err(error) = self.store.set_session(&turn.id, id) {
                            persistence_error.borrow_mut().get_or_insert(error);
                        }
                    }
                    agent::Event::Process { pid } => {
                        let _ = self.store.set_turn_pid(&turn.id, *pid);
                    }
                    // Streamed fragments are joined and stored as one block
                    // when something else happens, not as a row each.
                    agent::Event::Text { delta } => {
                        log.borrow_mut().text.push_str(delta);
                        record(progress.borrow_mut().push(delta));
                        if log.borrow().text.len() >= content::CHUNK_BYTES {
                            flush_text(&log);
                        }
                        return;
                    }
                    _ => {}
                }
                flush_text(&log);
                // Reasoning and tool output are bounded per turn.
                let weight = match &event {
                    agent::Event::Reasoning { delta } => delta.len(),
                    agent::Event::Output { text, .. } => text.len(),
                    _ => 0,
                };
                if weight > 0 && !log.borrow_mut().admit(weight) {
                    if !std::mem::replace(&mut log.borrow_mut().marked, true) {
                        announce(
                            "agent",
                            json!({"type": "output", "name": "antislop",
                                   "text": "[further output of this turn is not stored]",
                                   "exit_code": null}),
                        );
                    }
                    return;
                }
                if let Ok(payload) = serde_json::to_value(&event) {
                    announce("agent", payload);
                }
            };
            if let Some(native) = native.as_ref() {
                native.run(&turn.id, &text, &inbox, cancel, &mut on_event)
            } else {
                agent::run(&spec, &text, cancel, &mut on_event)
            }
        });
        flush_text(&log);
        record(progress.borrow_mut().finish());
        match outcome {
            Ok(outcome) => {
                // Some CLIs report the answer only at the end.
                record(
                    outcome
                        .text
                        .lines()
                        .filter_map(tasks::parse_progress)
                        .collect(),
                );
                // Some CLIs report the answer only at the end.
                if !log.borrow().stored_text {
                    let preview = &outcome.text;
                    if !preview.is_empty() {
                        announce("agent", json!({"type": "text", "delta": preview}));
                    }
                }
                announce("agent_final", json!({"text": outcome.text}));
                if let Some(error) = persistence_error.borrow_mut().take() {
                    self.store.finish_turn(
                        &turn.id,
                        TurnStatus::Failed,
                        Some(&error.to_string()),
                    )?;
                    return Err(error);
                }
                self.store.finish_turn(&turn.id, TurnStatus::Done, None)?;
                let delivered: Vec<u64> = inbox.iter().map(|mail| mail.id).collect();
                self.store
                    .mark_delivered(conversation, &delivered, &turn.id)?;
                announce("turn_finished", json!({"role": role, "status": "done"}));
                Ok(Some(outcome.text))
            }
            Err(error) => {
                let status = if error.to_string().contains("native turn interrupted") {
                    TurnStatus::Interrupted
                } else if cancel.load(Ordering::SeqCst) {
                    TurnStatus::Cancelled
                } else {
                    TurnStatus::Failed
                };
                let error = agent::persisted_error(&error.to_string());
                self.store.finish_turn(&turn.id, status, Some(&error))?;
                let direct = inbox
                    .iter()
                    .filter(|mail| mail.kind == "agent_work")
                    .map(|mail| mail.id)
                    .collect::<Vec<_>>();
                if !direct.is_empty() {
                    announce(
                        "mail_delivery_failed",
                        json!({"recipient":role,"ids":direct,"reason":error,"state":"queued"}),
                    );
                }

                announce(
                    "turn_finished",
                    json!({"role": role, "status": status, "error": error}),
                );
                Ok(None)
            }
        }
    }
}
