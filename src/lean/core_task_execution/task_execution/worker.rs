use super::super::super::core_path_helpers::{task_path_component, worktree_branch, worktree_path};
use super::super::super::*;
use super::RoundState;
mod completion;
pub(super) use completion::commit_round;
pub(in crate::lean) use completion::complete_report;

pub(in crate::lean) fn writer_worktree(
    core: &Core,
    task: &mut Task,
    spec: &TaskContract,
    project: &Path,
) -> Result<Option<PathBuf>> {
    if !spec.writes {
        return Ok(None);
    }
    require(
        git::is_repository(project),
        "tasks that change files need the project to be a git repository: run `git init` and commit first",
    )?;
    let artifacts = if spec.delivery == Some(tasks::DeliveryMode::Untracked) {
        core.prepare_artifact_outputs(task, spec, project)?
    } else {
        Default::default()
    };
    if task
        .feedback
        .as_ref()
        .is_some_and(|f| f["continuation_review"] == true)
    {
        let onto = core.writer_base(&task.plan_id, spec, project)?;
        let combined = match (&task.base, &task.result) {
            (Some(base), Some(result)) => {
                combine_saved_checkpoint(core, task, project, &onto, base, result)?
            }
            _ => git::CombineResult::Applied(onto.clone()),
        };
        match combined {
            git::CombineResult::Applied(combined) => {
                task.base = Some(onto);
                task.result = Some(combined);
            }
            git::CombineResult::Conflicted(detail) => {
                // Keep the original delta so the existing integration repair
                // can reconcile it after the worker and reviewer assess it.
                task.feedback.as_mut().expect("continuation feedback")["checkpoint_merge"] =
                    json!({"onto":onto,"detail":detail});
            }
        }
    }
    let base = match &task.base {
        Some(base) => base.clone(),
        None => core.writer_base(&task.plan_id, spec, project)?,
    };
    let base = git::base_with_artifacts(project, &base, &artifacts)?;
    let path = worktree_path(&core.state, task)?;
    let start = task.result.clone().unwrap_or_else(|| base.clone());
    git::add_worktree(project, &path, &worktree_branch(&core.state, task)?, &start)?;
    task.base = Some(base);
    Ok(Some(path))
}

pub(super) fn task_directories(
    core: &Core,
    task: &Task,
    spec: &TaskContract,
    project: &Path,
    worktree: &Option<PathBuf>,
) -> Result<(PathBuf, Vec<PathBuf>)> {
    let observation = observation_worktree(core, task, spec, project, worktree)?;
    let workdir = worktree
        .clone()
        .or(observation)
        .unwrap_or_else(|| project.to_path_buf());
    Ok((workdir, Vec::new()))
}

fn observation_worktree(
    core: &Core,
    task: &Task,
    spec: &TaskContract,
    project: &Path,
    worktree: &Option<PathBuf>,
) -> Result<Option<PathBuf>> {
    if worktree.is_some() || !git::is_repository(project) {
        return Ok(None);
    }
    let onto = core.writer_base(&task.plan_id, spec, project)?;
    // A narrowed read-only continuation must inspect its own retained changes,
    // even when the unfinished writer's checkpoint has not landed yet.
    // Generated options run before their owner, but must inspect the owner's
    // retained implementation when choosing how to continue it.
    let owner = spec
        .options_for
        .as_deref()
        .map(|id| core.store.task_in_plan_by_id(&task.plan_id, id))
        .transpose()?
        .flatten();
    let source = owner
        .as_ref()
        .filter(|owner| owner.status != status::APPROVED)
        .unwrap_or(task);
    let base = match (&source.base, &source.result) {
        (Some(base), Some(result)) => {
            match combine_saved_checkpoint(core, task, project, &onto, base, result)? {
                git::CombineResult::Applied(combined) => combined,
                git::CombineResult::Conflicted(_) => result.clone(),
            }
        }
        _ => onto,
    };
    let path = core.state.join("worktrees").join(format!(
        "{}-observation-g{}",
        task_path_component(&task.id)?,
        task.input_generation
    ));
    let branch = format!(
        "antislop/{}-observation-g{}",
        task.id, task.input_generation
    );
    if path.exists()
        && git::tree_identity(project, &git::head(&path)?)? != git::tree_identity(project, &base)?
    {
        // Recreate clean read-only views through the existing pin/session-aware
        // cleanup. Never reset a writer or remove an attached/edited checkout.
        core.release_task_views(&task.id, None);
        require(
            !path.exists(),
            "the old observation checkout is still in use or has edits; detach its native terminal and retry",
        )?;
    }
    core.release_task_views(&task.id, Some(&path));
    git::add_worktree(project, &path, &branch, &base)?;
    Ok(Some(path))
}

