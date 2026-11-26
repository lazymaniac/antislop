use super::RoundState;
use crate::lean::core::worktree::paths::{task_path_component, worktree_branch, worktree_path};
use crate::lean::*;
mod completion;
pub(in crate::lean) use completion::commit_round;
pub(in crate::lean) use completion::complete_report;

pub(in crate::lean) fn writer_worktree(
    core: &Core,
    task: &mut Task,
    spec: &TaskContract,
    project: &Path,
) -> Result<Option<PathBuf>> {
    if !spec.change_kind.has_delivery() {
        return Ok(None);
    }
    require(
        git::is_repository(project),
        "tasks that change files need the project to be a git repository: run `git init` and commit first",
    )?;
    if spec.integration_batch {
        let base = core.initial_plan_anchor(&task.plan_id, project)?;
        let path = worktree_path(&core.state, task)?;
        let start = task.result.clone().unwrap_or_else(|| base.clone());
        let branch = worktree_branch(&core.state, task)?;
        if let Ok(tip) = git::ok(
            project,
            &["rev-parse", "--verify", &format!("refs/heads/{branch}")],
        ) && tip != start
            && git::tree_identity(project, &tip).ok() == git::tree_identity(project, &start).ok()
        {
            git::add_worktree(project, &path, &branch, &tip)?;
        } else {
            git::add_worktree(project, &path, &branch, &start)?;
        }
        task.base = Some(base);
        return Ok(Some(path));
    }
    let artifacts = if spec.delivery == Some(tasks::DeliveryMode::Untracked) {
        core.prepare_artifact_outputs(task, spec, project)?
    } else {
        Default::default()
    };
    let continuation = (spec.change_kind() == tasks::ChangeKind::Artifact && task.base.is_some())
        || task
            .feedback
            .as_ref()
            .is_some_and(|f| f["continuation_review"] == true);
    let base = if continuation || task.base.is_none() {
        core.writer_base(&task.plan_id, spec, project)?
    } else {
        task.base.clone().expect("checked task base")
    };
    let base = git::base_with_artifacts(project, &base, &artifacts)?;
    let path = worktree_path(&core.state, task)?;
    if continuation {
        // A retired checkout leaves its branch intact. Reattach it before
        // reconciling, since add_worktree deliberately preserves that tip.
        let previous = task.result.as_deref().or(task.base.as_deref());
        if let Some(previous) = previous {
            let branch = worktree_branch(&core.state, task)?;
            git::add_worktree(project, &path, &branch, previous)?;
        }
        let combined = match (&task.base, &task.result) {
            (Some(checkpoint_base), Some(result)) => {
                combine_saved_checkpoint(core, task, spec, project, &base, checkpoint_base, result)?
            }
            _ => git::CombineResult::Applied(base.clone()),
        };
        match combined {
            git::CombineResult::Applied(combined) => {
                if let Some(result) = previous
                    && path.exists()
                    && let Err(detail) = apply_checkpoint_delta(project, &path, result, &combined)?
                {
                    task.feedback.get_or_insert_with(|| json!({}))["checkpoint_merge"] =
                        json!({"onto":base,"detail":detail});
                    core.store.update_task(task)?;
                    return Ok(Some(path));
                }
                if let Some(fields) = task
                    .feedback
                    .as_mut()
                    .and_then(serde_json::Value::as_object_mut)
                {
                    fields.remove("checkpoint_merge");
                }
                task.base = Some(base.clone());
                task.result = Some(combined);
                // Make the checkpoint pair durable before preparation is
                // reported complete. If this write fails, the old DB pair
                // remains authoritative and the next attempt recognizes the
                // already moved clean checkout by tree identity above.
                core.store.update_task(task)?;
            }
            git::CombineResult::Conflicted(detail) => {
                // Keep the original delta so the existing integration repair
                // can reconcile it after the worker and reviewer assess it.
                task.feedback.get_or_insert_with(|| json!({}))["checkpoint_merge"] =
                    json!({"onto":base,"detail":detail});
                core.store.update_task(task)?;
                return Ok(Some(path));
            }
        }
    }
    let mut start = task.result.clone().unwrap_or_else(|| base.clone());
    let branch = worktree_branch(&core.state, task)?;
    // A preserved branch tip may be a different commit with the same content
    // as the recorded result; reattach at the tip instead of resetting it.
    if let Ok(tip) = git::ok(
        project,
        &["rev-parse", "--verify", &format!("refs/heads/{branch}")],
    ) && tip != start
        && git::tree_identity(project, &tip).ok() == git::tree_identity(project, &start).ok()
    {
        start = tip;
    }
    git::add_worktree(project, &path, &branch, &start)?;
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
    let workdir = if let Some(writer) = worktree {
        writer.clone()
    } else if let Some(owner_id) = spec.options_for.as_deref() {
        let mut owner = core
            .store
            .task_in_plan_by_id(&task.plan_id, owner_id)?
            .ok_or_else(|| {
                Error::Invalid(format!(
                    "options task {} has no owner task {owner_id}",
                    task.id
                ))
            })?;
        let owner_spec: TaskContract = serde_json::from_value(owner.spec.clone())?;
        let owner_worktree =
            writer_worktree(core, &mut owner, &owner_spec, project)?.ok_or_else(|| {
                Error::Invalid("options task owner does not have a writer worktree".into())
            })?;
        core.store.update_task(&owner)?;
        owner_worktree
    } else if core.plan_has_integrated_results(&task.plan_id)? {
        core.session_worktree(&core.store.plan_conversation(&task.plan_id)?, project)?
    } else {
        project.to_path_buf()
    };
    Ok((workdir, Vec::new()))
}

