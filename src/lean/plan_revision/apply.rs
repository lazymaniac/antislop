use super::*;

impl Store {
    pub(in crate::lean) fn plan_revision(&self, plan: &str) -> Result<i64> {
        self.db()
            .query_row(
                "SELECT revision FROM plan_heads WHERE plan_id=?1",
                [plan],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::Invalid("plan has no revision head; it was replaced".into()))
    }

    pub(in crate::lean) fn apply_plan_revision(
        &self,
        conversation: &str,
        revision: &PlanRevision,
        baseline: &[Task],
        specs: &[TaskContract],
        affected: &std::collections::BTreeSet<String>,
        lease: &RevisionLease<'_>,
    ) -> Result<()> {
        self.apply_plan_revision_with_options(
            conversation,
            revision,
            baseline,
            specs,
            affected,
            lease,
            true,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::lean) fn apply_plan_revision_with_options(
        &self,
        conversation: &str,
        revision: &PlanRevision,
        baseline: &[Task],
        specs: &[TaskContract],
        affected: &std::collections::BTreeSet<String>,
        lease: &RevisionLease<'_>,
        is_accepted: bool,
    ) -> Result<()> {
        let apply = RevisionApply {
            conversation,
            revision,
            baseline,
            specs,
            affected,
            is_accepted,
        };
        let mut db = self.db();
        let tx = db.transaction()?;
        apply.review_lease(&tx, lease)?;
        apply.update_removed_tasks(&tx)?;
        for (position, spec) in specs.iter().enumerate() {
            apply.save_task(&tx, position, spec)?;
        }
        apply.record_graph(&tx)?;
        apply.finish(&tx, lease)?;
        tx.commit()?;
        Ok(())
    }
}

struct RevisionApply<'a> {
    conversation: &'a str,
    revision: &'a PlanRevision,
    baseline: &'a [Task],
    specs: &'a [TaskContract],
    affected: &'a std::collections::BTreeSet<String>,
    is_accepted: bool,
}

#[path = "apply/tasks.rs"]
mod tasks;