fn combine_saved_checkpoint(
    core: &Core,
    task: &Task,
    project: &Path,
    onto: &str,
    base: &str,
    result: &str,
) -> Result<git::CombineResult> {
    let combined = git::combine_or_conflict(
        project,
        &core.state.join("worktrees").join(format!(
            "resume-{}-g{}",
            task_path_component(&task.id)?,
            task.input_generation
        )),
        onto,
        &[(base.to_owned(), result.to_owned())],
    )?;
    if let git::CombineResult::Conflicted(detail) = &combined {
        let conversation = core.store.plan_conversation(&task.plan_id)?;
        core.notice(&conversation, &format!(
            "Saved checkpoint kept for assessment; integration repair will reconcile its conflict with upstream changes: {detail}"
        ));
    }
    Ok(combined)
}

pub(super) fn is_recheck(
    core: &Core,
    conversation: &str,
    task: &Task,
    worker: &str,
    reviewer: &str,
) -> Result<bool> {
    if task.report.is_none() {
        return Ok(false);
    }
    // A blocker can preserve text/checkpoints without producing a completion.
    // A review hint must never turn that absence into a missing-record failure.
    let completed = match core.store.completed_task_output(task) {
        Ok(result) => result["type"] == "completed",
        Err(Error::Sql(rusqlite::Error::QueryReturnedNoRows)) => false,
        Err(error) => return Err(error),
    };
    if !completed {
        return Ok(false);
    }
    if task
        .feedback
        .as_ref()
        .is_some_and(|f| f["resume_role"] == "reviewer")
    {
        return Ok(true);
    }
    let reviewer_work = core
        .store
        .pending_mail(conversation, reviewer)?
        .iter()
        .any(|mail| mail.kind == "agent_work");
    let worker_empty = core.store.pending_mail(conversation, worker)?.is_empty();
    Ok(reviewer_work && worker_empty)
}

pub(super) struct WorkerRound<'a> {
    pub core: &'a Arc<Core>,
    pub conversation: &'a str,
    pub task: &'a Task,
    pub cancel: &'a AtomicBool,
    pub spec: &'a TaskContract,
    pub workdir: &'a Path,
    pub project_text: &'a str,
    pub add_dirs: &'a [PathBuf],
    pub upstream: &'a str,
    pub worker: &'a str,
    pub round: u32,
    pub state: &'a RoundState,
    pub recheck: bool,
}

pub(super) fn run(input: WorkerRound<'_>) -> Result<Option<String>> {
    if input.recheck {
        return Ok(Some(
            input
                .core
                .store
                .completed_task_output(input.task)?
                .to_string(),
        ));
    }
    run_worker_round(input)
}

fn run_worker_round(input: WorkerRound<'_>) -> Result<Option<String>> {
    let setup = worker_setup(&input);
    run_worker_turn(
        input.core,
        input.cancel,
        setup,
        input.project_text,
        input.spec,
        continuation(input.round, input.state),
        input.upstream,
    )
}

fn run_worker_turn(
    core: &Core,
    cancel: &AtomicBool,
    setup: TurnSetup<'_>,
    project_text: &str,
    spec: &TaskContract,
    continuation: Option<String>,
    upstream: &str,
) -> Result<Option<String>> {
    core.turn(setup, cancel, |_, _| {
        tasks::worker_turn_prompt(project_text, spec, continuation.as_deref(), upstream)
    })
}