fn combine_saved_checkpoint(
    core: &Core,
    task: &Task,
    spec: &TaskContract,
    project: &Path,
    onto: &str,
    base: &str,
    result: &str,
) -> Result<git::CombineResult> {
    // Artifact pairs delimit the author's delta even when composition made
    // base and result siblings. An ancestry boundary would replay old inputs.
    let checkpoint_base = if spec.change_kind() == tasks::ChangeKind::Artifact {
        artifact_checkpoint_base(core, task, spec, project, base, result)?
    } else {
        git::common_base(project, base, result)?
    };
    let combined = git::combine_checkpoint_or_conflict(
        project,
        &core.state.join("worktrees").join(format!(
            "resume-{}-g{}",
            task_path_component(&task.id)?,
            task.input_generation
        )),
        onto,
        &checkpoint_base,
        result,
        &spec.outputs,
    )?;
    if let git::CombineResult::Conflicted(detail) = &combined {
        let conversation = core.store.plan_conversation(&task.plan_id)?;
        core.notice(&conversation, &format!(
            "Saved checkpoint kept for assessment; integration repair will reconcile its conflict with upstream changes: {detail}"
        ));
    }
    Ok(combined)
}

fn artifact_checkpoint_base(
    core: &Core,
    task: &Task,
    spec: &TaskContract,
    project: &Path,
    base: &str,
    result: &str,
) -> Result<String> {
    let invalid = match spec.validate_change_paths(&git::changed_files(project, base, result)?) {
        Ok(()) => return Ok(base.to_owned()),
        Err(error) => error,
    };
    let mut recovered: Option<(String, String)> = None;
    // Older runtimes could advance only base. Recover from this task's saved
    // round pairs, never from a convenient Git parent or an outputs filter.
    for summary in core.store.task_rounds(&task.id)? {
        let round: u32 = serde_json::from_value(summary["round"].clone())?;
        if round >= task.round || summary["history_unavailable"] == true {
            continue;
        }
        let record = core
            .store
            .task_round_with_references(&task.id, round, true)?;
        if record["id"].as_str() != Some(task.id.as_str())
            || !record["input_generation"]
                .as_i64()
                .is_some_and(|generation| generation <= task.input_generation)
        {
            continue;
        }
        let (Some(saved_base), Some(saved_result)) =
            (record["base"].as_str(), record["result"].as_str())
        else {
            continue;
        };
        if git::common_base(project, saved_base, saved_result)? != saved_base
            || git::common_base(project, saved_result, result)? != saved_result
            || spec
                .validate_change_paths(&git::changed_files(project, saved_base, saved_result)?)
                .is_err()
            || spec
                .validate_change_paths(&git::changed_files(project, saved_base, result)?)
                .is_err()
        {
            continue;
        }
        let tree = git::tree_identity(project, saved_base)?;
        if let Some((_, previous_tree)) = &recovered {
            require(
                previous_tree == &tree,
                &format!(
                    "{invalid}; ambiguous saved artifact checkpoint bases for task {}",
                    task.id
                ),
            )?;
        }
        recovered = Some((saved_base.to_owned(), tree));
    }
    let (recovered, _) = recovered.ok_or_else(|| {
        Error::Invalid(format!(
            "{invalid}; no proven saved artifact checkpoint base for task {}",
            task.id
        ))
    })?;
    // Preserve the inconsistent pair and its proven boundary before any
    // checkout or task-row update can replace them. This records evidence,
    // not a claim that preparation or delivery has finished.
    core.store.append_event(
        &core.store.plan_conversation(&task.plan_id)?,
        None,
        Some(&task.plan_id),
        "artifact_checkpoint_recovery",
        &json!({"task":task.id,"input_generation":task.input_generation,
            "base":base,"result":result,"recovered_base":recovered}),
    )?;
    Ok(recovered)
}

