use super::*;

impl Core {
    /// Result access follows current task inputs, never arbitrary session history.
    pub(super) fn read_dependency_result(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        args: &Value,
    ) -> Result<Value> {
        let requested = required_text(args, "dependency")?;
        let (plan, generation, allowed) = if let Some((id, _)) = task_role(role) {
            let task = self
                .store
                .task_by_id(id)?
                .ok_or_else(|| Error::Invalid("task no longer exists".into()))?;
            require(
                self.store.plan_conversation(&task.plan_id)? == conversation,
                "task belongs to another conversation",
            )?;
            let spec: TaskContract = serde_json::from_value(task.spec)?;
            let inputs = self.store.dependency_inputs(&task.plan_id, &spec)?;
            (
                task.plan_id,
                task.input_generation,
                Some(
                    spec.depends_on
                        .into_iter()
                        .chain(inputs.into_iter().map(|input| input.id))
                        .collect::<std::collections::HashSet<_>>(),
                ),
            )
        } else {
            require(
                matches!(role, ORCHESTRATOR | COMMENT_ORCHESTRATOR),
                "tool unavailable for this role",
            )?;
            let plan = self.store.dependency_plan(conversation, role, turn)?;
            let revision = self.store.plan_revision(&plan)?;
            (plan, revision, None)
        };
        let task = self
            .store
            .plan(&plan)?
            .into_iter()
            .filter(|task| {
                allowed
                    .as_ref()
                    .is_none_or(|ids| task.spec["id"].as_str().is_some_and(|id| ids.contains(id)))
            })
            .find(|task| {
                task.id == requested
                    || task.spec["id"].as_str() == Some(&requested)
                    || self
                        .store
                        .completed_task_result(task)
                        .is_ok_and(|(producer, _)| producer == requested)
            })
            .ok_or_else(|| {
                Error::Invalid(
                    "dependency is not an input of this task or is unknown in its bound plan"
                        .into(),
                )
            })?;
        require(
            task.status == status::APPROVED,
            "dependency is not approved",
        )?;
        let requested_round = args
            .get("round")
            .filter(|v| !v.is_null())
            .map(|value| {
                value
                    .as_u64()
                    .and_then(|n| u32::try_from(n).ok())
                    .ok_or_else(|| Error::Invalid("round: expected a non-negative integer".into()))
            })
            .transpose()?;
        let (producer, round, available_rounds, output) =
            self.store.dependency_report_round(&task, requested_round)?;
        let report = output["report"]
            .as_str()
            .ok_or_else(|| Error::Invalid("dependency has no report".into()))?;
        let binding = crate::hash(serde_json::to_vec(&(
            conversation,
            role,
            turn,
            generation,
            &task.id,
            round,
            crate::hash(report.as_bytes()),
        ))?);
        let offset = match args.get("cursor").filter(|value| !value.is_null()) {
            None => 0,
            Some(value) => {
                let (key, offset) = value
                    .as_str()
                    .and_then(|s| s.split_once(':'))
                    .ok_or_else(|| Error::Invalid("cursor: expected continuation cursor".into()))?;
                require(
                    key == binding,
                    "cursor: belongs to another turn, input generation or result",
                )?;
                offset
                    .parse::<usize>()
                    .map_err(|_| Error::Invalid("cursor: invalid offset".into()))?
            }
        };
        require(
            offset <= report.len() && report.is_char_boundary(offset),
            "cursor: invalid boundary",
        )?;
        let mut end = offset.saturating_add(16 * 1024).min(report.len());
        while !report.is_char_boundary(end) {
            end -= 1;
        }
        let cursor = (end < report.len()).then(|| format!("{binding}:{end}"));
        Ok(
            json!({"dependency":requested,"result_id":producer,"round":round,"available_rounds":available_rounds,"report":&report[offset..end],"cursor":cursor,"complete":end==report.len()}),
        )
    }
}
