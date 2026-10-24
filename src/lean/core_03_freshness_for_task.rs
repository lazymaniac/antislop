use super::core_path_helpers::task_path_component;
use super::*;

impl Core {
    pub(in crate::lean) fn freshness_for_task(
        &self,
        conversation: &str,
        plan_id: &str,
        task: &Task,
    ) -> Result<String> {
        let conv = self.store.conversation(conversation)?;
        let project = PathBuf::from(conv.project);
        let head = git::head(&project).unwrap_or_default();
        let scope: Vec<String> =
            serde_json::from_value(task.spec["scope"].clone()).unwrap_or_default();
        let fingerprint = git::working_tree_fingerprint(&project, &scope);
        let mut data = freshness_seed(conversation, plan_id, task, &head, &fingerprint);
        self.append_run_revision(conversation, plan_id, &mut data)?;
        Ok(crate::hash(data.as_bytes()))
    }

    fn append_run_revision(
        &self,
        conversation: &str,
        plan_id: &str,
        data: &mut String,
    ) -> Result<()> {
        if let Some(run) = self.store.plan_run(plan_id)? {
            let revision = self.store.run(conversation, &run)?.spec_revision;
            data.push_str(&revision.to_string());
        }
        Ok(())
    }

    pub fn task_unscoped(&self, plan: &str, id: &str) -> Result<Task> {
        self.store
            .task_in_plan_by_id(plan, id)?
            .ok_or_else(|| crate::Error::Invalid(format!("no task {id} in plan {plan}")))
    }

    /// One task of a plan, by the id the orchestrator gave it or its UUID.
    /// Populates freshness_token when the plan belongs to a conversation.
    pub fn task(&self, plan: &str, id: &str) -> Result<Task> {
        let mut task = self.task_unscoped(plan, id)?;
        if let Ok(conv) = self.store.plan_conversation(plan) {
            task.freshness_token = self.freshness_for_task(&conv, plan, &task).ok();
        }
        task.checks = self
            .store
            .checks(&task.id, task.round)
            .ok()
            .map(serde_json::Value::from);
        Ok(task)
    }

    /// Read-only inspector data needs no filesystem or Git freshness work.
    pub fn task_checks(&self, plan: &str, id: &str) -> Result<Task> {
        let mut task = self.task_unscoped(plan, id)?;
        task.checks = Some(serde_json::to_value(
            self.store.checks(&task.id, task.round)?,
        )?);
        Ok(task)
    }

    pub fn task_scoped(&self, conversation: &str, plan: &str, id: &str) -> Result<Task> {
        let mut task = self.task_unscoped(plan, id)?;
        task.freshness_token = self.freshness_for_task(conversation, plan, &task).ok();
        task.checks = self
            .store
            .checks(&task.id, task.round)
            .ok()
            .map(serde_json::Value::from);
        Ok(task)
    }

    /// Where a writer's change can be read: its worktree while it works (the
    /// working tree against its base), else the project's repository between
    /// its base and result commits.
    pub(in crate::lean) fn change(
        &self,
        conversation: &str,
        task: &Task,
    ) -> Result<(PathBuf, String, Option<String>)> {
        let id = task.spec["id"].as_str().unwrap_or_default();
        let base = task.base.clone().ok_or_else(|| {
            crate::Error::Invalid(format!(
                "task {id} has no change: it does not write files or has not started"
            ))
        })?;
        let worktree = self
            .state
            .join("worktrees")
            .join(task_path_component(&task.id)?);
        if worktree.exists() {
            return Ok((worktree, base, None));
        }
        let result = task.result.clone().ok_or_else(|| {
            crate::Error::Invalid(format!("task {id} has no committed change yet"))
        })?;
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        Ok((project, base, Some(result)))
    }

    /// A writer's change as files with per-file counts and numbered lines.
    pub fn diff(&self, conversation: &str, plan: &str, id: &str) -> Result<serde_json::Value> {
        let config = self.config_snapshot();
        let task = self.task_unscoped(plan, id)?;
        let (dir, base, result) = self.change(conversation, &task)?;
        let patch = git::patch(&dir, &base, result.as_deref())?;
        let (patch, truncated) = truncate_patch(patch);
        let scope_hash = diff_scope_hash(conversation, plan, id, &base, result.as_deref());
        let reviewer = reviewer_profile(&config, &task.spec);
        Ok(json!({
            "task": id,
            "base": base,
            "head": result,
            "files": diff::parse(&patch),
            "truncated": truncated,
            "scope_hash": scope_hash,
            "reviewer": reviewer,
        }))
    }
}

fn freshness_seed(
    conversation: &str,
    plan_id: &str,
    task: &Task,
    head: &str,
    fingerprint: &str,
) -> String {
    let mut data = String::new();
    data.push_str(conversation);
    data.push('\0');
    data.push_str(plan_id);
    data.push('\0');
    data.push_str(&task.id);
    data.push('\0');
    data.push_str(&task.input_generation.to_string());
    data.push_str(&task.round.to_string());
    data.push('\0');
    data.push_str(&task.status);
    data.push('\0');
    data.push_str(task.base.as_deref().unwrap_or(""));
    data.push('\0');
    data.push_str(task.result.as_deref().unwrap_or(""));
    data.push('\0');
    data.push_str(head);
    data.push('\0');
    data.push_str(fingerprint);
    data.push_str(&task.input_generation.to_string());
    data
}

fn truncate_patch(mut patch: String) -> (String, bool) {
    if patch.len() <= DIFF_LIMIT {
        return (patch, false);
    }
    let mut end = DIFF_LIMIT;
    while !patch.is_char_boundary(end) {
        end -= 1;
    }
    patch.truncate(end);
    (patch, true)
}

fn reviewer_profile(config: &Config, value: &serde_json::Value) -> Option<serde_json::Value> {
    let spec = serde_json::from_value::<TaskSpec>(value.clone()).ok()?;
    let profile = config.resolve(spec.kind, RoleKind::Verifier).ok()?;
    Some(json!({
        "provider": profile.provider,
        "model": profile.model,
        "effort": profile.effort,
    }))
}
