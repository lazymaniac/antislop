use super::*;

impl Core {
    pub(in crate::lean) fn turn_inner(
        &self,
        setup: TurnSetup<'_>,
        cancel: &AtomicBool,
        prompt: impl FnOnce(bool) -> String,
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
            upto_message,
        } = setup;
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
        if role == ORCHESTRATOR {
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
        // What ran, for the history and its export (design §15).
        announce(
            "turn_started",
            json!({
                "role": role,
                "provider": provider,
                "model": profile.as_ref().ok().and_then(|profile| profile.model.clone()),
                "effort": profile.as_ref().ok().and_then(|profile| profile.native_effort.clone()),
                "comments": comments,
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
            let identity = native.identity().or_else(|| resume.clone());
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
                provider_thread_id: (spec.provider == crate::config::ProviderKind::Codex)
                    .then(|| identity.clone())
                    .flatten(),
                provider_session_id: (spec.provider != crate::config::ProviderKind::Codex)
                    .then(|| identity.clone())
                    .flatten(),
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
            if let Some((id, verifier)) = task_role(&role) {
                let task = self
                    .store
                    .task_by_id(id)?
                    .ok_or_else(|| Error::Invalid("task no longer exists".into()))?;
                if git::is_repository(&spec.cwd) {
                    text.push_str(&format!(
                        "Input revision: {}; source commit: {}\n",
                        task.input_generation,
                        git::head(&spec.cwd)?
                    ));
                }
                let local: TaskContract = serde_json::from_value(task.spec)?;
                let questions = self.store.local_questions(&turn.id)?;
                if !questions.is_empty() {
                    text.push_str(&format!(
                        "Your task questions and answers: {}\n",
                        serde_json::to_string(&questions)?
                    ));
                }
                text.push_str(&format!(
                    "Local check ids: {}\n",
                    serde_json::to_string(
                        &tasks::obligations(&local)
                            .iter()
                            .map(|(id, _)| id)
                            .collect::<Vec<_>>()
                    )?
                ));
                text.push_str(if verifier {
                    "Task role: verifier\n"
                } else {
                    "Task role: worker\n"
                });
            }
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
            {
                require(
                    task.status != status::STALE,
                    "task was invalidated before its process started",
                )?;
                self.store.assert_task_inputs(&task)?;
            }
            let weak = self.weak.get().expect("core weak reference").clone();
            let (conv, agent_role, agent_turn) =
                (conversation.to_owned(), role.clone(), turn.id.clone());
            native.set_tool_handler(Arc::new(move |tool, args| {
                let core = weak
                    .upgrade()
                    .ok_or_else(|| Error::Invalid("daemon stopped".into()))?;
                let outcome = core.agent_tool(&conv, &agent_role, &agent_turn, tool, args);
                if let Err(error) = &outcome {
                    core.store.append_event(
                        &conv,
                        Some(&agent_turn),
                        None,
                        "agent_tool_rejected",
                        &json!({"tool":tool,"error":error.to_string()}),
                    )?;
                }
                outcome
            }))?;
            native.run(&self.native, &turn.id, &text, cancel)
        });
        match outcome {
            Ok(outcome) => {
                if let Some(session) = &outcome.session {
                    self.store.set_session(&turn.id, session)?;
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
                let status = if cancel.load(Ordering::SeqCst) {
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
