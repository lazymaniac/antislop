use super::core_feedback_helpers::land;
use super::core_path_helpers::{task_path_component, worktree_branch};
use super::*;

impl Core {
    /// Worker and verifier take turns until the verifier approves or the
    /// rounds run out; each new round starts fresh sessions with a durable handoff.
    pub(in crate::lean) fn run_task(
        self: &Arc<Self>,
        conversation: &str,
        task: &mut Task,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let integration_repairs = task
            .feedback
            .as_ref()
            .and_then(|feedback| feedback["integration_repairs"].as_u64())
            .unwrap_or(0);
        let config = self.config_snapshot();
        self.store.assert_task_inputs(task)?;
        let mut spec: TaskSpec = serde_json::from_value(task.spec.clone())?;
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        let in_place = spec.writes && config.workspace.mode == WorkspaceMode::InPlace;
        let autonomous = self.autonomous(conversation)?;
        // In place, writers take turns in the user's directory.
        let _writing = if in_place {
            require(
                git::is_repository(&project),
                "tasks that change files need the project to be a git repository: run `git init` and commit first",
            )?;
            require(
                config.policy.local_write,
                "in-place writers need policy.local_write",
            )?;
            loop {
                match self.in_place.try_lock() {
                    Ok(guard) => break Some(guard),
                    // A writer that panicked left nothing half-locked.
                    Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                        break Some(poisoned.into_inner());
                    }
                    Err(std::sync::TryLockError::WouldBlock) => {}
                }
                if cancel.load(Ordering::SeqCst) {
                    return self.set_status(conversation, task, status::QUEUED);
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        } else {
            None
        };
        if in_place && task.base.is_none() {
            task.base = Some(git::snapshot(&project, &format!("before {}", spec.title))?);
        }
        // Earlier approved results may still be uncommitted. Exempt only
        // paths that still match their latest approved version.
        let user_edits = match (&task.base, in_place) {
            (Some(base), true) => self.in_place_user_edits(&project, base)?,
            _ => vec![],
        };
        // A writer works in its own worktree (in place: in the project);
        // everyone else in the project.
        let worktree = if spec.writes && !in_place {
            require(
                git::is_repository(&project),
                "tasks that change files need the project to be a git repository: run `git init` and commit first",
            )?;
            let base = match &task.base {
                Some(base) => base.clone(),
                None => match self.writer_base(&task.plan_id, &spec, &project) {
                    Ok(base) => base,
                    Err(error) if error.to_string().contains("could not combine the results") => {
                        self.repair_writer_base(conversation, task, &mut spec, &project, &error)?
                    }
                    Err(error) => return Err(error),
                },
            };
            let path = self
                .state
                .join("worktrees")
                .join(task_path_component(&task.id)?);
            // A resumed writer whose worktree is gone continues from its work.
            let start = task.result.clone().unwrap_or_else(|| base.clone());
            git::add_worktree(
                &project,
                &path,
                &worktree_branch(&self.state, task)?,
                &start,
            )?;
            task.base = Some(base);
            Some(path)
        } else {
            None
        };
        let workdir = worktree.clone().unwrap_or_else(|| project.clone());
        let project_text = workdir.to_string_lossy().into_owned();
        let access = if spec.writes {
            agent::Access::Write
        } else {
            agent::Access::ReadOnly
        };
        // Related directories outside the project (`../service` included)
        // are made readable; paths inside it already are. A writer's CLI
        // would make them writable too, so a writer only reads them where
        // its tools allow: a plan never widens where an agent may write.
        let add_dirs = spec
            .scope
            .iter()
            .filter_map(|path| project.join(path).canonicalize().ok())
            .map(|path| {
                if path.is_dir() {
                    path
                } else {
                    path.parent().map(Path::to_path_buf).unwrap_or(path)
                }
            })
            .filter(|path| !path.starts_with(&project))
            .collect::<Vec<_>>();
        let team: Vec<TaskSpec> = self
            .store
            .plan_specs(&task.plan_id)?
            .into_iter()
            .map(serde_json::from_value)
            .collect::<std::result::Result<_, _>>()?;
        let upstream = self.upstream(&task.plan_id, &spec)?;
        let worker = format!("worker:{}", task.id);
        let verifier = format!("verifier:{}", task.id);
        // A resumed task keeps its rounds and its verifier's last word.
        let verdict = task
            .verdict
            .as_ref()
            .and_then(|verdict| serde_json::from_value::<Verdict>(verdict.clone()).ok());
        if let Some(verdict) = &verdict
            && verdict.approved
        {
            if let Some(path) = &worktree {
                return self.integrate(conversation, task, &spec, &project, path, &config, cancel);
            }
            if in_place {
                return self.finish_in_place(conversation, task, &spec, &project, &config);
            }
        }
        let mut findings = verdict.map(|verdict| verdict.findings).unwrap_or_default();
        let mut dependents_invalidated = self
            .store
            .pending_mail(conversation, &worker)?
            .iter()
            .any(|mail| mail.kind == "agent_work");
        let resumed = task.round > 0;
        let first_round = task.round + 1;
        // Supervised: a few rounds run on their own, then the user decides.
        // Autonomous: rounds go on until approval or until the verifier
        // repeats its findings, which means the worker is not getting closer.
        let last_round = if autonomous {
            u32::MAX
        } else {
            first_round + tasks::MAX_ROUNDS - 1
        };
        let mut previous: Option<String> = None;
        for round in first_round..=last_round {
            let config = self.config_snapshot();
            task.round = round;
            task.verdict = None;
            // A verifier-only request checks the same result; a worker round
            // begins without exposing the preceding report as its new result.
            let recheck = task.report.is_some()
                && self
                    .store
                    .pending_mail(conversation, &verifier)?
                    .iter()
                    .any(|mail| mail.kind == "agent_work")
                && self.store.pending_mail(conversation, &worker)?.is_empty();
            if !recheck && resumed && !dependents_invalidated {
                self.store.invalidate_task_dependents(conversation, task)?;
                dependents_invalidated = true;
            }
            if !recheck {
                task.report = None;
            }
            task.feedback = None;
            self.set_status(
                conversation,
                task,
                if recheck {
                    status::VERIFYING
                } else {
                    status::WORKING
                },
            )?;
            let worker_task = task.id.clone();
            let worker_plan = task.plan_id.clone();
            let worker_setup = || TurnSetup {
                session_policy: SessionPolicy::TaskRound {
                    task: worker_task.clone(),
                    round,
                },
                conversation,
                plan_id: Some(worker_plan.clone()),
                role: worker.clone(),
                profile: self
                    .config_snapshot()
                    .resolve(spec.kind, RoleKind::Executor),
                cwd: workdir.clone(),
                add_dirs: if spec.writes {
                    vec![]
                } else {
                    add_dirs.clone()
                },
                access,
                upto_message: 0,
            };
            let report = if recheck {
                task.report.clone()
            } else {
                self.turn(worker_setup(), cancel, |first| {
                    let continuation = if round == 1 {
                        None
                    } else if resumed && round == first_round {
                        Some(tasks::worker_resume(&findings))
                    } else {
                        Some(tasks::worker_followup(&findings))
                    };
                    tasks::worker_turn_prompt(
                        &project_text,
                        &spec,
                        &team,
                        autonomous,
                        continuation.as_deref(),
                        if first { "" } else { &upstream },
                        &if first {
                            String::new()
                        } else {
                            tasks::user_edits_note(&user_edits)
                        },
                    )
                })?
            };
            let Some(report) = report else {
                return self.stop_task(conversation, task, &worker, cancel);
            };
            let mut report = self.post(conversation, Some(&task.plan_id), &spec.id, &report)?;
            task.report = Some(report.clone());
            self.store.update_task(task)?;
            // Approved tasks addressed by it answer now, so its next turn reads them.
            self.answer_waiting(conversation, &task.plan_id, cancel)?;
            // Messages that arrived while it worked get a turn of their own.
            for _ in 0..MAIL_TURNS {
                if self.store.pending_mail(conversation, &worker)?.is_empty() {
                    break;
                }
                let next = self.turn(worker_setup(), cancel, |first| {
                    tasks::worker_turn_prompt(
                        &project_text,
                        &spec,
                        &team,
                        autonomous,
                        if first {
                            None
                        } else {
                            Some(tasks::INBOX_FOLLOWUP)
                        },
                        if first { "" } else { &upstream },
                        &if first {
                            String::new()
                        } else {
                            tasks::user_edits_note(&user_edits)
                        },
                    )
                })?;
                let Some(next) = next else {
                    return self.stop_task(conversation, task, &worker, cancel);
                };
                report = self.post(conversation, Some(&task.plan_id), &spec.id, &next)?;
                task.report = Some(report.clone());
                self.store.update_task(task)?;
            }
            // The worker may have stated its own effect for the card in the
            // same answer. Keep that line for feedback and drop the block
            // from the stored report; an empty or damaged block stays in
            // the report whole and the round falls back to the diffstat
            // (writers) or the report's first text line (read-only).
            if let Some(blocker) = blocker::parse(&report) {
                let reason = blocker.summary();
                if let Some(path) = &worktree {
                    task.result = Some(git::commit_all(
                        path,
                        &format!("{} (blocked round {round})", spec.title),
                    )?);
                } else if in_place {
                    task.result = Some(git::snapshot(
                        &project,
                        &format!("{} (blocked round {round})", spec.title),
                    )?);
                }
                task.report = Some(format!("{report}\n\n[runtime] {reason}"));
                task.verdict = None;
                return self.stop(conversation, task, status::NEEDS_USER, Some(&reason));
            }
            if let Some(question) = tasks::integration_question(&report) {
                task.report = Some(report);
                task.verdict = None;
                return self.stop(conversation, task, status::NEEDS_USER, Some(&question));
            }
            let (report, stated) = tasks::parse_effect(&report);
            task.report = Some(report.clone());
            let mut effect_text = stated;
            let changes = match (&worktree, &task.base) {
                (Some(path), Some(base)) => {
                    let result = git::commit_all(path, &format!("{} (round {round})", spec.title))?;
                    task.result = Some(result.clone());
                    let stat = git::diff_stat(path, base, &result)?;
                    let files = git::changed_files(path, base, &result)?;
                    effect_text.get_or_insert_with(|| tasks::effect(&stat, &files));
                    Some(tasks::changes_text(base, &result, &stat))
                }
                (None, Some(base)) if in_place => {
                    let result =
                        git::snapshot(&project, &format!("{} (round {round})", spec.title))?;
                    task.result = Some(result.clone());
                    let stat = git::diff_stat(&project, base, &result)?;
                    let files = git::changed_files(&project, base, &result)?;
                    effect_text.get_or_insert_with(|| tasks::effect(&stat, &files));
                    let touched: Vec<String> = files
                        .into_iter()
                        .filter(|file| user_edits.contains(file))
                        .collect();
                    if !touched.is_empty() {
                        let text = format!(
                            "the worker changed files you were editing: {}. Your version is kept: \
                              `git checkout {base} -- <file>` restores it",
                            touched.join(", ")
                        );
                        let (effect, effect_cut) = tasks::effect_keys(&effect_text);
                        let _ = self.store.append_event(
                            conversation,
                            None,
                            Some(&task.plan_id),
                            "integration",
                            &json!({"id": spec.id, "title": spec.title,
                                    "status": status::NEEDS_USER,
                                    "detail": tasks::bound(&text, tasks::DETAIL_LIMIT)}),
                        );
                        task.report = Some(format!("{report}\n\n[in place] {text}"));
                        task.feedback = Some(json!({"round": task.round, "effect": effect,
                                        "effect_cut": effect_cut}));
                        land(
                            task,
                            Outcome::InPlaceUserEdits,
                            "run",
                            &text,
                            None,
                            None,
                            &touched,
                        );
                        return self.stop(conversation, task, status::NEEDS_USER, Some(&text));
                    }
                    Some(tasks::changes_text(base, &result, &stat))
                }
                _ => None,
            };
            {
                // Serialize the final mailbox check with send_to_agent. Mail
                // accepted before this boundary must run before verification.
                let _integration = self.integrating.lock().expect("integration mutex");
                if self
                    .store
                    .pending_mail(conversation, &worker)?
                    .iter()
                    .any(|mail| mail.kind == "agent_work")
                {
                    task.verdict = None;
                    self.set_status(conversation, task, status::QUEUED)?;
                    return Ok(());
                }
                self.set_status(conversation, task, status::VERIFYING)?;
            }
            let setup = TurnSetup {
                session_policy: SessionPolicy::TaskRound {
                    task: task.id.clone(),
                    round: task.round,
                },
                conversation,
                plan_id: Some(task.plan_id.clone()),
                role: verifier.clone(),
                profile: self
                    .config_snapshot()
                    .resolve(spec.kind, RoleKind::Verifier),
                cwd: workdir.clone(),
                add_dirs: add_dirs.clone(),
                access: agent::Access::ReadOnly,
                upto_message: 0,
            };
            let user_checks = self.check_comments(conversation, &task.plan_id, &spec.id);
            let answer = self.turn(setup, cancel, |first| {
                let prompt = tasks::verifier_turn_prompt(
                    &project_text,
                    &spec,
                    if first { "" } else { &report },
                    changes.as_deref(),
                    first || round == 1,
                    if first { "" } else { &upstream },
                );
                format!("{user_checks}{prompt}")
            })?;
            let Some(answer) = answer else {
                return self.stop_task(conversation, task, &verifier, cancel);
            };
            let answer = self.post(
                conversation,
                Some(&task.plan_id),
                &format!("{}/verifier", spec.id),
                &answer,
            )?;
            if let Some(blocker) = blocker::parse(&answer) {
                let reason = blocker.summary();
                task.report = Some(format!("{report}\n\n{answer}\n\n[runtime] {reason}"));
                return self.stop(conversation, task, status::NEEDS_USER, Some(&reason));
            }
            let mut verdict = tasks::parse_verdict(&answer);
            let obligations = tasks::obligations(&spec);
            let missing = tasks::unaccounted(&verdict, &obligations);
            if !missing.is_empty() {
                // A verdict that skips an obligation does not count; the
                // verifier gets one turn to account for it.
                let prompt = tasks::coverage_followup(&missing);
                let setup = TurnSetup {
                    session_policy: SessionPolicy::TaskRound {
                        task: task.id.clone(),
                        round: task.round,
                    },
                    conversation,
                    plan_id: Some(task.plan_id.clone()),
                    role: verifier.clone(),
                    profile: self
                        .config_snapshot()
                        .resolve(spec.kind, RoleKind::Verifier),
                    cwd: workdir.clone(),
                    add_dirs: add_dirs.clone(),
                    access: agent::Access::ReadOnly,
                    upto_message: 0,
                };
                let Some(answer) = self.turn(setup, cancel, |fresh| {
                    if fresh {
                        let instructions = tasks::verifier_turn_prompt(
                            &project_text,
                            &spec,
                            "",
                            changes.as_deref(),
                            true,
                            "",
                        );
                        format!("{user_checks}{instructions}\n\n{prompt}")
                    } else {
                        prompt
                    }
                })?
                else {
                    return self.stop_task(conversation, task, &verifier, cancel);
                };
                let answer = self.post(
                    conversation,
                    Some(&task.plan_id),
                    &format!("{}/verifier", spec.id),
                    &answer,
                )?;
                let earlier = std::mem::replace(&mut verdict, tasks::parse_verdict(&answer));
                for (id, entry) in earlier.coverage {
                    verdict.coverage.entry(id).or_insert(entry);
                }
            }
            let incomplete = !tasks::unaccounted(&verdict, &obligations).is_empty();
            tasks::settle(&mut verdict, &obligations, !spec.writes);
            // The verdict has the last word on every check, with its evidence.
            for (id, _) in &obligations {
                if let Some(entry) = verdict.coverage.get(id.as_str()) {
                    let status = entry["status"].as_str().map(str::to_ascii_lowercase);
                    let note = tasks::bound(
                        entry["note"].as_str().unwrap_or_default(),
                        tasks::DETAIL_LIMIT,
                    );
                    self.record_check(
                        conversation,
                        task,
                        &tasks::CheckUpdate {
                            id: id.clone(),
                            status: status.filter(|status| {
                                matches!(status.as_str(), "ok" | "issue" | "blocked" | "n/a")
                            }),
                            note,
                        },
                    );
                }
            }
            let mut v_val = serde_json::to_value(&verdict)?;
            if let Some(obj) = v_val.as_object_mut() {
                obj.insert("round".into(), json!(task.round));
                if let Some(res) = &task.result {
                    obj.insert("result".into(), json!(res));
                }
            }
            task.verdict = Some(v_val);
            // The round's card data: what landed, and how every real
            // obligation was accounted for. The integration outcome is
            // added when the result lands (or is ruled out).
            let effect = match (&effect_text, spec.writes) {
                (Some(effect), _) => Some(effect.clone()),
                (None, false) => Some(tasks::report_effect(&report)),
                (None, true) => None,
            };
            let (effect, effect_cut) = tasks::effect_keys(&effect);
            task.feedback = Some(json!({
                "round": task.round,
                "effect": effect,
                "effect_cut": effect_cut,
                "coverage": tasks::coverage(&verdict, &obligations),
            }));
            if integration_repairs > 0 {
                task.feedback.as_mut().expect("round feedback")["integration_repairs"] =
                    json!(integration_repairs);
            }
            // Preserve this verdict before the next round clears mutable fields.
            self.store.update_task(task)?;
            if incomplete {
                // The worker cannot fix a verifier that skips its checks.
                return self.stop(
                    conversation,
                    task,
                    status::NEEDS_USER,
                    Some("the verifier's verdict still leaves obligations unaccounted for"),
                );
            }
            let worker_mail = self
                .store
                .pending_mail(conversation, &worker)?
                .iter()
                .any(|mail| mail.kind == "agent_work");
            let verifier_mail = self
                .store
                .pending_mail(conversation, &verifier)?
                .iter()
                .any(|mail| mail.kind == "agent_work");
            if worker_mail || verifier_mail {
                self.store.update_task(task)?;
                findings = verdict.findings;
                continue;
            }
            if verdict.approved {
                return match &worktree {
                    Some(path) => {
                        self.integrate(conversation, task, &spec, &project, path, &config, cancel)
                    }
                    None if in_place => {
                        self.finish_in_place(conversation, task, &spec, &project, &config)
                    }
                    None => {
                        let _integration = self.integrating.lock().expect("integration mutex");
                        if self.defer_agent_work(conversation, task)? {
                            return Ok(());
                        }
                        land(
                            task,
                            Outcome::NotApplicable,
                            "run",
                            "read-only task; nothing needs to land",
                            None,
                            None,
                            &[],
                        );
                        self.set_status(conversation, task, status::APPROVED)
                    }
                };
            }
            if task
                .report
                .as_deref()
                .is_some_and(tasks::environment_denied)
            {
                return self.stop(
                    conversation,
                    task,
                    status::NEEDS_USER,
                    Some(
                        "the environment denied an operation (Operation not permitted); \
                         more rounds cannot fix it. Change the permissions, then restart the agent",
                    ),
                );
            }
            let text = tasks::findings_key(&verdict.findings);
            if previous.as_deref() == Some(text.as_str()) {
                let report = task.report.take().unwrap_or_default();
                task.report = Some(format!(
                    "{report}\n\n[runtime] the verifier repeated the same findings; \
                     the worker is not getting closer"
                ));
                return self.stop(
                    conversation,
                    task,
                    status::NEEDS_USER,
                    Some(
                        "the verifier repeated the same findings; the worker is not getting closer",
                    ),
                );
            }
            previous = Some(text);
            findings = verdict.findings;
        }
        self.stop(
            conversation,
            task,
            status::NEEDS_USER,
            Some("the correction rounds that run without asking are over; the verifier has not approved"),
        )
    }
}
