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
        let apply =
            self.revision_apply_context(conversation, revision, baseline, specs, affected, lease)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        apply.verify_lease(&tx, lease)?;
        apply.update_removed_tasks(&tx)?;
        for (position, spec) in specs.iter().enumerate() {
            apply.save_task(&tx, position, spec)?;
        }
        apply.record_graph(&tx)?;
        apply.finish(&tx, lease)?;
        tx.commit()?;
        Ok(())
    }

    fn revision_apply_context<'a>(
        &self,
        conversation: &'a str,
        revision: &'a PlanRevision,
        baseline: &'a [Task],
        specs: &'a [TaskContract],
        affected: &'a std::collections::BTreeSet<String>,
        lease: &RevisionLease<'_>,
    ) -> Result<RevisionApply<'a>> {
        let run = self
            .plan_run(&revision.plan)?
            .ok_or_else(|| Error::Invalid("plan has no requirements".into()))?;
        let entries = self.ledger_entries(conversation, &run, Some(lease.requirement_revision))?;
        tasks::validate_revision_coverage(specs, &entries)?;
        Ok(RevisionApply {
            conversation,
            revision,
            baseline,
            specs,
            affected,
            run,
            entries,
        })
    }
}

struct RevisionApply<'a> {
    conversation: &'a str,
    revision: &'a PlanRevision,
    baseline: &'a [Task],
    specs: &'a [TaskContract],
    affected: &'a std::collections::BTreeSet<String>,
    run: String,
    entries: Vec<crate::lean::ledger::LedgerEntry>,
}

#[path = "apply/tasks.rs"]
mod tasks;
