use super::*;

/// How much of a dependency's summary a dependent task is shown.
const DEPENDENCY_SUMMARY_LIMIT: usize = 2_000;
/// How much of an earlier check's summary a closing task is shown.
const CHECK_SUMMARY_LIMIT: usize = 800;

impl Store {
    pub(in crate::lean) fn dependency_inputs(
        &self,
        plan: &str,
        spec: &crate::lean::tasks::TaskContract,
    ) -> Result<Vec<crate::lean::tasks::DependencyReport>> {
        let mut inputs = self.direct_dependency_reports(plan, spec)?;
        inputs.extend(self.earlier_check_reports(plan, spec)?);
        inputs.extend(self.selected_approach_reports(plan, &spec.id)?);
        Ok(inputs)
    }

    fn direct_dependency_reports(
        &self,
        plan: &str,
        spec: &crate::lean::tasks::TaskContract,
    ) -> Result<Vec<crate::lean::tasks::DependencyReport>> {
        let mut inputs = Vec::new();
        for dependency in &spec.depends_on {
            if let Some(report) = self.dependency_report(plan, spec, dependency)? {
                inputs.push(report);
            }
        }
        Ok(inputs)
    }

    fn dependency_report(
        &self,
        plan: &str,
        spec: &crate::lean::tasks::TaskContract,
        dependency: &str,
    ) -> Result<Option<crate::lean::tasks::DependencyReport>> {
        let task = self
            .task_in_plan_by_id(plan, dependency)?
            .ok_or_else(|| Error::Invalid(format!("missing dependency {dependency}")))?;
        if task.spec["options_for"].as_str() == Some(spec.id.as_str()) {
            return Ok(None);
        }
        crate::require(task.status == "approved", "dependency is not reviewed")?;
        let summary = self.dependency_summary(&task)?;
        Ok(Some(crate::lean::tasks::DependencyReport {
            id: dependency.to_owned(),
            title: task.spec["title"].as_str().unwrap_or(dependency).to_owned(),
            report: format!(
                "Summary (abridged); full result: {}. Read with read_dependency_result(dependency=\"{}\").\n{}",
                task.feedback
                    .as_ref()
                    .and_then(|f| f["carried_from"].as_str())
                    .unwrap_or(&task.id),
                dependency,
                crate::lean::tasks::bound(&summary, DEPENDENCY_SUMMARY_LIMIT)
            ),
        }))
    }

    /// Closing work includes approved checks reached through its dependencies.
    fn earlier_check_reports(
        &self,
        plan: &str,
        spec: &crate::lean::tasks::TaskContract,
    ) -> Result<Vec<crate::lean::tasks::DependencyReport>> {
        let tasks = self.plan(plan)?;
        let by_id = task_contracts(&tasks);
        if !is_closing_task(spec, &by_id) {
            return Ok(Vec::new());
        }
        let reached = earlier_checks(spec, &by_id);
        self.summarize_earlier_checks(reached)
    }

    fn summarize_earlier_checks(
        &self,
        reached: Vec<(&Task, &crate::lean::tasks::TaskContract)>,
    ) -> Result<Vec<crate::lean::tasks::DependencyReport>> {
        let mut reports = Vec::new();
        for (task, check) in reached {
            reports.push(crate::lean::tasks::DependencyReport {
                id: check.id.clone(),
                title: format!("{} (earlier check)", check.title),
                report: crate::lean::tasks::bound(
                    &self.dependency_summary(task)?,
                    CHECK_SUMMARY_LIMIT,
                ),
            });
        }
        reports.sort_by(|left, right| left.id.cmp(&right.id));
        Ok(reports)
    }

    fn selected_approach_reports(
        &self,
        plan: &str,
        task_id: &str,
    ) -> Result<Vec<crate::lean::tasks::DependencyReport>> {
        let choices = self.option_task_ids(plan, task_id)?;
        choices
            .iter()
            .map(|id| self.selected_approach_report(id))
            .collect()
    }

