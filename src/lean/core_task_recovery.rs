use super::core_feedback_helpers::{event_files, land, note};
use super::core_path_helpers::{task_path_component, worktree_branch};
use super::*;

impl Core {
    pub(in crate::lean) fn recover_task_as(
        self: &Arc<Self>,
        req: TaskRecoveryRequest,
        actor: &str,
    ) -> Result<TaskRecoveryResponse> {
        let _graph = self.graph_updates.lock().expect("graph update mutex");
        let config = self.config_snapshot();
        let conv = self.store.conversation(&req.conversation)?;
        let project = PathBuf::from(conv.project);
        let _one_at_a_time = self.integrating.lock().expect("integration mutex");
        let mut task = self.task(&req.plan, &req.task)?;
        let spec: TaskSpec = serde_json::from_value(task.spec.clone())?;
        self.store.assert_task_inputs(&task)?;
        let current_token = self.task_freshness_token(&req.conversation, &req.plan, &task.id)?;
        require(req.freshness_token == current_token, "stale_token")?;
        require(
            (req.action == RecoveryAction::RestartAgent
                && matches!(
                    task.status.as_str(),
                    status::QUEUED | status::WORKING | status::VERIFYING
                ))
                || task.status == status::NEEDS_USER
                || task.status == status::WAITING
                || task.status == status::FAILED
                || task.status == status::SKIPPED
                || task.status == status::CANCELLED,
            "task is not waiting for recovery",
        )?;

        if req.action == RecoveryAction::RestartAgent {
            let active = self.task_cancels.lock().expect("task cancel mutex");
            if let Some((_, cancel)) = active.get(&task.id) {
                self.task_restarts
                    .lock()
                    .expect("task restart mutex")
                    .insert(task.id.clone(), req);
                cancel.store(true, Ordering::SeqCst);
                return Ok(TaskRecoveryResponse {
                    task: task.id,
                    status: task.status,
                    round: task.round,
                    integrated_files: Vec::new(),
                    preserved_branch: None,
                    new_freshness_token: current_token,
                    notice: "stopping the current round before restarting the agent".into(),
                });
            }
        }

        let mut integrated_files = Vec::new();
        let mut preserved_branch = None;
        let notice: String;

        match req.action {
            RecoveryAction::IntegrateWithConsent => {
                let change = task.result.clone().ok_or_else(|| {
                    Error::Invalid("task has no result commit to integrate".into())
                })?;
                let base = task
                    .base
                    .clone()
                    .ok_or_else(|| Error::Invalid("task has no base commit".into()))?;
                let all_changed = git::changed_files(&project, &base, &change)?;
                if let Some(consent) = &req.consent_files {
                    for file in consent {
                        require(
                            all_changed.contains(file),
                            &format!("consent file '{file}' is not in changed files"),
                        )?;
                        require(
                            spec.scope.is_empty()
                                || spec.scope.iter().any(|s| {
                                    s == "." || s.is_empty() || file.starts_with(s) || s == file
                                }),
                            &format!("consent file '{file}' is outside task scope"),
                        )?;
                    }
                }
                let outcome = git::integrate_scoped(
                    &project,
                    &base,
                    &change,
                    req.consent_files.as_deref(),
                    config.policy.local_commits,
                )?;
                let mut card = (Outcome::Applied, None);
                match outcome {
                    git::Integration::Committed(commit) => {
                        task.status = status::APPROVED.to_string();
                        notice = format!("committed as {commit}");
                        card = (Outcome::Committed, Some(commit));
                    }
                    git::Integration::Applied => {
                        task.status = status::APPROVED.to_string();
                        notice = "applied to the working tree, not committed".to_string();
                    }
                    git::Integration::Conflict(conflicts) => {
                        return Err(Error::Invalid(format!(
                            "conflicts remain in {}",
                            conflicts.join(", ")
                        )));
                    }
                    git::Integration::UserEdits(edits) => {
                        return Err(Error::Invalid(format!(
                            "uncommitted edits remain in {}",
                            edits.join(", ")
                        )));
                    }
                }
                integrated_files = req.consent_files.unwrap_or_else(|| all_changed.clone());
                if integrated_files.len() < all_changed.len() {
                    let current = git::snapshot(&project, "Check remaining task changes")?;
                    if git::matching_paths(&project, &current, &change, &all_changed)?.len()
                        != all_changed.len()
                    {
                        task.status = status::NEEDS_USER.to_string();
                    }
                }
                if task.status == status::APPROVED {
                    let worktree = self
                        .state
                        .join("worktrees")
                        .join(task_path_component(&task.id)?);
                    if worktree.exists() {
                        git::remove_worktree(
                            &project,
                            &worktree,
                            &worktree_branch(&self.state, &task)?,
                        )?;
                    }
                }
                land(
                    &mut task,
                    card.0,
                    "recovery",
                    &notice,
                    card.1.as_deref(),
                    None,
                    &integrated_files,
                );
                note(&mut task, notice.clone());
                self.store.update_task(&task)?;
                self.announce_task(&req.conversation, &task, None);
                self.store.append_event(
                    &req.conversation,
                    None,
                    Some(&req.plan),
                    "task_recovery",
                    &json!({
                        "actor": actor,
                        "task": task.id,
                        "action": tasks::action::INTEGRATE_WITH_CONSENT,
                        "status": task.status,
                        "files": event_files(&integrated_files),
                        "files_more": integrated_files.len().saturating_sub(tasks::FILE_LIMIT),
                        "detail": tasks::bound(&notice, tasks::DETAIL_LIMIT)
                    }),
                )?;
                if task.status == status::APPROVED {
                    self.unblock_dependent_tasks(&req.conversation, &req.plan, &task.id)?;
                }
                self.maybe_continue_plan(&req.conversation, &req.plan)?;
            }
            RecoveryAction::KeepOnBranch => {
                let b = worktree_branch(&self.state, &task)?;
                preserved_branch = Some(b.clone());
                notice = format!("change preserved on branch {b}; working tree untouched");
                task.status = status::NEEDS_USER.to_string();
                land(
                    &mut task,
                    Outcome::LeftOnBranch,
                    "recovery",
                    &notice,
                    None,
                    Some(&b),
                    &[],
                );
                let worktree = self
                    .state
                    .join("worktrees")
                    .join(task_path_component(&task.id)?);
                let _ = git::remove_worktree_keep_branch(&project, &worktree);
                note(&mut task, notice.clone());
                self.store.update_task(&task)?;
                self.announce_task(&req.conversation, &task, None);
                self.store.append_event(
                    &req.conversation,
                    None,
                    Some(&req.plan),
                    "task_recovery",
                    &json!({
                        "actor": actor,
                        "task": task.id,
                        "action": tasks::action::KEEP_ON_BRANCH,
                        "status": task.status,
                        "branch": b,
                        "detail": tasks::bound(&notice, tasks::DETAIL_LIMIT)
                    }),
                )?;
            }
            RecoveryAction::VerifyManualSnapshot => {
                let snapshot = git::snapshot(
                    &project,
                    &format!("manual fix verification: {}", spec.title),
                )?;
                let base = task
                    .base
                    .clone()
                    .unwrap_or_else(|| git::head(&project).unwrap_or_default());
                let stat = git::diff_stat(&project, &base, &snapshot)?;
                let changes = tasks::changes_text(&base, &snapshot, &stat);

                task.round += 1;
                task.verdict = None;
                task.base = Some(base.clone());
                task.result = Some(snapshot.clone());
                self.set_status(&req.conversation, &mut task, status::VERIFYING)?;
                let verifier = format!("verifier:{}", task.id);
                let setup = TurnSetup {
                    session_policy: SessionPolicy::TaskRound {
                        task: task.id.clone(),
                        round: task.round,
                    },
                    conversation: &req.conversation,
                    plan_id: Some(task.plan_id.clone()),
                    role: verifier.clone(),
                    profile: config.resolve(spec.kind, RoleKind::Verifier),
                    cwd: project.clone(),
                    add_dirs: vec![],
                    access: agent::Access::ReadOnly,
                    upto_message: 0,
                };
                let upstream = self.upstream(&task.plan_id, &spec)?;
                let cancel = AtomicBool::new(false);
                let answer = self.turn(setup, &cancel, |fresh| {
                    tasks::verifier_turn_prompt(
                        &project.to_string_lossy(),
                        &spec,
                        if fresh {
                            ""
                        } else {
                            task.report.as_deref().unwrap_or("")
                        },
                        Some(&changes),
                        fresh,
                        if fresh { "" } else { &upstream },
                    )
                })?;
                let answer = answer.unwrap_or_default();
                let answer = self.post(
                    &req.conversation,
                    Some(&task.plan_id),
                    &format!("{}/verifier", spec.id),
                    &answer,
                )?;
                let mut verdict = tasks::parse_verdict(&answer);
                let obligations = tasks::obligations(&spec);
                tasks::settle(&mut verdict, &obligations, !spec.writes);

                let mut v_val = serde_json::to_value(&verdict)?;
                if let Some(obj) = v_val.as_object_mut() {
                    obj.insert("round".into(), json!(task.round));
                    obj.insert("result".into(), json!(snapshot));
                }
                task.verdict = Some(v_val);
                let fixed = git::changed_files(&project, &base, &snapshot).unwrap_or_default();
                let (effect, effect_cut) = tasks::effect(&stat, &fixed);
                task.feedback = Some(json!({
                    "round": task.round,
                    "effect": effect,
                    "effect_cut": effect_cut,
                    "coverage": tasks::coverage(&verdict, &obligations),
                }));

                if verdict.approved {
                    task.result = Some(snapshot.clone());
                    task.status = status::APPROVED.to_string();
                    let (outcome, commit) = if config.policy.local_commits && !fixed.is_empty() {
                        (
                            Outcome::InPlaceCommitted,
                            Some(git::commit_paths(&project, &fixed, &spec.title)?),
                        )
                    } else {
                        (Outcome::InPlaceApplied, None)
                    };
                    notice = match &commit {
                        Some(commit) => format!("manual fixes verified and committed as {commit}"),
                        None => format!("manual fixes verified and applied on snapshot {snapshot}"),
                    };
                    land(
                        &mut task,
                        outcome,
                        "recovery",
                        &notice,
                        commit.as_deref(),
                        None,
                        &fixed,
                    );
                    note(&mut task, notice.clone());
                    let worktree = self
                        .state
                        .join("worktrees")
                        .join(task_path_component(&task.id)?);
                    if worktree.exists() {
                        git::remove_worktree(
                            &project,
                            &worktree,
                            &worktree_branch(&self.state, &task)?,
                        )?;
                    }
                    self.store.update_task(&task)?;
                    self.announce_task(&req.conversation, &task, None);
                    self.store.append_event(
                        &req.conversation,
                        None,
                        Some(&req.plan),
                        "task_recovery",
                        &json!({
                            "actor": actor,
                            "task": task.id,
                            "action": tasks::action::VERIFY_MANUAL_SNAPSHOT,
                            "status": task.status,
                            "result": snapshot,
                            "detail": tasks::bound(&notice, tasks::DETAIL_LIMIT)
                        }),
                    )?;
                    self.unblock_dependent_tasks(&req.conversation, &req.plan, &task.id)?;
                    self.maybe_continue_plan(&req.conversation, &req.plan)?;
                } else {
                    task.status = status::NEEDS_USER.to_string();
                    let findings = tasks::findings_text(&verdict.findings);
                    notice = format!("manual fix verification found open issues:\n{findings}");
                    task.report = Some(format!(
                        "{}\n\n[manual fix verification]\n{findings}",
                        task.report.as_deref().unwrap_or("")
                    ));
                    land(
                        &mut task,
                        Outcome::InPlaceCommitted,
                        "recovery",
                        &notice,
                        Some(&snapshot),
                        None,
                        &fixed,
                    );
                    self.store.update_task(&task)?;
                    self.announce_task(&req.conversation, &task, Some(&findings));
                    self.store.append_event(
                        &req.conversation,
                        None,
                        Some(&req.plan),
                        "task_recovery",
                        &json!({
                            "actor": actor,
                            "task": task.id,
                            "action": tasks::action::VERIFY_MANUAL_SNAPSHOT,
                            "status": task.status,
                            "findings": verdict.findings,
                            "detail": tasks::bound(&notice, tasks::DETAIL_LIMIT)
                        }),
                    )?;
                }
            }
            RecoveryAction::RetryWithGuidance => {
                if let Some(msg) = &req.message {
                    let mail = Mail {
                        id: 0,
                        created_ms: 0,
                        delivered_turn: None,
                        delivered_ms: None,
                        sender: RUNTIME.into(),
                        recipient: format!("worker:{}", task.id),
                        kind: "guidance".into(),
                        text: msg.clone(),
                        state: "queued".into(),
                    };
                    self.store.add_mail(
                        &req.conversation,
                        &format!("worker:{}", task.id),
                        &mail,
                    )?;
                }
                task.status = status::QUEUED.to_string();
                task.verdict = None;
                // A retry invalidates the earlier round's card result.
                task.feedback = None;
                notice = "retrying task with guidance".to_string();
                self.store.update_task(&task)?;
                self.announce_task(&req.conversation, &task, None);
                self.store.append_event(
                    &req.conversation,
                    None,
                    Some(&req.plan),
                    "task_recovery",
                    &json!({
                        "actor": actor,
                        "task": task.id,
                        "action": tasks::action::RETRY_WITH_GUIDANCE,
                        "status": task.status,
                        // whole guidance already queued to the worker as mail
                        "message": req.message
                            .as_deref()
                            .map(|m| tasks::bound(m, tasks::DETAIL_LIMIT))
                    }),
                )?;
                self.maybe_continue_plan(&req.conversation, &req.plan)?;
            }
            RecoveryAction::ExtendRounds => {
                task.status = status::QUEUED.to_string();
                task.verdict = None;
                task.feedback = None;
                notice = "extended rounds for task".to_string();
                self.store.update_task(&task)?;
                self.announce_task(&req.conversation, &task, None);
                self.store.append_event(
                    &req.conversation,
                    None,
                    Some(&req.plan),
                    "task_recovery",
                    &json!({
                        "actor": actor,
                        "task": task.id,
                        "action": tasks::action::EXTEND_ROUNDS,
                        "status": task.status
                    }),
                )?;
                self.maybe_continue_plan(&req.conversation, &req.plan)?;
            }
            RecoveryAction::RestartAgent => {
                // The next turns of this task start in fresh CLI sessions;
                // its worktree, report and results stay as they are.
                self.store.forget_task_sessions(&task.id)?;
                if let Some(msg) = &req.message {
                    let role = format!("worker:{}", task.id);
                    let mail = Mail {
                        id: 0,
                        created_ms: 0,
                        delivered_turn: None,
                        delivered_ms: None,
                        sender: RUNTIME.into(),
                        recipient: role.clone(),
                        kind: "guidance".into(),
                        text: msg.clone(),
                        state: "queued".into(),
                    };
                    self.store.add_mail(&req.conversation, &role, &mail)?;
                }
                task.status = status::QUEUED.to_string();
                task.verdict = None;
                task.feedback = None;
                notice = "restarting the task's agent in a fresh session".to_string();
                self.store.update_task(&task)?;
                self.announce_task(&req.conversation, &task, None);
                self.store.append_event(
                    &req.conversation,
                    None,
                    Some(&req.plan),
                    "task_recovery",
                    &json!({
                        "actor": actor,
                        "task": task.id,
                        "action": tasks::action::RESTART_AGENT,
                        "status": task.status
                    }),
                )?;
                self.maybe_continue_plan(&req.conversation, &req.plan)?;
            }
            RecoveryAction::ForceApprove => {
                let worker_done = self
                    .store
                    .last_turn(&req.conversation, &format!("worker:{}", task.id))?
                    .is_some_and(|turn| turn.status == TurnStatus::Done);
                require(
                    worker_done && task.report.is_some() && task.round > 0,
                    "a task that never ran cannot be approved; retry it instead",
                )?;
                if spec.writes {
                    require(
                        task.base.is_some() && task.result.is_some(),
                        "a writer must have a result commit before approval",
                    )?;
                    let worktree = self
                        .state
                        .join("worktrees")
                        .join(task_path_component(&task.id)?);
                    git::add_worktree(
                        &project,
                        &worktree,
                        &worktree_branch(&self.state, &task)?,
                        task.result.as_deref().unwrap(),
                    )?;
                    // integrate takes the same mutex; the recovery guard already serializes it.
                    drop(_one_at_a_time);
                    self.integrate(
                        &req.conversation,
                        &mut task,
                        &spec,
                        &project,
                        &worktree,
                        &config,
                        &AtomicBool::new(false),
                    )?;
                    require(
                        task.status == status::APPROVED,
                        "the result is not integrated; resolve the recorded integration problem",
                    )?;
                }
                task.status = status::APPROVED.to_string();
                notice = req
                    .message
                    .as_deref()
                    .map(|m| format!("verification overridden by {actor}: {m}"))
                    .unwrap_or_else(|| format!("verification overridden by {actor}"));
                if spec.writes {
                    if let Some(feedback) = task.feedback.as_mut() {
                        feedback["verification_override"] =
                            json!({"source": "recovery", "detail": notice});
                    }
                } else {
                    land(
                        &mut task,
                        Outcome::ForceApproved,
                        "recovery",
                        &notice,
                        None,
                        None,
                        &[],
                    );
                }
                note(&mut task, notice.clone());
                self.store.update_task(&task)?;
                self.announce_task(&req.conversation, &task, None);
                self.store.append_event(
                    &req.conversation,
                    None,
                    Some(&req.plan),
                    "task_recovery",
                    &json!({
                        "actor": actor,
                        "task": task.id,
                        "action": tasks::action::FORCE_APPROVE,
                        "status": task.status,
                        "detail": tasks::bound(&notice, tasks::DETAIL_LIMIT)
                    }),
                )?;
                self.unblock_dependent_tasks(&req.conversation, &req.plan, &task.id)?;
                // Mail that reached the task before it was approved by hand
                // has no round left to be read in.
                self.answer_in_background(&req.conversation, &req.plan);
                self.maybe_continue_plan(&req.conversation, &req.plan)?;
            }
            RecoveryAction::CancelTask => {
                return Err(Error::Invalid(
                    "tasks cannot be discarded; pause the plan or retry the task".into(),
                ));
            }
        }

        let new_freshness_token =
            self.task_freshness_token(&req.conversation, &req.plan, &task.id)?;
        Ok(TaskRecoveryResponse {
            task: task.id,
            status: task.status,
            round: task.round,
            integrated_files,
            preserved_branch,
            new_freshness_token,
            notice,
        })
    }
}
