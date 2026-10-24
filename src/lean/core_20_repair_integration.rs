use super::core_feedback_helpers::land;
use super::core_path_helpers::worktree_branch;
use super::*;

impl Core {
    /// Resolve in the allocated worktree, then run the independent verifier
    /// again. A semantic choice is the resolver's explicit user question.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::lean) fn repair_integration(
        self: &Arc<Self>,
        conversation: &str,
        task: &mut Task,
        spec: &TaskSpec,
        worktree: &Path,
        onto: &str,
        change: &str,
        outcome: Outcome,
        files: &[String],
        cancel: &AtomicBool,
    ) -> Result<()> {
        let attempts = task
            .feedback
            .as_ref()
            .and_then(|feedback| feedback["integration_repairs"].as_u64())
            .unwrap_or(0)
            + 1;
        task.feedback
            .get_or_insert_with(|| json!({"round": task.round}))["integration_repairs"] =
            json!(attempts);
        let base = task.base.as_deref().unwrap_or_default().to_owned();
        let prompt = format!(
            "Automatic integration conflict in {}. Resolve it in your allocated worktree. \
             Original base: {base}; your result: {change}; current project snapshot: {onto}.\n\
             Inspect git diff {base} {change} and git diff {base} {onto}, plus the relevant \
             files and tests. Combine both intentions, preserving current local user edits \
             and the task's behavior. Edit only task paths in this worktree; never modify \
             the primary working tree or another worktree. The runtime will use {onto} \
             as the new base and independently verify your repaired result before landing. \
             Do not merge unrelated snapshot paths into your result.\n\
             Resolve mechanical conflicts yourself. If the versions represent an unclear \
             product or behavior choice, do not guess or silently pick ours/theirs. Return \
             a fenced integration_question block describing the conflicting intentions, \
             the concrete alternatives and a question asking the user which to accept. \
             Otherwise resolve the files and report what you preserved and how you tested it.",
            files.join(", "),
        );
        self.set_status(conversation, task, status::WORKING)?;
        let worker = format!("worker:{}", task.id);
        let setup = TurnSetup {
            session_policy: SessionPolicy::TaskRound {
                task: task.id.clone(),
                round: task.round,
            },
            conversation,
            plan_id: Some(task.plan_id.clone()),
            role: worker.clone(),
            profile: self
                .config_snapshot()
                .resolve(spec.kind, RoleKind::Executor),
            cwd: worktree.to_path_buf(),
            add_dirs: vec![],
            access: agent::Access::Write,
            upto_message: 0,
        };
        let handoff = self.round_handoff(&task.id, task.round, worktree)?;
        let Some(answer) = self.turn(setup, cancel, |fresh| {
            let prompt = tasks::worker_turn_prompt(
                &worktree.to_string_lossy(),
                spec,
                &[],
                false,
                Some(&prompt),
                "",
                "",
            );
            if fresh {
                prompt
            } else {
                prompt + &tasks::round_handoff_prompt(&handoff, false)
            }
        })?
        else {
            return self.stop_task(conversation, task, &worker, cancel);
        };
        let report = self.post(conversation, Some(&task.plan_id), &spec.id, &answer)?;
        if let Some(question) = tasks::integration_question(&report) {
            task.verdict = None;
            task.report = Some(report);
            land(
                task,
                outcome,
                "run",
                &question,
                None,
                Some(&worktree_branch(&self.state, task)?),
                files,
            );
            return self.stop(conversation, task, status::NEEDS_USER, Some(&question));
        }
        self.store.assert_task_inputs(task)?;
        // Invalidate approval before rebuilding the private branch, including
        // across interruption between the Git operations and the row update.
        task.report = Some(report);
        task.verdict = None;
        task.feedback = Some(json!({"round": task.round, "integration_repairs": attempts}));
        self.store.update_task(task)?;
        let result = git::transplant_repair(
            worktree,
            &base,
            onto,
            &format!("{} (conflict resolution)", spec.title),
        )?;
        task.base = Some(onto.to_owned());
        task.result = Some(result);
        self.store.update_task(task)?;
        let verifier = format!("verifier:{}", task.id);
        self.store.add_mail(conversation, &verifier, &Mail {
            id: 0,
            sender: RUNTIME.into(),
            recipient: verifier.clone(),
            kind: "agent_work".into(),
            text: format!("Verify the conflict resolution independently. Compare original task delta: git diff {base} {change}; concurrent project edits: git diff {base} {onto}. Confirm both intentions survive in {} and that no unrelated project edits are reverted. Reject silent ours/theirs choices and unresolved conflict markers. If a behavior choice needs the user, require an explicit integration_question from the worker.", files.join(", ")),
            created_ms: 0,
            delivered_turn: None,
            delivered_ms: None,
            state: "queued".into(),
        })?;
        self.set_status(conversation, task, status::QUEUED)
    }

    /// Protect dirty paths unless their starting version is exactly the
    /// last approved writer's output for that path. Only paths that writer
    /// changed count: an approved snapshot also contains unrelated user edits.
    pub(in crate::lean) fn in_place_user_edits(
        &self,
        project: &Path,
        base: &str,
    ) -> Result<Vec<String>> {
        let committed_base = git::common_base(project, base, "HEAD")?;
        let protected = git::changed_files(project, &committed_base, base)?;
        if protected.is_empty() {
            return Ok(protected);
        }
        let approved = self
            .store
            .approved_writer_results(&project.to_string_lossy())?;
        retain_user_edits(project, base, protected, approved)
    }
}

fn retain_user_edits(
    project: &Path,
    base: &str,
    mut protected: Vec<String>,
    approved: Vec<(String, String)>,
) -> Result<Vec<String>> {
    let mut unresolved = protected.clone();
    for (before, result) in approved {
        let candidates = changed_candidates(project, &before, &result, &unresolved)?;
        if candidates.is_empty() {
            continue;
        }
        let owned = git::matching_paths(project, &result, base, &candidates)?;
        protected.retain(|path| !owned.contains(path));
        unresolved.retain(|path| !candidates.contains(path));
        if unresolved.is_empty() {
            break;
        }
    }
    Ok(protected)
}

fn changed_candidates(
    project: &Path,
    before: &str,
    result: &str,
    unresolved: &[String],
) -> Result<Vec<String>> {
    let changed = git::changed_files(project, before, result)?;
    Ok(unresolved
        .iter()
        .filter(|path| changed.contains(path))
        .cloned()
        .collect())
}
