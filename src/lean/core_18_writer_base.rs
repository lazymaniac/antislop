use super::core_plan_helpers::writer_base_scratch;
use super::*;

impl Core {
    pub(in crate::lean) fn writer_base(
        &self,
        plan_id: &str,
        spec: &TaskSpec,
        project: &Path,
    ) -> Result<String> {
        let _integration = self.integrating.lock().expect("integration mutex");
        let changes: Vec<_> = self
            .dependency_tasks(plan_id, spec)?
            .into_iter()
            .filter_map(|task| Some((task.base?, task.result?)))
            .collect();
        let onto = if git::uncommitted(project)?.is_empty() {
            git::head(project)?
        } else {
            git::snapshot(project, "User working tree before task")?
        };
        if changes.is_empty() {
            return Ok(onto);
        }
        let scratch = writer_base_scratch(&self.state, plan_id, &spec.id);
        git::combine(project, &scratch, &onto, &changes).map_err(|error| {
            Error::Invalid(format!(
                "could not combine the results of {} for {}: {error}",
                spec.depends_on.join(", "),
                spec.id
            ))
        })
    }

    /// A real merge conflict is work for the writer, not a failure before its
    /// first turn. Preserve the original error and exact input commits in mail.
    pub(in crate::lean) fn repair_writer_base(
        &self,
        conversation: &str,
        task: &mut Task,
        spec: &mut TaskSpec,
        project: &Path,
        error: &Error,
    ) -> Result<String> {
        let upstream = self.dependency_tasks(&task.plan_id, spec)?;
        let mut inputs = Vec::new();
        for dependency in upstream {
            if let (Some(base), Some(result)) = (&dependency.base, &dependency.result) {
                inputs.push(format!(
                    "{}: git diff {base} {result}",
                    dependency.spec["id"]
                ));
                let scope: Vec<String> =
                    serde_json::from_value(dependency.spec["scope"].clone()).unwrap_or_default();
                for path in scope {
                    if !spec.scope.contains(&path) {
                        spec.scope.push(path);
                    }
                }
            }
        }
        let guidance = format!(
            "Automatic composition failed: {error}\nResolve the upstream changes below in your allocated worktree first, preserving every dependency's behavior and local user edits, then complete your original goal. These upstream paths are included in your scope for this repair. Use Git's stored commits, never alter another task's worktree or the primary working tree.\n{}",
            inputs.join("\n")
        );
        spec.goal = format!(
            "Integrate the required upstream results as described in the runtime guidance before this goal: {}",
            spec.goal
        );
        task.spec = serde_json::to_value(&spec)?;
        self.store.update_task(task)?;
        let role = format!("worker:{}", task.id);
        self.store.add_mail(
            conversation,
            &role,
            &Mail {
                id: 0,
                sender: RUNTIME.into(),
                recipient: role.clone(),
                kind: "guidance".into(),
                text: guidance,
                created_ms: 0,
                delivered_turn: None,
                delivered_ms: None,
                state: "queued".into(),
            },
        )?;
        self.store.append_event(
            conversation,
            None,
            Some(&task.plan_id),
            "base_repair",
            &json!({"task": task.id, "error": error.to_string(), "inputs": inputs}),
        )?;
        let _integration = self.integrating.lock().expect("integration mutex");
        git::snapshot(project, "User working tree before task")
    }

    pub(in crate::lean) fn defer_agent_work(
        &self,
        conversation: &str,
        task: &mut Task,
    ) -> Result<bool> {
        let pending = [
            format!("worker:{}", task.id),
            format!("verifier:{}", task.id),
        ]
        .iter()
        .map(|role| self.store.pending_mail(conversation, role))
        .collect::<Result<Vec<_>>>()?
        .iter()
        .flatten()
        .any(|mail| mail.kind == "agent_work");
        if pending {
            task.verdict = None;
            task.feedback = None;
            self.set_status(conversation, task, status::QUEUED)?;
        }
        Ok(pending)
    }
}
