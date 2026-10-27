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
        let onto = self.plan_base(plan_id, project)?;
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

    /// The commit every task of a plan starts from. A snapshot is a new commit
    /// each time, even of an unchanged tree, so tasks that snapshotted when
    /// they started disagreed about their baseline and kept asking which
    /// commit was authoritative. The first task of the plan fixes it; what the
    /// user changes later stays protected when results are integrated.
    fn plan_base(&self, plan_id: &str, project: &Path) -> Result<String> {
        let record = self
            .state
            .join("plan-bases")
            .join(crate::hash(plan_id.as_bytes()));
        if let Ok(pinned) = std::fs::read_to_string(&record) {
            let pinned = pinned.trim();
            if !pinned.is_empty() && git::tree_identity(project, pinned).is_ok() {
                return Ok(pinned.to_owned());
            }
        }
        let base = if git::uncommitted(project)?.is_empty() {
            git::head(project)?
        } else {
            git::snapshot(project, "User working tree before task")?
        };
        if let Some(directory) = record.parent() {
            std::fs::create_dir_all(directory)?;
        }
        std::fs::write(&record, &base)?;
        Ok(base)
    }
}
