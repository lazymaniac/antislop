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
            false,
            true,
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
        allow_uncovered: bool,
        require_run: bool,
        is_accepted: bool,
    ) -> Result<()> {
        let apply = self.revision_apply_context(
            conversation,
            revision,
            baseline,
            specs,
            affected,
            lease,
            allow_uncovered,
            require_run,
            is_accepted,
        )?;
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

    pub(in crate::lean) fn check_user_revision_coverage(
        &self,
        conversation: &str,
        plan: &str,
        specs: &[TaskContract],
    ) -> Result<Option<String>> {
        let Some(run) = self.plan_run(plan)? else {
            return Ok(None);
        };
        let entries = self.ledger_entries(conversation, &run, None)?;
        if entries.is_empty() {
            return Ok(None);
        }
        match tasks::validate_revision_coverage(specs, &entries) {
            Ok(()) => Ok(None),
            Err(error) => Ok(Some(error.to_string())),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn revision_apply_context<'a>(
        &self,
        conversation: &'a str,
        revision: &'a PlanRevision,
        baseline: &'a [Task],
        specs: &'a [TaskContract],
        affected: &'a std::collections::BTreeSet<String>,
        lease: &RevisionLease<'_>,
        allow_uncovered: bool,
        require_run: bool,
        is_accepted: bool,
    ) -> Result<RevisionApply<'a>> {
        let (run, entries) = match self.plan_run(&revision.plan)? {
            Some(run) => {
                let entries =
                    self.ledger_entries(conversation, &run, Some(lease.requirement_revision))?;
                if !allow_uncovered && (!entries.is_empty() || require_run) {
                    tasks::validate_revision_coverage(specs, &entries)?;
                }
                (run, entries)
            }
            None if require_run => {
                return Err(Error::Invalid("plan has no requirements".into()));
            }
            None => (String::new(), Vec::new()),
        };
        Ok(RevisionApply {
            conversation,
            revision,
            baseline,
            specs,
            affected,
            run,
            entries,
            is_accepted,
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
    is_accepted: bool,
}

#[path = "apply/tasks.rs"]
mod tasks;
