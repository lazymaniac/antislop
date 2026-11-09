use super::*;

impl Core {
    pub(super) fn apply_checked_ledger_change(
        &self,
        proposal_id: &str,
        diff_hash: &str,
        change: &LedgerChange,
    ) -> Result<ChangeResult> {
        self.store
            .check_ledger_proposal(proposal_id, diff_hash, change)?;
        match self.store.applied_ledger_change(change)? {
            Some(saved) => Ok(saved),
            None => {
                let current = self.store.preview_ledger_change(change, &self.state)?;
                require(
                    super::super::diff_hash(&current)? == diff_hash,
                    "ledger impact changed; preview the current diff before applying",
                )?;
                self.store.apply_ledger_change(change, &self.state, true)
            }
        }
    }

    pub(super) fn announce_affected_ledger_tasks(
        &self,
        change: &LedgerChange,
        result: &ChangeResult,
    ) -> Result<()> {
        self.activity.cancel_tasks(&result.affected_tasks);
        for id in &result.affected_tasks {
            self.queue_worktree_cleanup(id);
            if let Some(task) = self.store.task_by_id(id)? {
                self.announce_task(
                    &change.conversation,
                    &task,
                    Some("scope changed; previous result requires reassessment"),
                );
            }
        }
        Ok(())
    }
}