fn apply_checkpoint_delta(
    project: &Path,
    worktree: &Path,
    old_result: &str,
    combined: &str,
) -> Result<std::result::Result<(), String>> {
    let head = git::head(worktree)?;
    let old_tree = git::tree_identity(project, old_result)?;
    let head_tree = git::tree_identity(project, &head)?;
    let combined_tree = git::tree_identity(project, combined)?;
    let patch = git::patch(project, old_result, Some(combined))?;
    // `git apply` updates the worktree before `reset --mixed` moves HEAD and
    // the index. After a crash in that interval, recover only if the checkout
    // still starts at the saved result and its complete patch is exactly the
    // expected checkpoint delta. A distinct local edit therefore remains a
    // conflict instead of being absorbed into the new result.
    if head_tree == old_tree
        && checkpoint_delta_is_applied(worktree, project, old_result, combined, &patch)?
    {
        git::ok(worktree, &["reset", "--mixed", combined])?;
        return Ok(Ok(()));
    }
    if !git::is_clean(worktree) {
        return Ok(Err("preserved writer checkout has local changes".into()));
    }
    // Preparation can also be interrupted after reset but before the caller
    // persists the new task.base/result pair. Recognize that completed tree
    // without applying the checkpoint delta twice.
    if head_tree == combined_tree {
        git::ok(worktree, &["reset", "--mixed", combined])?;
        return Ok(Ok(()));
    }
    if head_tree != old_tree {
        return Ok(Err(
            "preserved writer branch differs from the saved checkpoint".into(),
        ));
    }
    if !patch.trim().is_empty() {
        let file = worktree.with_extension("checkpoint.patch");
        std::fs::write(&file, format!("{patch}\n"))?;
        let path = file.to_string_lossy();
        let checked = git::git(worktree, &["apply", "--check", &path])?;
        if !checked.status.success() {
            let _ = std::fs::remove_file(&file);
            return Ok(Err(String::from_utf8_lossy(&checked.stderr)
                .trim()
                .to_owned()));
        }
        let applied = git::git(worktree, &["apply", &path]);
        let _ = std::fs::remove_file(&file);
        let applied = applied?;
        if !applied.status.success() {
            return Ok(Err(String::from_utf8_lossy(&applied.stderr)
                .trim()
                .to_owned()));
        }
    }
    // Move the branch and index without rewriting files, so edits racing
    // preparation remain visible as local changes for the next pass.
    git::ok(worktree, &["reset", "--mixed", combined])?;
    Ok(Ok(()))
}

