use super::*;

impl Core {
    /// The summaries of the direct dependencies and the selected local
    /// approach, as task data.
    pub(in crate::lean) fn upstream(&self, plan_id: &str, spec: &TaskContract) -> Result<String> {
        let reports = self.store.dependency_inputs(plan_id, spec)?;
        Ok(tasks::dependency_reports_note(&reports))
    }

    /// Where a writer starts: from the result of the one task it depends on,
    /// from the user's HEAD when none of them wrote, and from HEAD with all
    /// their results laid on it when several did.
    pub(in crate::lean) fn dependency_tasks(
        &self,
        plan_id: &str,
        spec: &TaskContract,
    ) -> Result<Vec<Task>> {
        let plan = self.store.plan(plan_id)?;
        let mut seen = std::collections::HashSet::new();
        let mut ordered = Vec::new();
        for dep in &spec.depends_on {
            collect_dependencies(dep, &plan, &mut seen, &mut ordered)?;
        }
        Ok(ordered)
    }
}

fn collect_dependencies(
    id: &str,
    plan: &[Task],
    seen: &mut std::collections::HashSet<String>,
    ordered: &mut Vec<Task>,
) -> Result<()> {
    if !seen.insert(id.to_owned()) {
        return Ok(());
    }
    let task = plan
        .iter()
        .find(|task| task.spec["id"] == id)
        .ok_or_else(|| Error::Invalid(format!("missing dependency {id}")))?;
    let dependencies: Vec<String> = serde_json::from_value(task.spec["depends_on"].clone())?;
    for dependency in dependencies {
        collect_dependencies(&dependency, plan, seen, ordered)?;
    }
    ordered.push(task.clone());
    Ok(())
}
