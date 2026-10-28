use super::core_feedback_helpers::land;
use super::core_path_helpers::{task_path_component, worktree_branch, worktree_path};
use super::*;

/// Worker and verifier rounds one run of a task may take before it is handed back.
const MAX_ROUNDS_PER_RUN: u32 = 16;

impl Core {
    /// Worker and verifier continue while verified obligations close.
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
        let spec: TaskContract = serde_json::from_value(task.spec.clone())?;
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        let worktree = if spec.writes {
            require(
                git::is_repository(&project),
                "tasks that change files need the project to be a git repository: run `git init` and commit first",
            )?;
            let base = match &task.base {
                Some(base) => base.clone(),
                None => self.writer_base(&task.plan_id, &spec, &project)?,
            };
            let path = worktree_path(&self.state, task)?;
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
        let observation = if worktree.is_none() && git::is_repository(&project) {
            let base = self.writer_base(&task.plan_id, &spec, &project)?;
            let path = self.state.join("worktrees").join(format!(
                "{}-observation-g{}",
                task_path_component(&task.id)?,
                task.input_generation
            ));
            git::add_worktree(
                &project,
                &path,
                &format!(
                    "antislop/{}-observation-g{}",
                    task.id, task.input_generation
                ),
                &base,
            )?;
            Some(path)
        } else {
            None
        };
        let workdir = worktree
            .clone()
            .or(observation)
            .unwrap_or_else(|| project.clone());
        let project_text = workdir.to_string_lossy().into_owned();
        // A context task maps the project by running its own checkers, so it
        // gets a shell in its throwaway observation checkout. It still has no
        // writer worktree and nothing it leaves there is ever integrated.
        let access = if spec.writes || spec.kind == crate::config::WorkKind::Context {
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
            && let Some(path) = &worktree
        {
            return self.integrate(conversation, task, &spec, &project, path, &config, cancel);
        }
        let mut findings = verdict.map(|verdict| verdict.findings).unwrap_or_default();
        let mut dependents_invalidated = self
            .store
            .pending_mail(conversation, &worker)?
            .iter()
            .any(|mail| mail.kind == "agent_work");
        let resumed = task.round > 0;
        let first_round = task.round + 1;
        let mut progress = super::progress::Progress::default();
        for round in first_round.. {
            // Finding ids are the measure of progress, but a verifier that gives
            // every round's findings new ids always looks like progress. After
            // this many rounds in one run the task goes back to the orchestrator,
            // which can narrow the contract, answer, or ask the user; running it
            // again starts a new count.
            if round - first_round >= MAX_ROUNDS_PER_RUN {
                let why = format!(
                    "{MAX_ROUNDS_PER_RUN} correction rounds ran without approval; the orchestrator must narrow the contract, supply a new basis or ask the user"
                );
                return self.stop(conversation, task, status::COORDINATING, Some(&why));
            }
            let config = self.config_snapshot();
            task.round = round;
            task.verdict = None;
            // A verifier-only request checks the same result; a worker round
            // begins without exposing the preceding report as its new result.
            let recheck = task.report.is_some()
                && (task
                    .feedback
                    .as_ref()
                    .is_some_and(|f| f["resume_role"] == "verifier")
                    || (self
                        .store
                        .pending_mail(conversation, &verifier)?
                        .iter()
                        .any(|mail| mail.kind == "agent_work")
                        && self.store.pending_mail(conversation, &worker)?.is_empty()));
            if !recheck && resumed && !dependents_invalidated {
                self.invalidate_task_dependents(conversation, task)?;
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
                Some(self.store.completed_task_output(task)?.to_string())
            } else {
                self.turn(worker_setup(), cancel, |_| {
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
                        continuation.as_deref(),
                        &upstream,
                    )
                })?
            };
            let Some(report) = report else {
                return self.stop_task(conversation, task, &worker, cancel);
            };

            task.report = Some(report.clone());
            self.store.update_task(task)?;
            let completion: turn_result::TurnResult = serde_json::from_str(&report)?;
            let (report, summary) = match completion {
                turn_result::TurnResult::Completed {
                    report,
                    summary,
                    selection,
                } => (
                    turn_result::report_with_deliverables(&report, selection.as_ref()),
                    summary,
                ),
                turn_result::TurnResult::NeedsInput {
                    request,
                    checkpoint,
                } => {
                    if let Some(path) = &worktree {
                        task.result = Some(git::commit_all(
                            path,
                            &format!("{} checkpoint", spec.title),
                        )?);
                    }
                    task.report = Some(checkpoint);
                    return self.wait_for_question(conversation, task, &request, false);
                }
                _ => return Err(Error::Invalid("worker did not submit a completion".into())),
            };
            task.report = Some(report.clone());
            let mut effect_text = Some(tasks::bound_cut(&summary, tasks::EFFECT_LIMIT));
            let changes = match (&worktree, &task.base) {
                (Some(path), Some(base)) => {
                    let result = git::commit_all(path, &format!("{} (round {round})", spec.title))?;
                    task.result = Some(result.clone());
                    let stat = git::diff_stat(path, base, &result)?;
                    let files = git::changed_files(path, base, &result)?;
                    effect_text.get_or_insert_with(|| tasks::effect(&stat, &files));
                    Some(tasks::changes_text(base, &result, &stat))
                }
                _ => None,
            };
            self.set_status(conversation, task, status::VERIFYING)?;
            let review_commit = task.result.clone().or_else(|| git::head(&workdir).ok());
            let reviewdir = if let Some(result) = &review_commit {
                let path = self.state.join("worktrees").join(format!(
                    "{}-review-g{}-{round}",
                    task_path_component(&task.id)?,
                    task.input_generation
                ));
                git::add_worktree(
                    &project,
                    &path,
                    &format!(
                        "antislop/{}-review-g{}-{round}",
                        task.id, task.input_generation
                    ),
                    result,
                )?;
                path
            } else {
                workdir.clone()
            };
            let verifier_project = reviewdir.to_string_lossy().into_owned();
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
                cwd: reviewdir,
                add_dirs: add_dirs.clone(),
                access: agent::Access::ReadOnly,
                upto_message: 0,
            };
            let user_checks = self.check_comments(conversation, &task.plan_id, &spec.id);
            let answer = self.turn(setup, cancel, |first| {
                let prompt = tasks::verifier_turn_prompt(
                    &verifier_project,
                    &spec,
                    &report,
                    changes.as_deref(),
                    first || round == 1,
                    &upstream,
                );
                format!("{user_checks}{prompt}")
            })?;
            let Some(answer) = answer else {
                return self.stop_task(conversation, task, &verifier, cancel);
            };

            let completion: turn_result::TurnResult = serde_json::from_str(&answer)?;
            let mut verdict = match completion {
                turn_result::TurnResult::Review { verdict, .. } => verdict,
                turn_result::TurnResult::NeedsInput {
                    request,
                    checkpoint,
                } => {
                    task.report = Some(report + "\n" + &checkpoint);
                    return self.wait_for_question(conversation, task, &request, true);
                }
                _ => return Err(Error::Invalid("verifier did not submit a review".into())),
            };
            let obligations = tasks::obligations(&spec);
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
            if verdict.approved {
                return match &worktree {
                    Some(path) => {
                        self.integrate(conversation, task, &spec, &project, path, &config, cancel)
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
            if progress.record(super::progress::open_obligations(&verdict)) {
                let why = "no previously open obligation was resolved; the orchestrator must supply a new basis or revise the contract";
                let report = task.report.take().unwrap_or_default();
                task.report = Some(format!("{report}\n\n[runtime] {why}"));
                return self.stop(conversation, task, status::COORDINATING, Some(why));
            }
            findings = verdict.findings;
        }
        self.stop(
            conversation,
            task,
            status::NEEDS_USER,
            Some("task attempt counter exhausted"),
        )
    }
}
