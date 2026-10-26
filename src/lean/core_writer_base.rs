use super::core_plan_helpers::writer_base_scratch;
use super::*;

impl Core {
    pub(in crate::lean) fn writer_base(
        &self,
        plan_id: &str,
        spec: &TaskContract,
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
}