fn worker_setup<'a>(input: &WorkerRound<'a>) -> TurnSetup<'a> {
    TurnSetup {
        session_policy: SessionPolicy::TaskRound {
            task: input.task.id.clone(),
            round: input.round,
        },
        conversation: input.conversation,
        plan_id: Some(input.task.plan_id.clone()),
        role: input.worker.to_owned(),
        profile: input
            .core
            .config_snapshot()
            .resolve(input.spec.kind, RoleKind::Executor),
        cwd: input.workdir.to_path_buf(),
        add_dirs: worker_dirs(input.spec, input.add_dirs),
        access: worker_access(input.spec),
        upto_message: 0,
    }
}

fn worker_dirs(spec: &TaskContract, add_dirs: &[PathBuf]) -> Vec<PathBuf> {
    if spec.writes {
        vec![]
    } else {
        add_dirs.to_vec()
    }
}

fn worker_access(spec: &TaskContract) -> agent::Access {
    if spec.writes || spec.kind == crate::config::WorkKind::Context {
        agent::Access::Write
    } else {
        agent::Access::ReadOnly
    }
}

fn continuation(round: u32, state: &RoundState) -> Option<String> {
    if round == 1 {
        None
    } else if state.resumed && round == state.first_round {
        Some(tasks::worker_resume(&state.findings))
    } else {
        Some(tasks::worker_followup(&state.findings))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_continuation_observes_retained_checkpoint_and_new_project_inputs() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir(&project).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(&project)
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(project.join("instrument.txt"), "old instrument").unwrap();
        let base = git::commit_all(&project, "base").unwrap();
        let checkpoint = directory.path().join("checkpoint");
        git::add_worktree(&project, &checkpoint, "antislop/checkpoint", &base).unwrap();
        std::fs::write(checkpoint.join("instrument.txt"), "repaired instrument").unwrap();
        let result = git::commit_all(&checkpoint, "repair").unwrap();
        std::fs::write(project.join("user.txt"), "new local input").unwrap();
        let config = Config::from_toml(include_str!("../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&directory.path().join("state"), config).unwrap();
        let conversation = core
            .store
            .create_conversation(project.to_str().unwrap())
            .unwrap();
        let spec: TaskContract = serde_json::from_value(json!({
            "id":"accept-repair", "title":"Accept repair", "goal":"Inspect the existing repair",
            "kind":"analysis", "writes":false
        }))
        .unwrap();
        let mut task = core
            .store
            .add_plan(&conversation.id, &[serde_json::to_value(&spec).unwrap()])
            .unwrap()
            .remove(0);
        task.base = Some(base.clone());
        task.result = Some(result);
        task.feedback = Some(json!({"continuation_review":true}));
        let (view, dirs) = task_directories(&core, &task, &spec, &project, &None).unwrap();
        assert!(dirs.is_empty());
        assert_ne!(view, project);
        assert_eq!(
            std::fs::read_to_string(view.join("instrument.txt")).unwrap(),
            "repaired instrument"
        );
        assert_eq!(
            std::fs::read_to_string(view.join("user.txt")).unwrap(),
            "new local input"
        );
        assert_eq!(
            std::fs::read_to_string(project.join("instrument.txt")).unwrap(),
            "old instrument"
        );
        assert_eq!(git::head(&project).unwrap(), base);
        let owner_spec: TaskContract = serde_json::from_value(json!({
            "id":"recover", "title":"Recover", "goal":"Continue the saved repair",
            "kind":"implementation", "writes":true
        }))
        .unwrap();
        let options_spec: TaskContract = serde_json::from_value(json!({
            "id":"recover-options", "title":"Options", "goal":"Assess recovery",
            "kind":"options", "options_for":"recover"
        }))
        .unwrap();
        let mut tasks = core
            .store
            .add_plan(
                &conversation.id,
                &[
                    serde_json::to_value(&owner_spec).unwrap(),
                    serde_json::to_value(&options_spec).unwrap(),
                ],
            )
            .unwrap();
        let (stale_view, _) =
            task_directories(&core, &tasks[1], &options_spec, &project, &None).unwrap();
        assert_eq!(
            std::fs::read_to_string(stale_view.join("instrument.txt")).unwrap(),
            "old instrument"
        );
        tasks[0].base = task.base.clone();
        tasks[0].result = task.result.clone();
        tasks[0].status = status::QUEUED.into();
        core.store.update_task(&tasks[0]).unwrap();
        let (options_view, _) =
            task_directories(&core, &tasks[1], &options_spec, &project, &None).unwrap();
        assert_eq!(
            std::fs::read_to_string(options_view.join("instrument.txt")).unwrap(),
            "repaired instrument"
        );
        assert_eq!(
            std::fs::read_to_string(project.join("instrument.txt")).unwrap(),
            "old instrument"
        );
        std::fs::write(project.join("instrument.txt"), "reviewed upstream change").unwrap();
        let upstream = git::commit_all(&project, "upstream edit").unwrap();
        core.record_plan_landing(&tasks[0].plan_id, "upstream", &upstream)
            .unwrap();
        let (conflict_view, _) =
            task_directories(&core, &tasks[1], &options_spec, &project, &None).unwrap();
        assert_eq!(
            std::fs::read_to_string(conflict_view.join("instrument.txt")).unwrap(),
            "repaired instrument"
        );
        tasks[0].feedback = Some(json!({"continuation_review":true}));
        let saved_base = tasks[0].base.clone();
        let saved_result = tasks[0].result.clone();
        let writer = writer_worktree(&core, &mut tasks[0], &owner_spec, &project)
            .unwrap()
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(writer.join("instrument.txt")).unwrap(),
            "repaired instrument"
        );
        assert_eq!(tasks[0].base, saved_base);
        assert_eq!(tasks[0].result, saved_result);
        assert!(
            tasks[0].feedback.as_ref().unwrap()["checkpoint_merge"]["detail"]
                .as_str()
                .unwrap()
                .contains("does not apply")
        );
        assert_eq!(
            std::fs::read_to_string(project.join("instrument.txt")).unwrap(),
            "reviewed upstream change"
        );
        assert_eq!(git::head(&project).unwrap(), upstream);
    }

    #[test]
    fn blocked_checkpoint_is_not_a_completed_result_for_review_only_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!("../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&dir.path().join("state"), config).unwrap();
        let conversation = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        let mut task=core.store.add_plan(&conversation,&[json!({"id":"work","title":"Work","goal":"Inspect saved code","kind":"analysis"})]).unwrap().remove(0);
        let worker = format!("worker:{}", task.id);
        let reviewer = format!("reviewer:{}", task.id);
        let turn = core
            .store
            .start_turn(&conversation, &worker, "fixture", 0, Some(&task.plan_id))
            .unwrap();
        core.store
            .save_turn_result(
                &turn.id,
                &crate::lean::turn_result::TurnResult::Blocked {
                    report: "Checkpoint preserved, build unavailable".into(),
                    resume_condition: "Runtime repaired".into(),
                },
            )
            .unwrap();
        core.store
            .finish_turn(&turn.id, store::TurnStatus::Done, None)
            .unwrap();
        task.report = Some("Checkpoint preserved, build unavailable".into());
        task.feedback = Some(json!({"resume_role":"reviewer"}));
        core.store.update_task(&task).unwrap();
        assert!(!is_recheck(&core, &conversation, &task, &worker, &reviewer).unwrap());
        task.round = 1;
        core.store.update_task(&task).unwrap();
        let turn = core
            .store
            .start_turn(&conversation, &worker, "fixture", 0, Some(&task.plan_id))
            .unwrap();
        core.store
            .save_turn_result(
                &turn.id,
                &crate::lean::turn_result::TurnResult::Completed {
                    summary: "Inspected".into(),
                    report: "Actual completed inspection".into(),
                    selection: None,
                },
            )
            .unwrap();
        core.store
            .finish_turn(&turn.id, store::TurnStatus::Done, None)
            .unwrap();
        task.report = Some("Actual completed inspection".into());
        core.store.update_task(&task).unwrap();
        assert!(is_recheck(&core, &conversation, &task, &worker, &reviewer).unwrap());
    }
}
