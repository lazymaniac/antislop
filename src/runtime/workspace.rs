//! Preparation effects execute only after their scoped intent commits.
use super::*;
impl Runtime {
    pub(super) fn work_workspace(
        self: Arc<Self>,
        run: Run,
        token: Arc<AtomicBool>,
        lease: crate::recovery::WorkspaceLease,
    ) {
        let _lease_scope = crate::process::LeaseScope::new(&lease.fds());
        let outcome = (|| -> Result<()> {
            self.await_action(&run)?;
            let workspace = run.managed_workspace.as_ref().unwrap();
            let receipt = crate::workspace::managed::prepare(workspace, &token)?;
            let mut store = self.store.lock().expect("store mutex");
            let mut current = store.get(&run.id)?;
            require(
                current.spec_revision == run.spec_revision
                    && current
                        .managed_workspace
                        .as_ref()
                        .is_some_and(|w| w.id == workspace.id),
                "workspace preparation is stale",
            )?;
            let result = current.managed_workspace.as_mut().unwrap();
            result.receipt = Some(receipt);
            result.state = crate::workspace::managed::State::Captured;
            result.diagnostic = None;
            current.reconciliation_required = false;
            if current.status == Status::Paused {
                current.paused_from = Some(Status::AwaitingStart)
            } else if current.status != Status::Cancelled {
                current.status = Status::AwaitingStart
            }
            current.diagnostic=Some("Private workspace captured; activation and task execution require separate approval".into());
            store.save(&current, "workspace_prepare_finished", None)?;
            Ok(())
        })();
        if let Err(error) = outcome {
            let mut store = self.store.lock().expect("store mutex");
            if let Ok(mut current) = store.get(&run.id)
                && let Some(workspace) = &mut current.managed_workspace
            {
                workspace.state = crate::workspace::managed::State::Unknown;
                workspace.diagnostic = Some(error.to_string());
                current.reconciliation_required = true;
                if current.status == Status::Paused {
                    current.paused_from = Some(Status::Blocked)
                } else if current.status != Status::Cancelled {
                    current.status = Status::Blocked
                }
                current.diagnostic = Some(format!(
                    "Workspace preparation outcome requires inspection; no automatic replay: {error}"
                ));
                if let Err(e) = store.save(&current, "workspace_prepare_unknown", None) {
                    eprintln!("Cannot persist workspace uncertainty: {e}")
                }
            }
        }
        drop(lease);
        self.workers.lock().expect("workers mutex").remove(&run.id);
        self.work_changed.notify_all();
    }
}