    fn option_task_ids(&self, plan: &str, task_id: &str) -> Result<Vec<String>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT id FROM tasks WHERE plan_id=?1 AND json_extract(spec,'$.options_for')=?2 AND status<>'superseded'",
        )?;
        Ok(statement
            .query_map(params![plan, task_id], |row| row.get(0))?
            .collect::<std::result::Result<_, _>>()?)
    }

    fn selected_approach_report(
        &self,
        task_id: &str,
    ) -> Result<crate::lean::tasks::DependencyReport> {
        let task = self
            .task_by_id(task_id)?
            .ok_or_else(|| Error::Invalid("options task disappeared".into()))?;
        crate::require(task.status == "approved", "local options are not reviewed")?;
        let result = self.completed_task_output(&task)?;
        selected_report(&result)
    }

    /// Uses a result summary, falling back to the first report line for older plans.
    fn dependency_summary(&self, task: &Task) -> Result<String> {
        let summary = self
            .completed_task_output(task)
            .map(Some)
            .or_else(|error| match error {
                Error::Sql(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                error => Err(error),
            })?;
        Ok(summary
            .and_then(|result| result["summary"].as_str().map(str::to_owned))
            .unwrap_or_else(|| first_report_line(task)))
    }
}

fn task_contracts(
    tasks: &[Task],
) -> std::collections::HashMap<String, (&Task, crate::lean::tasks::TaskContract)> {
    tasks
        .iter()
        .filter_map(|task| {
            serde_json::from_value(task.spec.clone())
                .ok()
                .map(|spec: crate::lean::tasks::TaskContract| (spec.id.clone(), (task, spec)))
        })
        .collect()
}

fn is_closing_task(
    spec: &crate::lean::tasks::TaskContract,
    by_id: &std::collections::HashMap<String, (&Task, crate::lean::tasks::TaskContract)>,
) -> bool {
    use crate::config::WorkKind;
    spec.kind == WorkKind::Delivery
        || spec.depends_on.iter().any(|id| {
            by_id
                .get(id)
                .is_some_and(|(_, dependency)| dependency.kind == WorkKind::Integration)
        })
}

fn earlier_checks<'a>(
    spec: &crate::lean::tasks::TaskContract,
    by_id: &'a std::collections::HashMap<String, (&'a Task, crate::lean::tasks::TaskContract)>,
) -> Vec<(&'a Task, &'a crate::lean::tasks::TaskContract)> {
    let mut seen: std::collections::BTreeSet<String> = spec.depends_on.iter().cloned().collect();
    let mut pending = spec.depends_on.clone();
    let mut reports = Vec::new();
    while let Some(id) = pending.pop() {
        let Some((_, dependency)) = by_id.get(&id) else {
            continue;
        };
        collect_earlier_checks(dependency, by_id, &mut seen, &mut pending, &mut reports);
    }
    reports.sort_by(|left, right| left.1.id.cmp(&right.1.id));
    reports
}

fn collect_earlier_checks<'a>(
    dependency: &crate::lean::tasks::TaskContract,
    by_id: &'a std::collections::HashMap<String, (&'a Task, crate::lean::tasks::TaskContract)>,
    seen: &mut std::collections::BTreeSet<String>,
    pending: &mut Vec<String>,
    reports: &mut Vec<(&'a Task, &'a crate::lean::tasks::TaskContract)>,
) {
    for next in &dependency.depends_on {
        if !seen.insert(next.clone()) {
            continue;
        }
        pending.push(next.clone());
        let Some((task, check)) = by_id.get(next) else {
            continue;
        };
        if earlier_check_is_approved(task, check) {
            reports.push((*task, check));
        }
    }
}

fn earlier_check_is_approved(task: &Task, check: &crate::lean::tasks::TaskContract) -> bool {
    use crate::config::WorkKind;
    matches!(
        check.kind,
        WorkKind::StaticAnalysis
            | WorkKind::Testing
            | WorkKind::Security
            | WorkKind::Performance
            | WorkKind::Integration
    ) && check.options_for.is_none()
        && task.status == "approved"
}

fn selected_report(result: &Value) -> Result<crate::lean::tasks::DependencyReport> {
    let selection = &result["selection"];
    let variant = selection["alternatives"]
        .as_array()
        .and_then(|all| all.iter().find(|item| item["id"] == selection["selected"]))
        .ok_or_else(|| Error::Invalid("local options have no selected variant".into()))?;
    Ok(crate::lean::tasks::DependencyReport {
        id: "selected_approach".into(),
        title: "Selected approach for this task".into(),
        report: serde_json::json!({"variant":variant,"rationale":selection["rationale"]})
            .to_string(),
    })
}

fn first_report_line(task: &Task) -> String {
    task.report
        .as_deref()
        .and_then(|report| report.lines().find(|line| !line.trim().is_empty()))
        .unwrap_or_default()
        .to_owned()
}
