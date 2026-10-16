//! Preparation effects execute only after their scoped intent commits.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InspectionState {
    Running,
    Completed,
    Failed,
    Interrupted,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Inspection {
    pub state: InspectionState,
    pub previous_status: Status,
    pub diagnostic: Option<String>,
}

pub(super) fn recover_inspection(run: &mut Run) -> bool {
    if let Some(activation) = &mut run.workspace_activation_attempt
        && activation.state == InspectionState::Running
        && !run.reconciliation_required
        && run.team.is_none()
        && run.pending_edits.is_empty()
        && run
            .managed_workspace
            .as_ref()
            .is_some_and(|w| w.state == crate::workspace::managed::State::Captured)
        && matches!(run.status, Status::Running | Status::Paused)
    {
        activation.state = InspectionState::Interrupted;
        activation.diagnostic = Some("Read-only candidate validation was interrupted; activation was not committed or replayed".into());
        if run.status == Status::Paused {
            run.paused_from = Some(activation.previous_status);
        } else {
            run.status = activation.previous_status;
        }
        run.diagnostic = activation.diagnostic.clone();
        return true;
    }
    let Some(inspection) = &mut run.workspace_inspection else {
        return false;
    };
    if inspection.state != InspectionState::Running
        || run.reconciliation_required
        || run.managed_workspace.is_some()
        || run.team.is_some()
        || !run.pending_edits.is_empty()
        || !matches!(run.status, Status::Running | Status::Paused)
    {
        return false;
    }
    inspection.state = InspectionState::Interrupted;
    inspection.diagnostic = Some("Read-only workspace observation was interrupted; request a fresh inspection. No preparation was admitted or replayed".into());
    run.workspace_plan = None;
    if run.status == Status::Paused {
        run.paused_from = Some(inspection.previous_status);
    } else {
        run.status = inspection.previous_status;
    }
    run.diagnostic = inspection.diagnostic.clone();
    true
}
impl Runtime {
    pub(super) fn work_workspace_activation(
        self: Arc<Self>,
        run: Run,
        scope: String,
        token: Arc<AtomicBool>,
        lease: crate::recovery::WorkspaceLease,
    ) {
        let _lease_scope = crate::process::LeaseScope::new(&lease.fds());
        let previous = run
            .workspace_activation_attempt
            .as_ref()
            .unwrap()
            .previous_status;
        let outcome = (|| -> Result<Run> {
            self.await_action(&run)?;
            require(
                !token.load(Ordering::SeqCst),
                "workspace activation cancelled",
            )?;
            let mut candidate = run.clone();
            candidate.status = previous;
            crate::workspace::managed::activate(&mut candidate, &scope)?;
            require(
                !token.load(Ordering::SeqCst),
                "workspace activation cancelled",
            )?;
            Ok(candidate)
        })();
        let mut store = self.store.lock().expect("store mutex");
        if let Ok(mut current) = store.get(&run.id)
            && current.spec_revision == run.spec_revision
            && current.attempt == run.attempt
        {
            match outcome {
                Ok(candidate)
                    if current.status != Status::Cancelled
                        && !token.load(Ordering::SeqCst)
                        && serde_json::to_value(&current.spec).ok()
                            == serde_json::to_value(&run.spec).ok() =>
                {
                    let mut next = current.clone();
                    let prepare_messages = (|| -> Result<()> {
                        next.communication_ledger.cancel_queued()?;
                        if next.chat_origin && messaging::agents(&next).contains("orchestrator") {
                            messaging::runtime_status(
                                &mut next,
                                "orchestrator",
                                "The accepted task now has a validated private workspace. Its activation does not authorize task execution. At the next explicitly approved orchestration turn, propose the appropriate team, predefined role/verifier pairs, task DAG and risk owners for the current task revision.",
                            )?;
                        }
                        Ok(())
                    })();
                    if let Err(error) = prepare_messages {
                        current.workspace_activation_attempt.as_mut().unwrap().state =
                            InspectionState::Failed;
                        current.diagnostic = Some(format!("Activation not committed: {error}"));
                        if current.status == Status::Paused {
                            current.paused_from = Some(previous);
                        } else {
                            current.status = previous;
                        }
                    } else {
                        next.spec = candidate.spec;
                        next.spec_revision = candidate.spec_revision;
                        next.input_snapshot = candidate.input_snapshot;
                        next.input_observation = candidate.input_observation;
                        next.diff_baseline = candidate.diff_baseline;
                        next.diff_baseline_observation = candidate.diff_baseline_observation;
                        next.managed_workspace = candidate.managed_workspace;
                        next.approvals
                            .push(candidate.approvals.last().unwrap().clone());
                        next.stage = candidate.stage;
                        next.active_session = None;
                        next.workspace_plan = None;
                        next.workspace_activation_attempt.as_mut().unwrap().state =
                            InspectionState::Completed;
                        next.diagnostic = candidate.diagnostic;
                        if next.status == Status::Paused {
                            next.paused_from = Some(candidate.status);
                        } else {
                            next.status = candidate.status;
                        }
                        crate::verification::refresh_provenance(&mut next);
                        current = next;
                    }
                }
                result => {
                    let activation = current.workspace_activation_attempt.as_mut().unwrap();
                    activation.state = InspectionState::Failed;
                    activation.diagnostic =
                        Some(result.err().map(|e| e.to_string()).unwrap_or_else(|| {
                            "Workspace activation interrupted or contract changed".into()
                        }));
                    current.diagnostic = activation.diagnostic.clone();
                    if current.status == Status::Paused {
                        current.paused_from = Some(previous);
                    } else if current.status != Status::Cancelled {
                        current.status = previous;
                    }
                }
            }
            if let Err(error) = store.save(&current, "workspace_activation_finished", None) {
                eprintln!("Cannot persist workspace activation: {error}");
            }
        }
        drop(store);
        drop(_lease_scope);
        drop(lease);
        self.workers.lock().expect("workers mutex").remove(&run.id);
        self.work_changed.notify_all();
    }
    pub(super) fn work_workspace_inspection(
        self: Arc<Self>,
        run: Run,
        token: Arc<AtomicBool>,
        lease: crate::recovery::WorkspaceLease,
    ) {
        let _lease_scope = crate::process::LeaseScope::new(&lease.fds());
        let previous = run.workspace_inspection.as_ref().unwrap().previous_status;
        let outcome = (|| -> Result<crate::workspace::managed::Plan> {
            self.await_action(&run)?;
            let state = self.store.lock().expect("store mutex").data_directory()?;
            let mut view = run.clone();
            view.status = previous;
            let selectors = view.spec.input_files.iter().cloned().collect();
            crate::workspace::managed::plan_cancelled(&view, &state, &selectors, &token)
        })();
        let mut store = self.store.lock().expect("store mutex");
        if let Ok(mut current) = store.get(&run.id)
            && current.spec_revision == run.spec_revision
            && current.attempt == run.attempt
        {
            let inspection = current.workspace_inspection.as_mut().unwrap();
            match outcome {
                Ok(plan)
                    if current.status != Status::Cancelled && !token.load(Ordering::SeqCst) =>
                {
                    current.workspace_plan = Some(plan);
                    inspection.state = InspectionState::Completed;
                    current.diagnostic = Some("Workspace inputs observed; inspect included local changes and approve candidate preparation".into());
                }
                result => {
                    inspection.state = InspectionState::Failed;
                    inspection.diagnostic =
                        Some(result.err().map(|e| e.to_string()).unwrap_or_else(|| {
                            "Workspace inspection cancelled; no candidate created".into()
                        }));
                    current.diagnostic = inspection.diagnostic.clone();
                }
            }
            if current.status == Status::Paused {
                current.paused_from = Some(previous);
            } else if current.status != Status::Cancelled {
                current.status = previous;
            }
            if let Err(error) = store.save(&current, "workspace_inspection_finished", None) {
                eprintln!("Cannot persist workspace inspection: {error}");
            }
        }
        drop(store);
        drop(_lease_scope);
        drop(lease);
        self.workers.lock().expect("workers mutex").remove(&run.id);
        self.work_changed.notify_all();
    }
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
            require(
                crate::workspace::inputs_current(&run)?,
                "task sources changed while preparing the candidate",
            )?;
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