fn checkpoint_delta_is_applied(
    worktree: &Path,
    project: &Path,
    old_result: &str,
    combined: &str,
    patch: &str,
) -> Result<bool> {
    if patch.trim().is_empty() {
        return Ok(false);
    }
    let expected: std::collections::BTreeSet<_> =
        git::changed_files(project, old_result, combined)?
            .into_iter()
            .collect();
    let status = git::git(
        worktree,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    if !status.status.success() {
        return Err(Error::Invalid(format!(
            "git status: {}",
            String::from_utf8_lossy(&status.stderr).trim()
        )));
    }
    let mut actual = std::collections::BTreeSet::new();
    for entry in status
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
    {
        if entry.len() < 4 || !((entry[0] == b' ' && entry[1] != b' ') || entry[..2] == *b"??") {
            return Ok(false);
        }
        let Ok(path) = std::str::from_utf8(&entry[3..]) else {
            return Ok(false);
        };
        actual.insert(path.to_owned());
    }
    if actual != expected {
        return Ok(false);
    }
    let file = worktree.with_extension("checkpoint-recovery.patch");
    std::fs::write(&file, patch)?;
    let check = git::git(
        worktree,
        &[
            "apply",
            "--reverse",
            "--check",
            file.to_str()
                .ok_or_else(|| Error::Invalid("checkpoint patch path is not valid UTF-8".into()))?,
        ],
    )?;
    std::fs::remove_file(file)?;
    Ok(check.status.success())
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
    core.turn(setup, cancel, |_, inputs| {
        let spec = inputs.task_contract(spec, crate::lean::agent_tools::turn_inputs::ReadFor::Work);
        tasks::worker_turn_prompt(project_text, &spec, continuation.as_deref(), upstream)
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
        upto_message: 0,
    }
}

fn worker_dirs(spec: &TaskContract, add_dirs: &[PathBuf]) -> Vec<PathBuf> {
    if spec.change_kind.has_delivery() {
        vec![]
    } else {
        add_dirs.to_vec()
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
    fn restart_after_checkpoint_branch_move_recovers_the_durable_task_pair() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir(&project).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .current_dir(&project)
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(project.join("report.txt"), "original\n").unwrap();
        let base = git::commit_all(&project, git::CommitPurpose::TestFixture).unwrap();
        let producer_tree = directory.path().join("producer");
        git::add_worktree(&project, &producer_tree, "antislop/test-producer", &base).unwrap();
        std::fs::write(producer_tree.join("producer.txt"), "approved dependency\n").unwrap();
        let producer_result =
            git::commit_all(&producer_tree, git::CommitPurpose::TestFixture).unwrap();
        let checkpoint_tree = directory.path().join("checkpoint");
        git::add_worktree(
            &project,
            &checkpoint_tree,
            "antislop/test-checkpoint",
            &base,
        )
        .unwrap();
        std::fs::write(checkpoint_tree.join("report.txt"), "saved report\n").unwrap();
        let checkpoint =
            git::commit_all(&checkpoint_tree, git::CommitPurpose::TestFixture).unwrap();

        let state = directory.path().join("state");
        let config =
            Config::from_toml(include_str!("../../../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&state, config.clone()).unwrap();
        let conversation = core.start(&project).unwrap();
        let producer_spec: TaskContract = serde_json::from_value(json!({
            "id":"producer", "title":"Producer", "acceptance_criteria":[{"id":"C1","text":"The dependency is approved"}],"goal":"Produce dependency",
            "change_kind":"code", "kind":"implementation"
        })).unwrap();
        let report_spec: TaskContract = serde_json::from_value(json!({
            "id":"report", "title":"Report", "acceptance_criteria":[{"id":"C1","text":"The report is preserved"}],"goal":"Continue report",
            "change_kind":"code", "kind":"implementation", "depends_on":["producer"]
        })).unwrap();
        let mut tasks = core
            .store
            .add_plan(
                &conversation.id,
                &[
                    serde_json::to_value(&producer_spec).unwrap(),
                    serde_json::to_value(&report_spec).unwrap(),
                ],
            )
            .unwrap();
        tasks[0].base = Some(base.clone());
        tasks[0].result = Some(producer_result);
        tasks[0].status = status::APPROVED.into();
        core.store.update_task(&tasks[0]).unwrap();
        tasks[1].base = Some(base.clone());
        tasks[1].result = Some(checkpoint.clone());
        tasks[1].feedback = Some(json!({"continuation_review":true}));
        tasks[1].status = status::QUEUED.into();
        core.store.update_task(&tasks[1]).unwrap();
        let writer = worktree_path(&core.state, &tasks[1]).unwrap();
        let branch = worktree_branch(&core.state, &tasks[1]).unwrap();
        git::add_worktree(&project, &writer, &branch, &checkpoint).unwrap();

        // Model termination after git apply but before reset --mixed and
        // update_task: the files are changed, while HEAD and the index remain
        // at the old checkpoint.
        let onto = core
            .writer_base(&tasks[1].plan_id, &report_spec, &project)
            .unwrap();
        let combined = match combine_saved_checkpoint(
            &core,
            &tasks[1],
            &report_spec,
            &project,
            &onto,
            &base,
            &checkpoint,
        )
        .unwrap()
        {
            git::CombineResult::Applied(commit) => commit,
            git::CombineResult::Conflicted(detail) => panic!("unexpected conflict: {detail}"),
        };
        let patch = git::patch(&project, &checkpoint, Some(&combined)).unwrap();
        let patch_file = writer.with_extension("interrupted-checkpoint.patch");
        std::fs::write(&patch_file, patch).unwrap();
        let applied = git::git(&writer, &["apply", patch_file.to_str().unwrap()]).unwrap();
        assert!(applied.status.success());
        std::fs::remove_file(patch_file).unwrap();
        assert!(!git::is_clean(&writer));
        assert_eq!(git::head(&writer).unwrap(), checkpoint);
        let saved = core
            .store
            .task_in_plan_by_id(&tasks[1].plan_id, "report")
            .unwrap()
            .unwrap();
        assert_eq!(saved.base.as_deref(), Some(base.as_str()));
        assert_eq!(saved.result.as_deref(), Some(checkpoint.as_str()));
        drop(core);

        let core = Core::open(&state, config).unwrap();
        let mut resumed = core
            .store
            .task_in_plan_by_id(&tasks[1].plan_id, "report")
            .unwrap()
            .unwrap();
        let resumed_path = writer_worktree(&core, &mut resumed, &report_spec, &project)
            .unwrap()
            .unwrap();
        assert_eq!(resumed_path, writer);
        assert_eq!(
            git::tree_identity(&project, resumed.base.as_deref().unwrap()).unwrap(),
            git::tree_identity(&project, &onto).unwrap()
        );
        let persisted = core
            .store
            .task_in_plan_by_id(&resumed.plan_id, "report")
            .unwrap()
            .unwrap();
        assert_eq!(
            git::tree_identity(&project, persisted.base.as_deref().unwrap()).unwrap(),
            git::tree_identity(&project, &onto).unwrap()
        );
        assert_eq!(
            git::tree_identity(&project, persisted.result.as_deref().unwrap()).unwrap(),
            git::tree_identity(&project, &combined).unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(writer.join("report.txt")).unwrap(),
            "saved report\n"
        );
        assert_eq!(
            std::fs::read_to_string(writer.join("producer.txt")).unwrap(),
            "approved dependency\n"
        );
        assert!(
            resumed
                .feedback
                .as_ref()
                .unwrap()
                .get("checkpoint_merge")
                .is_none()
        );
    }

    #[test]
    fn read_only_work_before_integration_uses_project_and_options_share_owner_checkout() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir(&project).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .current_dir(&project)
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(project.join("instrument.txt"), "old instrument").unwrap();
        let base = git::commit_all(&project, git::CommitPurpose::TestFixture).unwrap();
        let checkpoint = directory.path().join("checkpoint");
        git::add_worktree(&project, &checkpoint, "antislop/checkpoint", &base).unwrap();
        std::fs::write(checkpoint.join("instrument.txt"), "repaired instrument").unwrap();
        let result = git::commit_all(&checkpoint, git::CommitPurpose::TestFixture).unwrap();
        std::fs::write(project.join("user.txt"), "new local input").unwrap();
        let config =
            Config::from_toml(include_str!("../../../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&directory.path().join("state"), config).unwrap();
        let conversation = core.start(&project).unwrap();
        let spec: TaskContract = serde_json::from_value(json!({
            "id":"accept-repair", "title":"Accept repair", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Inspect the existing repair",
            "change_kind": "none", "kind":"analysis"}))
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
        assert_eq!(view, project);
        assert!(!worktree_path(&core.state, &task).unwrap().exists());
        assert_eq!(
            std::fs::read_to_string(project.join("user.txt")).unwrap(),
            "new local input"
        );
        assert_eq!(
            std::fs::read_to_string(project.join("instrument.txt")).unwrap(),
            "old instrument",
            "pre-integration read-only work uses the project rather than a private checkpoint"
        );
        assert_eq!(git::head(&project).unwrap(), base);
        let owner_spec: TaskContract = serde_json::from_value(json!({
            "id":"recover", "title":"Recover", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Continue the saved repair",
            "change_kind": "code", "kind":"implementation", "checkout_name":"same-label"}))
        .unwrap();
        let options_spec: TaskContract = serde_json::from_value(json!({
            "id":"recover-options", "title":"Options", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Assess recovery",
            "change_kind": "none", "kind":"options", "options_for":"recover"
        }))
        .unwrap();
        let isolated_spec: TaskContract = serde_json::from_value(json!({
            "id":"isolated", "title":"Isolated", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Keep another task isolated",
            "change_kind": "code", "kind":"implementation", "checkout_name":"same-label"
        }))
        .unwrap();
        let mut tasks = core
            .store
            .add_plan(
                &conversation.id,
                &[
                    serde_json::to_value(&owner_spec).unwrap(),
                    serde_json::to_value(&options_spec).unwrap(),
                    serde_json::to_value(&isolated_spec).unwrap(),
                ],
            )
            .unwrap();
        tasks[0].base = task.base.clone();
        tasks[0].result = task.result.clone();
        tasks[0].status = status::QUEUED.into();
        tasks[0].feedback = Some(json!({"continuation_review":true}));
        core.store.update_task(&tasks[0]).unwrap();
        let (options_view, _) =
            task_directories(&core, &tasks[1], &options_spec, &project, &None).unwrap();
        assert_eq!(options_view, worktree_path(&core.state, &tasks[0]).unwrap());
        assert_eq!(
            std::fs::read_to_string(options_view.join("instrument.txt")).unwrap(),
            "repaired instrument"
        );
        core.set_status(&conversation.id, &mut tasks[1], status::APPROVED)
            .unwrap();
        core.drain_worktree_cleanup();
        assert!(
            options_view.exists(),
            "options cleanup must retain the owner's checkout"
        );
        let plan_id = tasks[0].plan_id.clone();
        let state = directory.path().join("state");
        drop(core);
        let core = Core::open(
            &state,
            Config::from_toml(include_str!("../../../../../../examples/antislop.toml")).unwrap(),
        )
        .unwrap();
        let mut tasks = core.store.plan(&plan_id).unwrap();
        let (restarted_options_view, _) =
            task_directories(&core, &tasks[1], &options_spec, &project, &None).unwrap();
        assert_eq!(restarted_options_view, options_view);
        let isolated_worktree = writer_worktree(&core, &mut tasks[2], &isolated_spec, &project)
            .unwrap()
            .unwrap();
        assert_ne!(isolated_worktree, restarted_options_view);
        assert_ne!(
            git::worktree_branch(&isolated_worktree).unwrap(),
            git::worktree_branch(&restarted_options_view).unwrap()
        );
        assert_eq!(
            std::fs::read_to_string(project.join("instrument.txt")).unwrap(),
            "old instrument"
        );
        std::fs::write(project.join("instrument.txt"), "reviewed upstream change").unwrap();
        let upstream = git::commit_all(&project, git::CommitPurpose::TestFixture).unwrap();
        core.record_plan_landing(&tasks[0].plan_id, "upstream", &upstream)
            .unwrap();
        let (conflict_view, _) =
            task_directories(&core, &tasks[1], &options_spec, &project, &None).unwrap();
        assert_eq!(conflict_view, options_view);
        assert_eq!(
            std::fs::read_to_string(conflict_view.join("instrument.txt")).unwrap(),
            "repaired instrument"
        );
        let saved_base = tasks[0].base.clone();
        let saved_result = tasks[0].result.clone();
        let writer = writer_worktree(&core, &mut tasks[0], &owner_spec, &project)
            .unwrap()
            .unwrap();
        assert_eq!(writer, options_view);
        assert_eq!(
            std::fs::read_to_string(writer.join("instrument.txt")).unwrap(),
            "repaired instrument"
        );
        assert_eq!(tasks[0].base, saved_base);
        assert_eq!(
            git::tree_identity(&project, tasks[0].result.as_deref().unwrap()).unwrap(),
            git::tree_identity(&project, saved_result.as_deref().unwrap()).unwrap()
        );
        assert!(
            tasks[0]
                .feedback
                .as_ref()
                .unwrap()
                .get("checkpoint_merge")
                .is_none()
        );
        assert_eq!(
            std::fs::read_to_string(project.join("instrument.txt")).unwrap(),
            "reviewed upstream change"
        );
        assert_eq!(git::head(&project).unwrap(), upstream);
    }

    #[test]
    fn options_task_reads_owner_worktree_when_task_generations_differ() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir(&project).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .current_dir(&project)
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(project.join("input.txt"), "current input").unwrap();
        git::commit_all(&project, git::CommitPurpose::TestFixture).unwrap();
        let config =
            Config::from_toml(include_str!("../../../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&directory.path().join("state"), config).unwrap();
        let conversation = core.start(&project).unwrap();
        let owner_spec: TaskContract = serde_json::from_value(json!({
            "id":"owner", "title":"Owner", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Implement the selected change",
            "change_kind":"code", "kind":"implementation", "checkout_name":"named-owner"
        }))
        .unwrap();
        let options_spec: TaskContract = serde_json::from_value(json!({
            "id":"owner-options", "title":"Options", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Select an approach",
            "change_kind":"none", "kind":"options", "options_for":"owner"
        }))
        .unwrap();
        let tasks = core
            .store
            .add_plan(
                &conversation.id,
                &[
                    serde_json::to_value(&options_spec).unwrap(),
                    serde_json::to_value(&owner_spec).unwrap(),
                ],
            )
            .unwrap();
        let _run = core
            .store
            .create_scope(&conversation.id, "Options with current task inputs")
            .unwrap();
        let owner = tasks
            .iter()
            .find(|task| task.spec["id"] == "owner")
            .unwrap();
        core.store
            .db()
            .execute(
                "UPDATE task_inputs SET generation=1 WHERE task_id=?1",
                rusqlite::params![owner.id],
            )
            .unwrap();
        let options = tasks
            .iter()
            .find(|task| task.spec["id"] == "owner-options")
            .unwrap();
        core.store
            .db()
            .execute(
                "UPDATE task_inputs SET generation=0 WHERE task_id=?1",
                rusqlite::params![options.id],
            )
            .unwrap();
        let current = core.store.plan(&tasks[0].plan_id).unwrap();
        let options = current
            .iter()
            .find(|task| task.spec["id"] == "owner-options")
            .unwrap();
        let owner = current
            .iter()
            .find(|task| task.spec["id"] == "owner")
            .unwrap();
        assert_ne!(options.input_generation, owner.input_generation);

        let (workdir, add_dirs) =
            task_directories(&core, options, &options_spec, &project, &None).unwrap();

        assert!(add_dirs.is_empty());
        assert_eq!(workdir, worktree_path(&core.state, owner).unwrap());
        assert_eq!(
            std::fs::read_to_string(workdir.join("input.txt")).unwrap(),
            "current input"
        );
        let mut owner_for_path = owner.clone();
        let writer = writer_worktree(&core, &mut owner_for_path, &owner_spec, &project)
            .unwrap()
            .unwrap();
        assert_eq!(workdir, writer);
        assert!(
            writer
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("named-owner")
        );
        assert!(
            git::worktree_branch(&writer)
                .unwrap()
                .contains("named-owner")
        );
        std::fs::write(writer.join("local-edit.txt"), "keep this edit").unwrap();
        let mut resumed = owner.clone();
        resumed.input_generation += 1;
        resumed.round += 1;
        let resumed_writer = writer_worktree(&core, &mut resumed, &owner_spec, &project)
            .unwrap()
            .unwrap();
        assert_eq!(writer, resumed_writer);
        assert_eq!(
            std::fs::read_to_string(resumed_writer.join("local-edit.txt")).unwrap(),
            "keep this edit"
        );
        assert_ne!(resumed_writer, worktree_path(&core.state, options).unwrap());

        let retained_tip =
            git::commit_all(&resumed_writer, git::CommitPurpose::TestFixture).unwrap();
        git::remove_worktree_keep_branch(&project, &resumed_writer).unwrap();
        std::fs::write(project.join("input.txt"), "revised input").unwrap();
        let revised_base = git::commit_all(&project, git::CommitPurpose::TestFixture).unwrap();
        assert_ne!(retained_tip, revised_base);

        // Options preparation reopens the owner's stable branch after checkout
        // removal and an upstream revision. It must use that branch's current
        // tip without rewriting it to the new requested base.
        let (resumed_options_dir, _) =
            task_directories(&core, options, &options_spec, &project, &None).unwrap();
        assert_eq!(resumed_options_dir, resumed_writer);
        assert_eq!(git::head(&resumed_options_dir).unwrap(), retained_tip);
        assert_eq!(
            std::fs::read_to_string(resumed_options_dir.join("local-edit.txt")).unwrap(),
            "keep this edit"
        );
        assert_eq!(
            std::fs::read_to_string(resumed_options_dir.join("input.txt")).unwrap(),
            "current input"
        );
        assert_eq!(git::head(&project).unwrap(), revised_base);
        let git::CombineResult::Applied(combined) = git::combine_or_conflict(
            &project,
            &core.state.join("worktrees").join("verify-restored-owner"),
            &revised_base,
            &[(owner_for_path.base.unwrap(), retained_tip)],
        )
        .unwrap() else {
            panic!("the retained delta must integrate onto the revised input")
        };
        assert_eq!(
            git::ok(&project, &["show", &format!("{combined}:input.txt")]).unwrap(),
            "revised input"
        );
        assert_eq!(
            git::ok(&project, &["show", &format!("{combined}:local-edit.txt")]).unwrap(),
            "keep this edit"
        );
        assert_eq!(git::head(&project).unwrap(), revised_base);
    }

    #[test]
    fn parallel_writer_results_share_the_session_checkout_with_later_readers() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir(&project).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q", "-b", "main"])
                .current_dir(&project)
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(project.join("base.txt"), "base").unwrap();
        let base = git::commit_all(&project, git::CommitPurpose::TestFixture).unwrap();
        let config =
            Config::from_toml(include_str!("../../../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&directory.path().join("state"), config).unwrap();
        let conversation = core.start(&project).unwrap();
        let specs: Vec<TaskContract> = [
            json!({"id":"writer-alpha","title":"Alpha writer","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Write alpha","change_kind":"code","kind":"implementation","checkout_name":"alpha-change"}),
            json!({"id":"writer-beta","title":"Beta writer","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Write beta","change_kind":"code","kind":"implementation","checkout_name":"beta-change"}),
            json!({"id":"reader-task","title":"Combined reader","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Read combined results","change_kind":"none","kind":"analysis"}),
        ]
        .into_iter()
        .map(serde_json::from_value)
        .collect::<std::result::Result<_, _>>()
        .unwrap();
        let mut tasks = core
            .store
            .add_plan(
                &conversation.id,
                &specs
                    .iter()
                    .map(serde_json::to_value)
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .unwrap(),
            )
            .unwrap();

        let alpha_path = writer_worktree(&core, &mut tasks[0], &specs[0], &project)
            .unwrap()
            .unwrap();
        let beta_path = writer_worktree(&core, &mut tasks[1], &specs[1], &project)
            .unwrap()
            .unwrap();
        assert_ne!(alpha_path, beta_path);
        std::fs::write(alpha_path.join("alpha.txt"), "alpha").unwrap();
        std::fs::write(beta_path.join("beta.txt"), "beta").unwrap();
        let alpha_result = git::commit_all(&alpha_path, git::CommitPurpose::TestFixture).unwrap();
        let beta_result = git::commit_all(&beta_path, git::CommitPurpose::TestFixture).unwrap();

        let session = core.session_worktree(&conversation.id, &project).unwrap();
        let combined = match git::combine_or_conflict(
            &project,
            &core.state.join("worktrees").join("parallel-reader-combine"),
            &base,
            &[
                (base.clone(), alpha_result.clone()),
                (base.clone(), beta_result.clone()),
            ],
        )
        .unwrap()
        {
            git::CombineResult::Applied(commit) => commit,
            git::CombineResult::Conflicted(detail) => {
                panic!("parallel changes conflicted: {detail}")
            }
        };
        git::ok(&session, &["reset", "--hard", &combined]).unwrap();
        core.record_plan_landing(&tasks[0].plan_id, &tasks[0].id, &alpha_result)
            .unwrap();
        core.record_plan_landing(&tasks[1].plan_id, &tasks[1].id, &beta_result)
            .unwrap();

        let (reader_path, add_dirs) =
            task_directories(&core, &tasks[2], &specs[2], &project, &None).unwrap();
        assert!(add_dirs.is_empty());
        assert_eq!(reader_path, session);
        assert!(!worktree_path(&core.state, &tasks[2]).unwrap().exists());
        assert_eq!(
            std::fs::read_to_string(reader_path.join("alpha.txt")).unwrap(),
            "alpha"
        );
        assert_eq!(
            std::fs::read_to_string(reader_path.join("beta.txt")).unwrap(),
            "beta"
        );
    }

    #[test]
    fn incomplete_checkpoints_are_not_completed_results_for_review_only_recovery() {
        for result in [
            crate::lean::turn_result::TurnResult::Blocked {
                report: "Checkpoint preserved, build unavailable".into(),
                resume_condition: "Runtime repaired".into(),
            },
            crate::lean::turn_result::TurnResult::NeedsInput {
                request: "fixture-question".into(),
                checkpoint: "Awaiting an answer, not completed work".into(),
            },
        ] {
            let dir = tempfile::tempdir().unwrap();
            let config =
                Config::from_toml(include_str!("../../../../../../examples/antislop.toml"))
                    .unwrap();
            let core = Core::open(&dir.path().join("state"), config).unwrap();
            let conversation = core
                .store
                .create_conversation(dir.path().to_str().unwrap())
                .unwrap()
                .id;
            let mut task=core.store.add_plan(&conversation,&[json!({"id":"work","title":"Work","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Inspect saved code","change_kind": "none", "kind":"analysis"})]).unwrap().remove(0);
            let worker = format!("worker:{}", task.id);
            let reviewer = format!("reviewer:{}", task.id);
            let turn = core
                .store
                .start_turn(&conversation, &worker, "fixture", 0, Some(&task.plan_id))
                .unwrap();
            core.store.save_turn_result(&turn.id, &result).unwrap();
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
                        commit_message: String::new(),
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
}
