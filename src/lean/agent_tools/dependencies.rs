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
        let coordinating = matches!(role, ORCHESTRATOR | COMMENT_ORCHESTRATOR);
        let task = scope.tasks.iter().rev().find(|task| {
            if task_role(role).is_some_and(|(id, _)| id == task.id) && requested != "self" { return false; }
            (task.id == requested || (requested == "self" && task_role(role).is_some_and(|(id, _)| id == task.id)))
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
            task.status == status::APPROVED
                || coordinating
                || task_role(role).is_some_and(|(id, _)| id == task.id),
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
        let history = self.store.dependency_reports(task)?;
        let result_ref = args
            .get("result_ref")
            .filter(|v| !v.is_null())
            .map(|v| {
                v.as_str()
                    .ok_or_else(|| Error::Invalid("result_ref: expected a report reference".into()))
            })
            .transpose()?;
        let own_history = task_role(role).is_some_and(|(id, _)| id == task.id);
        let readable_default = |r: &&store::dependency_results::ReportReference| {
            r.current_inputs && (r.result_type == "completed" || own_history || coordinating)
        };
        let selected = if let Some(reference) = result_ref {
            Some(
                history
                    .reports
                    .iter()
                    .find(|r| r.result_ref == reference)
                    .ok_or_else(|| {
                        self.report_reference_error(conversation, reference, &history)
                    })?,
            )
        } else if let Some(round) = requested_round {
            history
                .reports
                .iter()
                .find(|r| r.round == round && readable_default(r))
        } else {
            history.reports.iter().find(readable_default).or_else(|| {
                history
                    .reports
                    .iter()
                    .find(|r| Some(&r.result_ref) == history.retained.as_ref())
            })
        };
        if let Some(selected) = selected {
            require(
                requested_round.is_none_or(|round| round == selected.round),
                "round: does not match result_ref",
            )?;
        }
        let mut available_rounds = history
            .reports
            .iter()
            .filter(|r| r.current_inputs && r.result_type == "completed")
            .map(|r| r.round)
            .collect::<Vec<_>>();
        available_rounds.dedup();
        let round = selected.map_or(history.producer.round, |r| r.round);
        let generation = selected.map_or(history.producer.input_generation, |r| r.input_generation);
        let historical = selected.is_some_and(|r| !r.current_inputs);
        let selected_producer = selected.map(|r| r.producer_task.clone());
        let result_type = selected.map_or("completed", |r| r.result_type.as_str());
        let selected_ref = selected.map(|r| r.result_ref.clone());
        let producer = history.producer.id.clone();
        let output = if let Some(reference) = selected {
            self.store.referenced_worker_report(reference)?
        } else {
            require(
                requested_round
                    .is_none_or(|r| r == history.producer.round && available_rounds.is_empty()),
                "round: no completed worker report for these dependency inputs",
            )?;
            match history.current {
                Some(output) => output,
                None => {
                    return Ok(
                        json!({"state":"no_current_result","result_id":producer,"report":"","complete":true,"available_rounds":available_rounds,"report_history":history.reports}),
                    );
                }
            }
        };
        if result_type == "completed"
            && available_rounds.is_empty()
            && generation == history.producer.input_generation
        {
            available_rounds.push(round);
        }
        let evidence = selected_ref
            .as_deref()
            .map(|id| self.store.result_evidence(id))
            .transpose()?
            .unwrap_or_default();
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
            generation,
            &selected_ref,
            crate::hash(report.as_bytes()),
        ))?);
        let (page, cursor) = page_text(&report, &binding, args)?;
        Ok(
            json!({"dependency":requested,"task_status":task.status,"task_approved":task.status == status::APPROVED,"result_type":result_type,"result_id":producer,"round":round,"available_rounds":available_rounds,"report_history":history.reports,"result_ref":selected_ref,"input_generation":generation,"historical":historical,"retained_for_review":history.retained.is_some(),"compatibility_reviewed":result_type == "completed" && history.retained.is_some() && task.status == status::APPROVED,"report_producer":selected_producer,"evidence":evidence,"report":page,"complete":cursor.is_none(),"cursor":cursor}),
        )
    }

    pub(super) fn dependency_reader_scope(
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
            let spec: TaskContract = serde_json::from_value(task.spec.clone())?;
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
            let mut tasks: Vec<Task> = plan_tasks
                .into_iter()
                .filter(|candidate| {
                    candidate.spec["id"]
                        .as_str()
                        .is_some_and(|id| allowed.contains(id))
                        || candidate.spec["options_for"] == spec.id
                })
                .collect();
            let generation = task.input_generation;
            tasks.push(task);
            return Ok(DependencyScope {
                tasks,
                generation,
                selected_for: Some(spec.id),
            });
        }
        if matches!(
            role,
            "requirements-audit-analyst" | "requirements-audit-reviewer"
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

pub(super) struct DependencyScope {
    pub(super) tasks: Vec<Task>,
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

/// Cursor leases include the reader turn and the immutable result identity.
pub(super) fn page_text(
    text: &str,
    binding: &str,
    args: &Value,
) -> Result<(String, Option<String>)> {
    let offset = match args.get("cursor").filter(|v| !v.is_null()) {
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
        offset <= text.len() && text.is_char_boundary(offset),
        "cursor: invalid boundary",
    )?;
    let mut end = offset.saturating_add(16 * 1024).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    Ok((
        text[offset..end].to_owned(),
        (end < text.len()).then(|| format!("{binding}:{end}")),
    ))
}
