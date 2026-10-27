use super::core_feedback_helpers::{event_files, land, note};
use super::core_path_helpers::{worktree_branch, worktree_path};
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
        let spec: TaskContract = serde_json::from_value(task.spec.clone())?;
        self.store.assert_task_inputs(&task)?;
        let current_token = self.task_freshness_token(&req.conversation, &req.plan, &task.id)?;
        require(req.freshness_token == current_token, "stale_token")?;
        require(
            (matches!(
                req.action,
                RecoveryAction::RestartAgent | RecoveryAction::CancelTask
            ) && matches!(
                task.status.as_str(),
                status::QUEUED | status::WORKING | status::VERIFYING
            )) || task.status == status::COORDINATING
                || task.status == status::NEEDS_USER
                || task.status == status::WAITING
                || task.status == status::FAILED
                || task.status == status::SKIPPED
                || task.status == status::CANCELLED,
            "task is not waiting for recovery",
        )?;

        if matches!(
            req.action,
            RecoveryAction::RestartAgent | RecoveryAction::CancelTask
        ) {
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
                require(
                    task.verdict.as_ref().is_some_and(|v| v["approved"] == true),
                    "independent review must approve the current result before integration",
                )?;
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
                    let worktree = worktree_path(&self.state, &task)?;
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
                let worktree = worktree_path(&self.state, &task)?;
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
                let reviewdir = self.state.join("worktrees").join(format!(
                    "{}-manual-review-g{}-{}",
                    task.id, task.input_generation, task.round
                ));
                git::add_worktree(
                    &project,
                    &reviewdir,
                    &format!(
                        "antislop/{}-manual-review-g{}-{}",
                        task.id, task.input_generation, task.round
                    ),
                    &snapshot,
                )?;
                let verifier_project = reviewdir.to_string_lossy().into_owned();
                let setup = TurnSetup {
                    session_policy: SessionPolicy::TaskRound {
                        task: task.id.clone(),
                        round: task.round,
                    },
                    conversation: &req.conversation,
                    plan_id: Some(task.plan_id.clone()),
                    role: verifier.clone(),
                    profile: config.resolve(spec.kind, RoleKind::Verifier),
                    cwd: reviewdir,
                    add_dirs: vec![],
                    access: agent::Access::ReadOnly,
                    upto_message: 0,
                };
                let upstream = self.upstream(&task.plan_id, &spec)?;
                let cancel = AtomicBool::new(false);
                let answer = self.turn(setup, &cancel, |fresh| {
                    tasks::verifier_turn_prompt(
                        &verifier_project,
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
                let Some(answer) = answer else {
                    self.stop_task(&req.conversation, &mut task, &verifier, &cancel)?;
                    return Err(Error::Invalid(
                        "manual snapshot verifier failed; inspect its raw turn error".into(),
                    ));
                };
                let mut verdict = match serde_json::from_str::<turn_result::TurnResult>(&answer)? {
                    turn_result::TurnResult::Review { verdict, .. } => verdict,
                    turn_result::TurnResult::NeedsInput {
                        request,
                        checkpoint,
                    } => {
                        task.report = Some(checkpoint);
                        self.wait_for_question(&req.conversation, &mut task, &request, true)?;
                        return Err(Error::Invalid(
                            "manual snapshot awaits orchestrator answer".into(),
                        ));
                    }
                    _ => {
                        return Err(Error::Invalid(
                            "manual snapshot requires an independent review".into(),
                        ));
                    }
                };
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
                            Outcome::Committed,
                            Some(git::commit_paths(&project, &fixed, &spec.title)?),
                        )
                    } else {
                        (Outcome::Applied, None)
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
                    let worktree = worktree_path(&self.state, &task)?;
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
                        Outcome::Committed,
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
            RecoveryAction::CancelTask => {
                task.status = status::CANCELLED.into();
                task.verdict = None;
                notice = "task cancelled; its outstanding requirements remain unsatisfied".into();
                note(&mut task, notice.clone());
                self.store.update_task(&task)?;
                self.announce_task(&req.conversation, &task, Some(&notice));
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
