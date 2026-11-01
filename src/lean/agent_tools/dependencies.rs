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
        let scope = self.dependency_reader_scope(conversation, role, turn)?;
        let task = scope.tasks.iter().rev().find(|task| {
            task.id == requested
                || task.spec["id"].as_str() == Some(&requested)
                || (requested == "selected_approach"
                    && scope.selected_for.as_deref().is_some_and(|id| task.spec["options_for"] == id))
                || self.store.completed_task_result(task).is_ok_and(|(producer, _)| producer == requested)
        }).ok_or_else(|| {
            let available = scope.tasks.iter().map(|task| {
                format!("{} ({})", task.spec["id"].as_str().unwrap_or("?"), task.id)
            }).take(20).collect::<Vec<_>>().join(", ");
            Error::Invalid(format!("dependency {requested:?} is not an input of this task or is unknown in its bound scope; available results: {available}"))
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
            self.store.dependency_report_round(task, requested_round)?;
        let report = dependency_report_text(&output)?;
        let binding = crate::hash(serde_json::to_vec(&(
            conversation,
            role,
            turn,
            scope.generation,
            task.input_generation,
            &task.id,
            &producer,
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

    fn dependency_reader_scope(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
    ) -> Result<DependencyScope> {
        if let Some((id, _)) = task_role(role) {
            let task = self
                .store
                .task_by_id(id)?
                .ok_or_else(|| Error::Invalid("task no longer exists".into()))?;
            require(
                self.store.plan_conversation(&task.plan_id)? == conversation,
                "task belongs to another conversation",
            )?;
            let spec: TaskContract = serde_json::from_value(task.spec)?;
            let mut allowed = spec
                .depends_on
                .iter()
                .cloned()
                .collect::<std::collections::HashSet<_>>();
            // selected_approach is a handoff alias for this task's local options.
            // It does not grant access to options owned by another task.
            let inputs = self.store.dependency_inputs(&task.plan_id, &spec)?;
            allowed.extend(inputs.into_iter().map(|input| input.id));
            let plan_tasks = self.store.plan(&task.plan_id)?;
            let mut pending = allowed.iter().cloned().collect::<Vec<_>>();
            while let Some(name) = pending.pop() {
                if let Some(ancestor) = plan_tasks.iter().find(|task| task.spec["id"] == name) {
                    for dependency in ancestor.spec["depends_on"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                    {
                        if allowed.insert(dependency.to_owned()) {
                            pending.push(dependency.to_owned());
                        }
                    }
                }
            }
            let tasks = plan_tasks
                .into_iter()
                .filter(|candidate| {
                    candidate.spec["id"]
                        .as_str()
                        .is_some_and(|id| allowed.contains(id))
                        || candidate.spec["options_for"] == spec.id
                })
                .collect();
            return Ok(DependencyScope {
                tasks,
                generation: task.input_generation,
                selected_for: Some(spec.id),
            });
        }
        if matches!(
            role,
            "requirements-audit-analyst" | "requirements-audit-verifier"
        ) {
            let (tasks, generation) =
                self.store
                    .audit_dependency_scope(conversation, role, turn)?;
            return Ok(DependencyScope {
                tasks,
                generation,
                selected_for: None,
            });
        }
        require(
            matches!(role, ORCHESTRATOR | COMMENT_ORCHESTRATOR),
            "tool unavailable for this role",
        )?;
        let plan = self.store.dependency_plan(conversation, role, turn)?;
        Ok(DependencyScope {
            tasks: self.store.plan(&plan)?,
            generation: self.store.plan_revision(&plan)?,
            selected_for: None,
        })
    }
}

struct DependencyScope {
    tasks: Vec<Task>,
    generation: i64,
    selected_for: Option<String>,
}

fn dependency_report_text(output: &Value) -> Result<String> {
    let report = output["report"].as_str();
    if output["selection"].is_object() {
        return Ok(format!(
            "{}\n\nSelected approach:\n{}",
            report.unwrap_or_default(),
            serde_json::to_string(&output["selection"])?
        ));
    }
    report
        .map(str::to_owned)
        .ok_or_else(|| Error::Invalid("dependency has no report".into()))
}
