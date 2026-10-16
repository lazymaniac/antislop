use crate::{Result, domain::*, hash, id, providers, require, storage::Store, verification};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
};
pub mod check_resources;
mod control;
pub mod delivery;
pub mod diff_review;
pub mod intake;
mod messaging;
mod orchestration;
pub mod orchestration_recovery;
pub mod team;
pub(crate) mod team_acceptance;
mod turns;
pub mod workspace;

/// Prefix of the rejection raised when another run's unsettled effects block
/// this workspace; clients match it to point at that run instead of retrying.
pub const OVERLAPPING_EFFECTS: &str = "overlapping workspace has unresolved effects";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    InspectDelivery {
        run_id: String,
    },
    DeliverSource {
        run_id: String,
        scope_hash: String,
    },
    InspectWorkspace {
        run_id: String,
    },
    ReconcileSourceDelivery {
        run_id: String,
        scope_hash: String,
        observation_hash: String,
    },
    RestoreSourceDeliveryBackup {
        run_id: String,
        path: String,
        scope_hash: String,
    },
    PrepareWorkspace {
        run_id: String,
        selected: std::collections::BTreeSet<String>,
        scope_hash: String,
    },
    RestoreWorkspaceResult {
        run_id: String,
        scope_hash: String,
    },
    ActivateWorkspace {
        run_id: String,
        scope_hash: String,
    },
    BeginIntake {
        objective: String,
        workspace: String,
        config: crate::config::PinnedConfig,
    },
    Create {
        spec: RunSpec,
    },
    SendMessage {
        run_id: String,
        target_agent_id: String,
        text: String,
    },
    Orchestrate {
        run_id: String,
        scope_hash: String,
    },
    RecoverOrchestration {
        run_id: String,
        round_id: String,
        kind: orchestration_recovery::Kind,
        scope_hash: String,
    },
    AcceptIntake {
        run_id: String,
        round_id: String,
        scope_hash: String,
    },
    AcceptTeam {
        run_id: String,
        round_id: String,
        scope_hash: String,
    },
    StepTeam {
        run_id: String,
        scope_hash: String,
    },
    AcceptTeamStage {
        run_id: String,
        stage: crate::team::TaskStage,
        task_ids: Vec<String>,
        scope_hash: String,
    },
    ReconcileWaveEffects {
        run_id: String,
        task_ids: Vec<String>,
        scope_hash: String,
    },
    ReconcileTaskEffects {
        run_id: String,
        task_id: String,
        scope_hash: String,
    },
    RestoreTaskResult {
        run_id: String,
        task_id: String,
        scope_hash: String,
    },
    RestoreTaskCheckResult {
        run_id: String,
        invocation_id: String,
        scope_hash: String,
    },
    RefreshTaskSources {
        run_id: String,
        task_id: String,
        scope_hash: String,
    },
    RetryTaskVerification {
        run_id: String,
        task_id: String,
        scope_hash: String,
    },
    ResumeTaskSession {
        run_id: String,
        task_id: String,
        scope_hash: String,
    },
    ReplaceTaskSession {
        run_id: String,
        task_id: String,
        scope_hash: String,
    },
    RepairTask {
        run_id: String,
        task_id: String,
        plan: String,
        scope_hash: String,
    },
    ChangeMode {
        run_id: String,
        mode: crate::config::Mode,
        expected_policy_revision: u64,
    },
    ApproveStart {
        run_id: String,
        scope_hash: String,
    },
    AcceptResult {
        run_id: String,
        scope_hash: String,
    },
    ApproveRepair {
        run_id: String,
        scope_hash: String,
    },
    Advance {
        run_id: String,
    },
    Check {
        run_id: String,
        argv: Vec<String>,
        scope_hash: String,
    },
    RetryVerification {
        run_id: String,
        artifact_hash: String,
        scope_hash: String,
    },
    Pause {
        run_id: String,
    },
    Resume {
        run_id: String,
    },
    Cancel {
        run_id: String,
    },
    Reconcile {
        run_id: String,
        scope_hash: String,
        input_hash: String,
    },
    Revise {
        run_id: String,
        expected_revision: u64,
        spec: RunSpec,
    },
    AddElementComment {
        run_id: String,
        target: crate::comments::ElementRef,
        text: String,
    },
    DeleteElementComment {
        run_id: String,
        comment_id: String,
    },
    ResolveElementComment {
        run_id: String,
        comment_id: String,
        resolved: bool,
    },
    AddDiffComment {
        run_id: String,
        path: String,
        line_number: Option<usize>,
        text: String,
        author: String,
    },
    DeleteDiffComment {
        run_id: String,
        comment_id: String,
    },
    ResolveDiffComment {
        run_id: String,
        comment_id: String,
        resolved: bool,
    },
    ReviewDiff {
        run_id: String,
        scope_hash: String,
    },
}
impl Command {
    fn run_id(&self) -> Option<&str> {
        match self {
            Self::Create { .. } | Self::BeginIntake { .. } => None,
            Self::ApproveStart { run_id, .. }
            | Self::InspectDelivery { run_id, .. }
            | Self::DeliverSource { run_id, .. }
            | Self::ReconcileSourceDelivery { run_id, .. }
            | Self::RestoreSourceDeliveryBackup { run_id, .. }
            | Self::InspectWorkspace { run_id, .. }
            | Self::PrepareWorkspace { run_id, .. }
            | Self::RestoreWorkspaceResult { run_id, .. }
            | Self::ActivateWorkspace { run_id, .. }
            | Self::Orchestrate { run_id, .. }
            | Self::RecoverOrchestration { run_id, .. }
            | Self::AcceptIntake { run_id, .. }
            | Self::AcceptTeam { run_id, .. }
            | Self::StepTeam { run_id, .. }
            | Self::AcceptTeamStage { run_id, .. }
            | Self::ReconcileWaveEffects { run_id, .. }
            | Self::ReconcileTaskEffects { run_id, .. }
            | Self::RestoreTaskResult { run_id, .. }
            | Self::RestoreTaskCheckResult { run_id, .. }
            | Self::RefreshTaskSources { run_id, .. }
            | Self::RetryTaskVerification { run_id, .. }
            | Self::ResumeTaskSession { run_id, .. }
            | Self::ReplaceTaskSession { run_id, .. }
            | Self::RepairTask { run_id, .. }
            | Self::ChangeMode { run_id, .. }
            | Self::SendMessage { run_id, .. }
            | Self::AcceptResult { run_id, .. }
            | Self::ApproveRepair { run_id, .. }
            | Self::Advance { run_id }
            | Self::Check { run_id, .. }
            | Self::RetryVerification { run_id, .. }
            | Self::Pause { run_id }
            | Self::Resume { run_id }
            | Self::Cancel { run_id }
            | Self::Revise { run_id, .. }
            | Self::Reconcile { run_id, .. }
            | Self::AddElementComment { run_id, .. }
            | Self::DeleteElementComment { run_id, .. }
            | Self::ResolveElementComment { run_id, .. }
            | Self::AddDiffComment { run_id, .. }
            | Self::DeleteDiffComment { run_id, .. }
            | Self::ResolveDiffComment { run_id, .. }
            | Self::ReviewDiff { run_id, .. } => Some(run_id),
        }
    }
}
pub struct Runtime {
    pub store: Mutex<Store>,
    pub(crate) management_projection: Arc<crate::ipc::management_projection::Cache>,
    pub(crate) task_projection: Arc<crate::ipc::task_projection::Cache>,
    workers: Mutex<HashMap<String, Arc<AtomicBool>>>,
    work_changed: Condvar,
    check_active: Mutex<bool>,
    check_changed: Condvar,
}
pub(crate) fn messaging_agents(run: &Run) -> std::collections::BTreeSet<String> {
    messaging::agents(run)
}
enum ObservedWork<'a> {
    Execute,
    Verify {
        artifact: &'a Artifact,
        evidence: &'a Evidence,
    },
    Prepared {
        task_input: Option<&'a team::restore::readonly::TaskInput>,
        role: crate::config::RoleKind,
        agent_id: &'a str,
        task_id: &'a str,
        message_ids: &'a [String],
        request: &'a providers::PreparedTurn,
    },
}
impl Runtime {
    pub fn new(mut store: Store) -> Result<Self> {
        for mut run in store.all()? {
            if workspace::recover_inspection(&mut run) {
                store.save(&run, "workspace_inspection_interrupted", None)?;
            }
            if run
                .delivery
                .as_ref()
                .is_some_and(|r| r.state == delivery::State::Inspecting)
                && !run.reconciliation_required
                && (run.status == Status::Running || run.paused_from == Some(Status::Running))
            {
                let record = run.delivery.as_mut().unwrap();
                record.state = delivery::State::Failed;
                record.diagnostic=Some("Read-only delivery inspection interrupted; request a fresh inspection without replay".into());
                if run.status == Status::Paused {
                    run.paused_from = Some(Status::Completed);
                } else {
                    run.status = Status::Completed;
                }
                run.diagnostic = record.diagnostic.clone();
                store.save(&run, "source_delivery_inspection_interrupted", None)?;
            }
            let interrupted =
                run.status == Status::Running || run.paused_from == Some(Status::Running);
            if interrupted
                && let Some(delivery) = &mut run.delivery
                && matches!(
                    delivery.state,
                    delivery::State::Applying | delivery::State::Inspecting
                )
            {
                if delivery.state == delivery::State::Applying {
                    delivery.state = delivery::State::Unknown;
                } else if delivery.state == delivery::State::Inspecting {
                    delivery.state = delivery::State::Failed;
                }
                delivery.diagnostic =
                    Some("Delivery continuation lost; retained effects are not replayed".into());
            }
            if !matches!(run.status, Status::Completed | Status::Cancelled)
                && let Err(error) = run.spec.validate()
            {
                if run.status == Status::Paused {
                    run.paused_from = Some(Status::Blocked);
                } else {
                    run.status = Status::Blocked;
                }
                run.diagnostic = Some(format!(
                    "Persisted run no longer satisfies the current contract; explicitly revise it: {error}"
                ));
                store.save(&run, "spec_migration_required", None)?;
            }
            if run.checklist_version != checklist_version()
                && !matches!(run.status, Status::Completed | Status::Cancelled)
            {
                if run.status == Status::Paused {
                    run.paused_from = Some(Status::Blocked);
                } else {
                    run.status = Status::Blocked;
                }
                run.diagnostic = Some(
                    "Pinned checklist changed; explicitly revise the run before continuing.".into(),
                );
                store.save(&run, "checklist_migration_required", None)?;
            }
            if interrupted {
                for review in &mut run.diff_reviews {
                    if review.state == diff_review::State::Running {
                        review.state = diff_review::State::Interrupted;
                        review.diagnostic = Some(
                            "Daemon continuation lost; retained review is not replayed".into(),
                        );
                        run.active_session = review.previous_active_session.clone();
                    }
                }
                if let Some(workspace) = &mut run.managed_workspace
                    && workspace.state == crate::workspace::managed::State::Intent
                {
                    workspace.state = crate::workspace::managed::State::Unknown;
                    workspace.diagnostic = Some(
                        "Preparation was interrupted; inspect retained result without replay"
                            .into(),
                    );
                }
                for round in &mut run.orchestration_rounds {
                    if round.status == crate::orchestration::planning::RoundStatus::Running {
                        run.active_session = round.previous_active_session.clone();
                        round.status = crate::orchestration::planning::RoundStatus::Interrupted;
                        round.diagnostic=Some("Daemon continuation lost; reconcile before another explicitly approved turn".into());
                    }
                }
                for check in &mut run.task_checks {
                    if check.state == team::checks::CheckState::Intent {
                        check.state = team::checks::CheckState::Unknown;
                    }
                }
                run.session_ledger.interrupted(&mut |_, _| Ok(()))?;
                run.communication_ledger.interrupted(&mut |_, _| Ok(()))?;
                if let Some(team) = &mut run.team {
                    team.recover_interrupted();
                }
                if run.status == Status::Paused {
                    run.paused_from = Some(Status::Blocked);
                } else {
                    run.status = Status::Blocked;
                }
                run.reconciliation_required = true;
                let detail = crate::recovery::inspect(&mut run)
                    .err()
                    .map(|error| format!(" Inspection failed: {error}"))
                    .unwrap_or_default();
                run.diagnostic = Some(format!(
                    "Interrupted attempt: reconcile workspace and persisted provider_session_id before authorizing a new attempt. No automatic replay.{detail}"
                ));
                store.save(&run, "recovery_required", None)?;
            }
            if !interrupted && intake::settle_legacy_directory_capture(&mut run).unwrap_or(false) {
                store.save(&run, "intake_directory_capture_reconciled", None)?;
            }
            if run
                .check_resources
                .records()
                .any(|record| record.unresolved())
            {
                if run.status == Status::Paused {
                    run.paused_from = Some(Status::Blocked);
                } else if run.status != Status::Cancelled {
                    run.status = Status::Blocked;
                }
                run.reconciliation_required = true;
                run.diagnostic=Some("Check-container custody is unsettled; inspect creation/start and confirm owned cleanup before reconciliation. No operation replayed.".into());
                store.save(&run, "check_container_recovery_required", None)?;
            }
        }
        Ok(Self {
            store: Mutex::new(store),
            management_projection: Arc::default(),
            task_projection: Arc::default(),
            workers: Mutex::new(HashMap::new()),
            work_changed: Condvar::new(),
            check_active: Mutex::new(false),
            check_changed: Condvar::new(),
        })
    }
    pub fn dispatch(self: &Arc<Self>, key: &str, command: Command) -> Result<Run> {
        self.dispatch_with_preparation(key, command, true)
    }
    pub(crate) fn dispatch_ipc(self: &Arc<Self>, key: &str, command: Command) -> Result<Run> {
        // The snapshot/mutation guard and this admission are separate reads.
        // A concurrent canonical change must not turn IPC into a cold, slow
        // authentication job after its preparation guard has already passed.
        self.dispatch_with_preparation(key, command, false)
    }
    fn dispatch_with_preparation(
        self: &Arc<Self>,
        key: &str,
        command: Command,
        allow_cold: bool,
    ) -> Result<Run> {
        require(!key.trim().is_empty(), "idempotency key required")?;
        let bytes = serde_json::to_vec(&command)?;
        // A cold direct-runtime caller authenticates outside the store lock.
        // IPC callers prepare this proof asynchronously before mutation. After
        // reacquiring the lock, prepare checks the entire current run and live
        // sources; a concurrent edit cannot reuse the earlier proof.
        let management_authentication = if let Command::RecoverOrchestration { run_id, .. } =
            &command
        {
            let (captured, document_hash) = {
                let store = self.store.lock().expect("store mutex");
                if let Some(run) = store.cached(key, &bytes)? {
                    return Ok(run);
                }
                (store.get(run_id)?, store.run_chunk(run_id, 0, 1)?.0)
            };
            Some(
                match self
                    .management_projection
                    .authenticated(&captured, &document_hash)?
                {
                    Some(authenticated) => authenticated,
                    None if !allow_cold => {
                        let store = self.store.lock().expect("store mutex");
                        if let Some(run) = store.cached(key, &bytes)? {
                            return Ok(run);
                        }
                        return Err(crate::Error::Invalid("management recovery preparation changed; refresh before approving; no mutation admitted".into()));
                    }
                    None => match orchestration_recovery::authenticated::Set::build(&captured) {
                        Ok(authenticated) => Arc::new(authenticated),
                        Err(error) => {
                            // A concurrent identical request may already
                            // have committed while this cold read failed.
                            let store = self.store.lock().expect("store mutex");
                            if let Some(run) = store.cached(key, &bytes)? {
                                return Ok(run);
                            }
                            return Err(error);
                        }
                    },
                },
            )
        } else {
            None
        };
        let team_authentication = if let Command::AcceptTeam { run_id, .. } = &command {
            let (captured, document_hash) = {
                let store = self.store.lock().expect("store mutex");
                if let Some(run) = store.cached(key, &bytes)? {
                    return Ok(run);
                }
                (store.get(run_id)?, store.run_chunk(run_id, 0, 1)?.0)
            };
            Some(
                match self
                    .management_projection
                    .prepared_team(&captured, &document_hash)?
                {
                    Some(prepared) => prepared,
                    None if !allow_cold => {
                        let store = self.store.lock().expect("store mutex");
                        if let Some(run) = store.cached(key, &bytes)? {
                            return Ok(run);
                        }
                        return Err(crate::Error::Invalid("team acceptance preparation changed; refresh before approving; no mutation admitted".into()));
                    }
                    None => match team_acceptance::Prepared::build(&captured) {
                        Ok(prepared) => Arc::new(prepared),
                        Err(error) => {
                            let store = self.store.lock().expect("store mutex");
                            if let Some(run) = store.cached(key, &bytes)? {
                                return Ok(run);
                            }
                            return Err(error);
                        }
                    },
                },
            )
        } else {
            None
        };
        let mut store = self.store.lock().expect("store mutex");
        if let Some(run) = store.cached(key, &bytes)? {
            return Ok(run);
        }
        let mut run = if let Command::Create { spec } = &command {
            let mut spec = spec.clone();
            spec.workspace = std::fs::canonicalize(&spec.workspace)?
                .to_string_lossy()
                .into_owned();
            Run::new(id(), spec)?
        } else if let Command::BeginIntake {
            objective,
            workspace,
            config,
        } = &command
        {
            let mut spec = intake::spec(objective, workspace, config)?;
            spec.workspace = std::fs::canonicalize(&spec.workspace)?
                .to_string_lossy()
                .into_owned();
            Run::new_intake(id(), spec)?
        } else {
            store.get(command.run_id().expect("run command"))?
        };
        require(
            matches!(
                &command,
                Command::Revise { .. }
                    | Command::Reconcile { .. }
                    | Command::Cancel { .. }
                    | Command::AddElementComment { .. }
                    | Command::DeleteElementComment { .. }
                    | Command::ResolveElementComment { .. }
                    | Command::Orchestrate { .. }
            ) || run.checklist_version == checklist_version(),
            "pinned checklist changed; explicit revision required",
        )?;
        require(
            !run.intake
                || matches!(
                    &command,
                    Command::BeginIntake { .. }
                        | Command::AcceptIntake { .. }
                        | Command::SendMessage { .. }
                        | Command::Orchestrate { .. }
                        | Command::RecoverOrchestration { .. }
                        | Command::Revise { .. }
                        | Command::Pause { .. }
                        | Command::Resume { .. }
                        | Command::Cancel { .. }
                        | Command::Reconcile { .. }
                        | Command::AddElementComment { .. }
                        | Command::DeleteElementComment { .. }
                        | Command::ResolveElementComment { .. }
                ),
            "intake is conversational; confirm an executable task contract before starting agents or checks",
        )?;
        let terminal = matches!(run.status, Status::Completed | Status::Cancelled);
        if matches!(
            &command,
            Command::Advance { .. } | Command::Check { .. } | Command::RetryVerification { .. }
        ) {
            crate::workspace::managed::require_active(&run)?;
        }
        require(
            run.team.is_none()
                || !matches!(
                    &command,
                    Command::Advance { .. }
                        | Command::ApproveStart { .. }
                        | Command::ApproveRepair { .. }
                        | Command::AcceptResult { .. }
                        | Command::RetryVerification { .. }
                ),
            "accepted team awaits scheduler execution; fixed-stage progression is disabled",
        )?;
        require(
            !terminal
                || matches!(
                    &command,
                    Command::Reconcile { .. }
                        | Command::InspectDelivery { .. }
                        | Command::DeliverSource { .. }
                        | Command::ReconcileSourceDelivery { .. }
                        | Command::RestoreSourceDeliveryBackup { .. }
                        | Command::AddElementComment { .. }
                        | Command::DeleteElementComment { .. }
                        | Command::ResolveElementComment { .. }
                        | Command::Orchestrate { .. }
                ),
            "run is terminal",
        )?;
        let mut launch = false;
        let mut prepare_workspace = false;
        let mut inspect_workspace = false;
        let mut activate_workspace = None;
        let mut diff_review = false;
        let mut source_delivery = None;
        let mut delivery_leases = vec![];
        let mut supplemental = None;
        let mut retry_verification = None;
        let mut orchestration_previous = None;
        let mut team_step = false;
        let mut team_verification_task = None;
        let mut team_refresh_task = None;
        let mut lease = None;
        match &command {
            Command::ReconcileSourceDelivery {
                scope_hash,
                observation_hash,
                ..
            } => {
                require(
                    !self
                        .workers
                        .lock()
                        .expect("workers mutex")
                        .contains_key(&run.id),
                    "delivery worker is still stopping",
                )?;
                lease = Some(crate::recovery::WorkspaceLease::acquire(
                    &run.delivery
                        .as_ref()
                        .and_then(|r| r.plan.as_ref())
                        .ok_or_else(|| crate::Error::Invalid("delivery plan missing".into()))?
                        .source_root,
                )?);
                delivery::reconcile(&mut run, scope_hash, observation_hash)?;
            }
            Command::RestoreSourceDeliveryBackup {
                path, scope_hash, ..
            } => {
                require(
                    !self
                        .workers
                        .lock()
                        .expect("workers mutex")
                        .contains_key(&run.id),
                    "delivery worker is still stopping",
                )?;
                lease = Some(crate::recovery::WorkspaceLease::acquire(
                    &run.delivery
                        .as_ref()
                        .and_then(|r| r.plan.as_ref())
                        .ok_or_else(|| crate::Error::Invalid("delivery plan missing".into()))?
                        .source_root,
                )?);
                require(
                    delivery::recovery(&run)?.scope_hash == *scope_hash,
                    "source backup restoration scope changed",
                )?;
                record_approval(
                    &mut run,
                    "restore-source-delivery-backup-intent",
                    scope_hash,
                );
                store.save(&run, "source_delivery_backup_restore_intent", None)?;
                delivery::restore(&mut run, path, scope_hash)?;
            }
            Command::InspectDelivery { .. } | Command::DeliverSource { .. } => {
                delivery::eligible(&run)?;
                require(
                    !self
                        .workers
                        .lock()
                        .expect("workers mutex")
                        .contains_key(&run.id),
                    "worker is still stopping",
                )?;
                let source_root = run
                    .managed_workspace
                    .as_ref()
                    .unwrap()
                    .original_spec
                    .workspace
                    .clone();
                let candidate = std::path::Path::new(&run.spec.workspace).canonicalize()?;
                let source = std::path::Path::new(&source_root).canonicalize()?;
                if candidate.starts_with(&source) {
                    delivery_leases.push(crate::recovery::WorkspaceLease::acquire(&source_root)?);
                } else if source.starts_with(&candidate) {
                    delivery_leases.push(crate::recovery::WorkspaceLease::acquire(
                        &run.spec.workspace,
                    )?);
                } else {
                    delivery_leases.push(crate::recovery::WorkspaceLease::acquire(&source_root)?);
                    delivery_leases.push(crate::recovery::WorkspaceLease::acquire(
                        &run.spec.workspace,
                    )?);
                }
                let apply = matches!(&command, Command::DeliverSource { .. });
                if let Command::DeliverSource { scope_hash, .. } = &command {
                    require(
                        delivery::pending(&run)?.scope_hash == *scope_hash,
                        "delivery approval is stale",
                    )?;
                    record_approval(&mut run, "deliver-source", scope_hash);
                    run.delivery.as_mut().unwrap().state = delivery::State::Applying;
                    run.reconciliation_required = true;
                } else {
                    run.delivery = Some(delivery::Record {
                        state: delivery::State::Inspecting,
                        plan: None,
                        effects: std::collections::BTreeMap::new(),
                        diagnostic: None,
                    });
                }
                run.status = Status::Running;
                run.diagnostic = Some(
                    if apply {
                        "Transferring separately approved changes to the original checkout"
                    } else {
                        "Inspecting the complete verified result for separate source delivery"
                    }
                    .into(),
                );
                source_delivery = Some(apply);
                launch = true;
            }
            Command::InspectWorkspace { .. } => {
                crate::workspace::managed::eligible(&run)?;
                require(
                    !self
                        .workers
                        .lock()
                        .expect("workers mutex")
                        .contains_key(&run.id),
                    "worker is still stopping",
                )?;
                lease = Some(crate::recovery::WorkspaceLease::acquire(
                    &run.spec.workspace,
                )?);
                run.workspace_plan = None;
                run.workspace_inspection = Some(workspace::Inspection {
                    state: workspace::InspectionState::Running,
                    previous_status: run.status,
                    diagnostic: None,
                });
                run.status = Status::Running;
                run.diagnostic = Some("Observing accepted workspace inputs and task-owned local changes; no candidate is created yet".into());
                inspect_workspace = true;
                launch = true;
            }
            Command::PrepareWorkspace {
                selected,
                scope_hash,
                ..
            } => {
                require(
                    !self
                        .workers
                        .lock()
                        .expect("workers mutex")
                        .contains_key(&run.id),
                    "worker is still stopping",
                )?;
                let state = store.data_directory()?;
                let plan = if run.workspace_plan.is_some() {
                    let plan = crate::workspace::managed::pending(&run, &state)?;
                    require(
                        &plan.selectors == selected,
                        "workspace selection differs from the displayed proposal",
                    )?;
                    plan.clone()
                } else {
                    crate::workspace::managed::plan(&run, &state, selected)?
                };
                require(
                    plan.scope_hash == *scope_hash,
                    "workspace preparation scope changed",
                )?;
                let workspace_id = store
                    .all()?
                    .iter()
                    .filter_map(|r| r.managed_workspace.as_ref())
                    .find(|w| {
                        w.plan.source.source_device == plan.source.source_device
                            && w.plan.source.source_inode == plan.source.source_inode
                            && w.plan.project_prefix == plan.project_prefix
                    })
                    .map_or_else(id, |w| w.workspace_id.clone());
                run.managed_workspace = Some(crate::workspace::managed::intent(
                    &run,
                    &state,
                    plan,
                    workspace_id,
                )?);
                run.approvals.push(Approval {
                    action: "prepare_workspace".into(),
                    scope_hash: scope_hash.clone(),
                    spec_revision: run.spec_revision,
                    actor: "local_user".into(),
                });
                lease = Some(crate::recovery::WorkspaceLease::acquire(
                    &run.spec.workspace,
                )?);
                run.status = Status::Running;
                run.reconciliation_required = true;
                launch = true;
                prepare_workspace = true;
            }
            Command::RestoreWorkspaceResult { scope_hash, .. } => {
                require(
                    !self
                        .workers
                        .lock()
                        .expect("workers mutex")
                        .contains_key(&run.id),
                    "worker is still stopping",
                )?;
                require(
                    crate::workspace::managed::recovery_scope(&run)? == *scope_hash,
                    "workspace restoration scope changed",
                )?;
                lease = Some(crate::recovery::WorkspaceLease::acquire(
                    &run.spec.workspace,
                )?);
                let workspace = run.managed_workspace.as_mut().unwrap();
                workspace.receipt = Some(crate::workspace::managed::observed_result(workspace)?);
                workspace.state = crate::workspace::managed::State::Captured;
                run.reconciliation_required = false;
                run.status = Status::AwaitingStart;
                run.approvals.push(Approval {
                    action: "restore_workspace_result".into(),
                    scope_hash: scope_hash.clone(),
                    spec_revision: run.spec_revision,
                    actor: "local_user".into(),
                });
            }
            Command::ActivateWorkspace { scope_hash, .. } => {
                require(
                    !self
                        .workers
                        .lock()
                        .expect("workers mutex")
                        .contains_key(&run.id),
                    "worker is still stopping",
                )?;
                require(
                    crate::workspace::managed::activation_scope(&run)? == *scope_hash,
                    "workspace activation scope changed",
                )?;
                lease = Some(crate::recovery::WorkspaceLease::acquire(
                    &run.managed_workspace
                        .as_ref()
                        .ok_or_else(|| crate::Error::Invalid("managed workspace missing".into()))?
                        .receipt
                        .as_ref()
                        .ok_or_else(|| crate::Error::Invalid("workspace result missing".into()))?
                        .project_root,
                )?);
                run.workspace_activation_attempt = Some(workspace::Inspection {
                    state: workspace::InspectionState::Running,
                    previous_status: run.status,
                    diagnostic: None,
                });
                run.status = Status::Running;
                run.diagnostic =
                    Some("Validating the captured private workspace before activation".into());
                activate_workspace = Some(scope_hash.clone());
                launch = true;
            }
            Command::Create { .. } | Command::BeginIntake { .. } => (),
            Command::Orchestrate { scope_hash, .. }
            | Command::RecoverOrchestration { scope_hash, .. } => {
                let previous =
                    if let Command::RecoverOrchestration { round_id, kind, .. } = &command {
                        orchestration_recovery::prepare(
                            &mut run,
                            round_id,
                            *kind,
                            scope_hash,
                            management_authentication
                                .as_deref()
                                .expect("management preflight"),
                        )?
                    } else {
                        orchestration::prepare(&mut run, scope_hash)?
                    };
                require(
                    !self
                        .workers
                        .lock()
                        .expect("workers mutex")
                        .contains_key(&run.id),
                    "worker is still stopping",
                )?;
                if !orchestration::read_only(&run)? {
                    require_no_overlapping_effects(&store, &run)?;
                }
                lease = Some(crate::recovery::WorkspaceLease::acquire(
                    &run.spec.workspace,
                )?);
                orchestration_previous = Some(previous);
                launch = true;
            }
            Command::AcceptIntake {
                round_id,
                scope_hash,
                ..
            } => {
                require(
                    !self
                        .workers
                        .lock()
                        .expect("workers mutex")
                        .contains_key(&run.id),
                    "worker is still stopping",
                )?;
                lease = Some(crate::recovery::WorkspaceLease::acquire(
                    &run.spec.workspace,
                )?);
                intake::accept(&mut run, round_id, scope_hash)?;
            }
            Command::AcceptTeam {
                round_id,
                scope_hash,
                ..
            } => {
                crate::workspace::managed::require_active(&run)?;
                orchestration::accept(
                    &mut run,
                    round_id,
                    scope_hash,
                    team_authentication
                        .as_deref()
                        .expect("team preauthentication"),
                )?;
            }
            Command::AcceptTeamStage {
                stage,
                task_ids,
                scope_hash,
                ..
            } => {
                require(
                    !self
                        .workers
                        .lock()
                        .expect("workers mutex")
                        .contains_key(&run.id),
                    "worker is still stopping",
                )?;
                lease = Some(crate::recovery::WorkspaceLease::acquire(
                    &run.spec.workspace,
                )?);
                require(
                    crate::workspace::inputs_current(&run)?,
                    "workspace changed before stage result acceptance",
                )?;
                team::stage::accept(&mut run, *stage, task_ids, scope_hash)?;
            }
            Command::ReconcileWaveEffects {
                task_ids,
                scope_hash,
                ..
            } => {
                require(
                    !self
                        .workers
                        .lock()
                        .expect("workers mutex")
                        .contains_key(&run.id),
                    "worker is still stopping",
                )?;
                lease = Some(crate::recovery::WorkspaceLease::acquire(
                    &run.spec.workspace,
                )?);
                team::adjudicate::reconcile_wave(&mut run, task_ids, scope_hash)?;
            }
            Command::ReconcileTaskEffects {
                task_id,
                scope_hash,
                ..
            } => {
                require(
                    !self
                        .workers
                        .lock()
                        .expect("workers mutex")
                        .contains_key(&run.id),
                    "worker is still stopping",
                )?;
                lease = Some(crate::recovery::WorkspaceLease::acquire(
                    &run.spec.workspace,
                )?);
                team::adjudicate::reconcile(&mut run, task_id, scope_hash)?;
            }
            Command::RestoreTaskCheckResult {
                invocation_id,
                scope_hash,
                ..
            } => {
                require(
                    !self
                        .workers
                        .lock()
                        .expect("workers mutex")
                        .contains_key(&run.id),
                    "worker is still stopping",
                )?;
                lease = Some(crate::recovery::WorkspaceLease::acquire(
                    &run.spec.workspace,
                )?);
                team::check_recovery::restore(&mut run, invocation_id, scope_hash)?;
            }
            Command::RestoreTaskResult {
                task_id,
                scope_hash,
                ..
            } => {
                require(
                    !self
                        .workers
                        .lock()
                        .expect("workers mutex")
                        .contains_key(&run.id),
                    "worker is still stopping",
                )?;
                lease = Some(crate::recovery::WorkspaceLease::acquire(
                    &run.spec.workspace,
                )?);
                require(
                    crate::workspace::inputs_current(&run)?,
                    "workspace changed before executor result restoration",
                )?;
                team::restore::restore(&mut run, task_id, scope_hash)?;
            }
            Command::StepTeam { scope_hash, .. }
            | Command::RetryTaskVerification { scope_hash, .. }
            | Command::ResumeTaskSession { scope_hash, .. }
            | Command::ReplaceTaskSession { scope_hash, .. }
            | Command::RefreshTaskSources { scope_hash, .. } => {
                require(run.team.is_some(), "no team installed for team execution")?;
                crate::workspace::managed::require_active(&run)?;
                require(
                    !run.team.as_ref().unwrap().checkpoint_uncertain,
                    "scheduler checkpoint requires recovery before team execution",
                )?;
                require(!run.reconciliation_required, "reconciliation required")?;
                require(
                    matches!(
                        run.status,
                        Status::Ready
                            | Status::AwaitingResult
                            | Status::AwaitingStart
                            | Status::ChangesRequested
                            | Status::Blocked
                    ),
                    "run not in a runnable status for team step",
                )?;
                require(
                    crate::workspace::inputs_current(&run)?,
                    "workspace changed; reconcile before stepping team",
                )?;
                if let Command::ReplaceTaskSession { task_id, .. } = &command {
                    if team::replacement::prepare(&mut run, task_id, scope_hash)?
                        == crate::team::TaskOperation::Verify
                    {
                        team_verification_task = Some(task_id.clone());
                    } else {
                        team_refresh_task = Some(task_id.clone());
                    }
                } else if let Command::ResumeTaskSession { task_id, .. } = &command {
                    if team::resume::prepare(&mut run, task_id, scope_hash)?
                        == crate::team::TaskOperation::Verify
                    {
                        team_verification_task = Some(task_id.clone());
                    } else {
                        team_refresh_task = Some(task_id.clone());
                    }
                } else if let Command::RetryTaskVerification { task_id, .. } = &command {
                    team::retry::prepare(&mut run, task_id, scope_hash)?;
                    team_verification_task = Some(task_id.clone());
                } else if let Command::RefreshTaskSources { task_id, .. } = &command {
                    team::refresh::prepare(&mut run, task_id, scope_hash)?;
                    team_refresh_task = Some(task_id.clone());
                } else {
                    require(
                        scope_hash == &run.team_step_scope()?,
                        "team step scope hash mismatch",
                    )?;
                }
                require(
                    !self
                        .workers
                        .lock()
                        .expect("workers mutex")
                        .contains_key(&run.id),
                    "worker is still stopping",
                )?;
                require_no_overlapping_effects(&store, &run)?;
                lease = Some(crate::recovery::WorkspaceLease::acquire(
                    &run.spec.workspace,
                )?);
                record_approval(
                    &mut run,
                    if matches!(command, Command::ReplaceTaskSession { .. }) {
                        "replace_task_session"
                    } else if matches!(command, Command::ResumeTaskSession { .. }) {
                        "resume_task_session"
                    } else if team_verification_task.is_some() {
                        "retry_task_verification"
                    } else if team_refresh_task.is_some() {
                        "refresh_task_sources"
                    } else {
                        "step_team"
                    },
                    scope_hash,
                );
                run.status = Status::Running;
                run.reconciliation_required = true;
                run.attempt = run
                    .attempt
                    .checked_add(1)
                    .ok_or_else(|| crate::Error::Invalid("run attempt counter exhausted".into()))?;
                run.provider_failure = None;
                team_step = true;
                launch = true;
            }
            Command::RepairTask {
                task_id,
                plan,
                scope_hash,
                ..
            } => {
                require(run.team.is_some(), "no team installed")?;
                require(!run.reconciliation_required, "reconcile before repair")?;
                require(
                    crate::workspace::inputs_current(&run)?,
                    "workspace changed; reconcile before repair",
                )?;
                require(
                    scope_hash == &run.team_repair_scope(task_id, plan)?,
                    "task repair scope hash mismatch",
                )?;
                let team = run.team.as_mut().unwrap();
                let task = team
                    .tasks
                    .iter()
                    .find(|t| &t.id == task_id)
                    .ok_or_else(|| crate::Error::Invalid("unknown task".into()))?;
                let review = task
                    .review
                    .as_ref()
                    .ok_or_else(|| crate::Error::Invalid("task has no rejected review".into()))?;
                let finding_ids = review.finding_ids.clone();
                team.request_repair(
                    task_id,
                    crate::team::RepairRound {
                        finding_ids,
                        plan: plan.clone(),
                    },
                )?;
                record_approval(&mut run, "repair_task", scope_hash);
                run.status = Status::Ready;
            }
            Command::ChangeMode {
                mode,
                expected_policy_revision,
                ..
            } => {
                require(
                    run.status != Status::Running
                        && !self
                            .workers
                            .lock()
                            .expect("workers mutex")
                            .contains_key(&run.id),
                    "finish or stop the active worker before changing its future autonomy mode",
                )?;
                require(
                    !run.reconciliation_required
                        && run
                            .team
                            .as_ref()
                            .is_none_or(|team| !team.checkpoint_uncertain),
                    "reconcile unknown effects before changing autonomy mode",
                )?;
                run.mode_configurations()?;
                let pinned = run
                    .spec
                    .config
                    .as_ref()
                    .ok_or_else(|| crate::Error::Invalid("no pinned config".into()))?;
                require(
                    pinned.config.policy.revision == *expected_policy_revision,
                    "policy revision mismatch",
                )?;
                require(
                    pinned.config.policy.mode != *mode,
                    "autonomy mode is already selected",
                )?;
                let change = crate::policy::ModeChange {
                    spec_revision: run.spec_revision,
                    previous: pinned.clone(),
                    mode: *mode,
                    actor: "local_user".into(),
                };
                let mut spec = run.spec.clone();
                let mut config = pinned.config.clone();
                config.policy.mode = *mode;
                config.policy.revision = config
                    .policy
                    .revision
                    .checked_add(1)
                    .ok_or_else(|| crate::Error::Invalid("policy revision overflow".into()))?;
                spec.config = Some(config.pin()?);
                // A mode switch cannot bypass missing runner capabilities or
                // introduce a configuration that Create/Revise would refuse.
                spec.validate()?;
                if let Some(team) = &mut run.team {
                    team.spec_hash = hash(serde_json::to_vec(&spec)?);
                    team.validate_all(&spec, &spec.config.as_ref().unwrap().config)?;
                }
                run.spec = spec;
                run.mode_changes.push(change);
                run.mode_configurations()?;
            }
            Command::SendMessage {
                target_agent_id,
                text,
                ..
            } => {
                messaging::accept(&mut run, target_agent_id, text)?;
            }
            Command::AddElementComment { target, text, .. } => {
                crate::comments::add(&mut run, target, text)?;
                if messaging::agents(&run).contains("orchestrator") {
                    let comment = run.element_comments.last().unwrap();
                    let notification = format!(
                        "User comment {} on {:?} {}: {}. Inspect element_comments and return a linked reaction; this message grants no additional permissions.",
                        comment.id, target.kind, target.id, text
                    );
                    messaging::accept(&mut run, "orchestrator", &notification)?;
                }
            }
            Command::DeleteElementComment { comment_id, .. } => {
                let comment = run
                    .element_comments
                    .iter_mut()
                    .find(|c| c.id == *comment_id && !c.deleted)
                    .ok_or_else(|| crate::Error::Invalid("unknown element comment".into()))?;
                comment.deleted = true;
            }
            Command::ResolveElementComment {
                comment_id,
                resolved,
                ..
            } => {
                let comment = run
                    .element_comments
                    .iter_mut()
                    .find(|c| c.id == *comment_id && !c.deleted)
                    .ok_or_else(|| crate::Error::Invalid("unknown element comment".into()))?;
                comment.resolved = *resolved;
            }
            Command::AddDiffComment {
                path,
                line_number,
                text,
                author,
                ..
            } => {
                require(!text.trim().is_empty(), "comment text is required")?;
                let comment = DiffComment {
                    id: crate::id(),
                    run_id: run.id.clone(),
                    path: path.clone(),
                    line_number: *line_number,
                    text: text.clone(),
                    author: author.clone(),
                    created_at: format!(
                        "{}",
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis()
                    ),
                    resolved: false,
                };
                run.diff_comments.push(comment);
                if messaging::agents(&run).contains("orchestrator") {
                    let comment = run.diff_comments.last().unwrap();
                    let notification = format!(
                        "User diff comment {} on {}{}: {}. Inspect unresolved_diff_comments and answer the concern or propose a scoped correction. This comment grants no additional permissions.",
                        comment.id,
                        comment.path,
                        comment
                            .line_number
                            .map(|line| format!(":{line}"))
                            .unwrap_or_default(),
                        comment.text,
                    );
                    messaging::accept(&mut run, "orchestrator", &notification)?;
                }
            }
            Command::DeleteDiffComment { comment_id, .. } => {
                require(
                    run.diff_comments.iter().any(|c| c.id == *comment_id),
                    "unknown diff comment",
                )?;
                run.diff_comments.retain(|c| c.id != *comment_id);
            }
            Command::ResolveDiffComment {
                comment_id,
                resolved,
                ..
            } => {
                let comment = run
                    .diff_comments
                    .iter_mut()
                    .find(|c| c.id == *comment_id)
                    .ok_or_else(|| crate::Error::Invalid("unknown diff comment".into()))?;
                comment.resolved = *resolved;
            }
            Command::ReviewDiff { scope_hash, .. } => {
                require(
                    !self
                        .workers
                        .lock()
                        .expect("workers mutex")
                        .contains_key(&run.id),
                    "worker is still stopping",
                )?;
                lease = Some(crate::recovery::WorkspaceLease::acquire(
                    &run.spec.workspace,
                )?);
                diff_review::admit(&mut run, scope_hash)?;
                diff_review = true;
                launch = true;
            }
            Command::ApproveStart { scope_hash, .. } => {
                require(
                    run.status == Status::AwaitingStart,
                    "not awaiting stage start",
                )?;
                approve(&mut run, "start", scope_hash)?;
                run.status = Status::Ready;
            }
            Command::AcceptResult { scope_hash, .. } => {
                require(
                    run.status == Status::AwaitingResult,
                    "not awaiting result acceptance",
                )?;
                let artifact = run
                    .artifacts
                    .last()
                    .ok_or_else(|| crate::Error::Invalid("missing artifact".into()))?;
                let review = run
                    .reviews
                    .last()
                    .ok_or_else(|| crate::Error::Invalid("missing review".into()))?;
                require(
                    crate::workspace::inputs_current(&run)?,
                    "workspace changed after review",
                )?;
                verification::validate_review(&run, artifact, review)?;
                approve(&mut run, "accept", scope_hash)?;
                if let Some(next) = run.stage.next() {
                    run.stage = next;
                    run.status = Status::AwaitingStart;
                } else {
                    run.status = Status::Completed;
                }
            }
            Command::ApproveRepair { scope_hash, .. } => {
                require(
                    !run.reconciliation_required,
                    "reconcile interrupted effects before approving repair",
                )?;
                require(
                    matches!(run.status, Status::ChangesRequested | Status::Blocked),
                    "not awaiting repair",
                )?;
                require(
                    crate::workspace::inputs_current(&run)?,
                    "workspace changed; explicitly revise before repair",
                )?;
                approve(&mut run, "repair", scope_hash)?;
                run.status = Status::Ready;
            }
            Command::Advance { .. } => {
                require(
                    run.team.is_none(),
                    "accepted team awaits scheduler execution; fixed-stage progression is disabled",
                )?;
                run.spec.validate()?;
                require(run.status == Status::Ready, "stage or repair not approved")?;
                require(!run.reconciliation_required, "reconciliation is required")?;
                require_no_overlapping_effects(&store, &run)?;
                lease = Some(crate::recovery::WorkspaceLease::acquire(
                    &run.spec.workspace,
                )?);
                run.reconciliation_required = true;
                run.status = Status::Running;
                run.attempt += 1;
                run.active_session = None;
                run.provider_failure = None;
                launch = true;
            }
            Command::Check {
                argv, scope_hash, ..
            } => {
                let action = Run::check_action(argv)?;
                run.spec.validate()?;
                if let Some(pinned) = &run.spec.config {
                    pinned
                        .config
                        .policy
                        .authorize_local(crate::config::LocalAction::Check, true)?;
                    require(
                        pinned.config.policy.mode == crate::config::Mode::Supervised,
                        "supplemental checks require explicitly supervised local execution",
                    )?;
                }
                require(
                    matches!(
                        run.status,
                        Status::AwaitingStart
                            | Status::Ready
                            | Status::ChangesRequested
                            | Status::Blocked
                            | Status::AwaitingResult
                    ),
                    "supplemental check requires a stopped nonterminal run",
                )?;
                require(!run.reconciliation_required, "reconciliation is required")?;
                require(
                    !self
                        .workers
                        .lock()
                        .expect("workers mutex")
                        .contains_key(&run.id),
                    "worker is still stopping",
                )?;
                require_no_overlapping_effects(&store, &run)?;
                lease = Some(crate::recovery::WorkspaceLease::acquire(
                    &run.spec.workspace,
                )?);
                require(
                    crate::workspace::inputs_current(&run)?,
                    "workspace changed; reconcile or revise before supplemental check",
                )?;
                approve(&mut run, &action, scope_hash)?;
                supplemental = Some((argv.clone(), run.status));
                run.status = Status::Running;
                run.reconciliation_required = true;
                launch = true;
            }
            Command::RetryVerification {
                artifact_hash,
                scope_hash,
                ..
            } => {
                run.spec.validate()?;
                require(
                    matches!(run.status, Status::Blocked | Status::ChangesRequested),
                    "verification retry requires a blocked or rejected run",
                )?;
                require(
                    !run.reconciliation_required,
                    "reconcile interrupted effects before verification retry",
                )?;
                require(
                    !self
                        .workers
                        .lock()
                        .expect("workers mutex")
                        .contains_key(&run.id),
                    "worker is still stopping",
                )?;
                let session = run
                    .active_session
                    .as_ref()
                    .and_then(|id| run.session_ledger.sessions.get(id))
                    .ok_or_else(|| {
                        crate::Error::Invalid("failed verifier session is missing".into())
                    })?;
                require(
                    session.scope.role == crate::config::RoleKind::Verifier
                        && session.scope.task_id.is_none()
                        && session.scope.stage == run.stage
                        && session.scope.spec_revision == run.spec_revision,
                    "only current main verification can be retried",
                )?;
                let artifact = run
                    .artifacts
                    .iter()
                    .rev()
                    .find(|artifact| {
                        artifact.stage == run.stage
                            && artifact.spec_revision == run.spec_revision
                            && artifact.attempt == run.attempt
                            && artifact.hash == *artifact_hash
                    })
                    .cloned()
                    .ok_or_else(|| {
                        crate::Error::Invalid("current artifact is missing or stale".into())
                    })?;
                require(
                    artifact.hash == hash(&artifact.content)
                        && run
                            .provenance
                            .get(&format!("artifact:{}", artifact.id))
                            .is_some_and(|node| {
                                node.kind == ProvenanceKind::Artifact
                                    && node.spec_revision == run.spec_revision
                                    && node.content_hash == artifact.hash
                                    && node.superseded_by.is_none()
                            })
                        && !verification::needs_context_refresh(&run)
                        && !verification::needs_solution_refresh(&run),
                    "artifact or prerequisite provenance is not current",
                )?;
                if session.state == crate::sessions::TurnState::Completed {
                    retryable_completed_review(&run, &artifact, session)?;
                } else {
                    require(
                        run.status == Status::Blocked
                            && matches!(
                                session.state,
                                crate::sessions::TurnState::Failed
                                    | crate::sessions::TurnState::Stopped
                                    | crate::sessions::TurnState::DeliveryUnknown
                            ),
                        "only interrupted or malformed completed verification can be retried",
                    )?;
                }
                let author = run
                    .session_ledger
                    .sessions
                    .get(&artifact.session_id)
                    .ok_or_else(|| {
                        crate::Error::Invalid("artifact author session is missing".into())
                    })?;
                let turn = author
                    .current_turn_id
                    .as_ref()
                    .and_then(|id| run.session_ledger.turns.get(id))
                    .ok_or_else(|| {
                        crate::Error::Invalid("artifact author turn is missing".into())
                    })?;
                require(
                    author.scope.role == crate::config::RoleKind::Executor
                        && author.scope.stage == run.stage
                        && author.scope.spec_revision == run.spec_revision
                        && author.state == crate::sessions::TurnState::Completed
                        && turn.state == crate::sessions::TurnState::Completed
                        && turn.provider_completed
                        && turn.session_id == artifact.session_id
                        && turn.output.as_deref() == Some(artifact.content.as_str())
                        && turn.output_ref.as_ref().is_some_and(|reference| {
                            reference.hash == artifact.hash
                                && reference.document == format!("turn:{}", turn.id)
                        }),
                    "artifact has no completed durable executor output",
                )?;
                require(
                    run.evidence
                        .iter()
                        .filter(|receipt| receipt.artifact_hash == artifact.hash)
                        .all(|receipt| receipt.success),
                    "failed artifact evidence requires repair",
                )?;
                if matches!(
                    run.stage,
                    Stage::Execution | Stage::Validation | Stage::Integration
                ) && !run.spec.check_command.is_empty()
                {
                    let source =
                        format!("runner:{}", serde_json::to_string(&run.spec.check_command)?);
                    require(
                        run.evidence.iter().any(|receipt| {
                            receipt.source == source
                                && receipt.artifact_hash == artifact.hash
                                && receipt.success
                                && receipt.output_hash == hash(&receipt.output)
                                && verification::evidence_is_current(&run, &artifact, receipt)
                        }),
                        "current artifact runner evidence is missing",
                    )?;
                }
                require_no_overlapping_effects(&store, &run)?;
                lease = Some(crate::recovery::WorkspaceLease::acquire(
                    &run.spec.workspace,
                )?);
                require(
                    crate::workspace::inputs_current(&run)?,
                    "workspace changed; reconcile or revise before verification retry",
                )?;
                approve(&mut run, &format!("verify:{artifact_hash}"), scope_hash)?;
                retry_verification = Some(artifact);
                run.status = Status::Running;
                run.reconciliation_required = true;
                run.provider_failure = None;
                launch = true;
            }
            Command::Pause { .. } => {
                require(run.status != Status::Paused, "already paused")?;
                run.paused_from = Some(run.status);
                run.status = Status::Paused;
            }
            Command::Resume { .. } => {
                require(run.status == Status::Paused, "not paused")?;
                run.status = run.paused_from.take().expect("paused state");
            }
            Command::Cancel { .. } => {
                for round in &mut run.orchestration_rounds {
                    if round.status == crate::orchestration::planning::RoundStatus::Running {
                        round.status = crate::orchestration::planning::RoundStatus::Interrupted;
                        round.diagnostic =
                            Some("Cancelled; prior outputs and effects retained".into());
                    }
                }
                if let Some(team) = &mut run.team {
                    for task in &mut team.tasks {
                        if task.state != crate::team::TaskState::Verified {
                            task.state = crate::team::TaskState::Cancelled;
                        }
                    }
                }
                run.communication_ledger.cancel_queued()?;
                run.communication_ledger.interrupted(&mut |_, _| Ok(()))?;
                run.status = Status::Cancelled;
                run.paused_from = None;
            }
            Command::Reconcile {
                scope_hash,
                input_hash,
                ..
            } => {
                require(
                    !run.check_resources
                        .records()
                        .any(|record| record.unresolved()),
                    "check container cleanup is unconfirmed; source-only reconciliation cannot settle execution resources",
                )?;
                require(
                    !run.delivery
                        .as_ref()
                        .is_some_and(|r| r.state == delivery::State::Unknown),
                    "source delivery effects need dedicated inspection; generic input reconciliation cannot settle them",
                )?;
                require(
                    run.reconciliation_required,
                    "run does not require reconciliation",
                )?;
                require(
                    !self
                        .workers
                        .lock()
                        .expect("workers mutex")
                        .contains_key(&run.id),
                    "worker is still stopping",
                )?;
                lease = Some(crate::recovery::WorkspaceLease::acquire(
                    &run.spec.workspace,
                )?);
                let observation = crate::recovery::observe(&run)?;
                require(
                    observation.hash == *input_hash,
                    "observed workspace hash is stale",
                )?;
                approve(&mut run, "reconcile", scope_hash)?;
                crate::recovery::inspect(&mut run)?;
                let output = serde_json::json!({"edits":run.pending_edits,"observed_inputs":observation.inputs}).to_string();
                run.evidence.push(Evidence {
                    input_hash: Some(hash(serde_json::to_vec(&run.input_snapshot)?)),
                    id: id(),
                    artifact_hash: hash(&output),
                    spec_revision: run.spec_revision,
                    source: "runtime:edit-reconciliation".into(),
                    output_hash: hash(&output),
                    output,
                    success: true,
                });
                let unreadable = observation.readable.is_none();
                let input_type_changed = run.input_observation.as_ref().is_some_and(|old| {
                    observation.inputs.iter().any(|(path, state)| {
                        matches!(state,
                        crate::recovery::InputObservation::Readable {kind:Some(kind), ..}
                        if old.entries.get(path).is_none_or(|entry|entry.kind != *kind))
                    })
                });
                // Task-worktree effects are private; the project only changes
                // through recorded integrations and user edits.
                let worktrees =
                    run.team.is_some() && team::views::enabled(&run) && run.task_views.is_some();
                if worktrees {
                    team::views::restore_worktrees(&run)?;
                }
                if !worktrees
                    && (observation.readable.as_ref() != Some(&run.input_snapshot)
                        || input_type_changed)
                    || run
                        .pending_edits
                        .iter()
                        .any(|receipt| receipt.state != crate::recovery::EditState::Applied)
                {
                    if let Some(current) = observation.readable.filter(|_| !input_type_changed) {
                        run.input_snapshot = current;
                        run.input_observation = observation.source_types;
                    }
                    run.spec_revision += 1;
                    run.team = None;
                    run.team_risk_owners.clear();
                    run.communication_ledger.cancel_queued()?;
                    run.risk_coverage = risk_coverage();
                    if !terminal {
                        run.stage = Stage::Discovery;
                        run.status = if unreadable || input_type_changed {
                            Status::Blocked
                        } else {
                            Status::AwaitingStart
                        };
                        run.paused_from = None;
                    }
                }
                run.pending_edits.clear();
                run.reconciliation_required = false;
                verification::refresh_provenance(&mut run);
                run.diagnostic = Some(if input_type_changed {
                    "Effects reconciled without replay. An input changed between file and directory; explicitly revise the input contract before continuing. Original source/type observations remain retained."
                } else if unreadable {
                    "Effects reconciled without replay. Explicitly revise the pinned input set before continuing; some files are missing or unreadable."
                } else { "Effects reconciled without replay; prior artifacts retained as history." }.into());
            }
            Command::Revise {
                spec,
                expected_revision,
                ..
            } => {
                require(run.spec_revision == *expected_revision, "revision conflict")?;
                require(
                    !run.reconciliation_required,
                    "reconcile interrupted effects before revision",
                )?;
                require(
                    run.status != Status::Running && run.paused_from != Some(Status::Running),
                    "pause and cancel active work before revision",
                )?;
                spec.validate()?;
                require(
                    std::fs::canonicalize(&spec.workspace)?.to_string_lossy() == run.spec.workspace,
                    "workspace change requires a new run",
                )?;
                run.spec = spec.clone();
                run.intake = false;
                run.spec.workspace = std::fs::canonicalize(&spec.workspace)?
                    .to_string_lossy()
                    .into_owned();
                let capture = crate::workspace::observation::capture(
                    std::path::Path::new(&run.spec.workspace),
                    &spec.input_files,
                )?;
                run.input_snapshot = capture.snapshot;
                run.input_observation = Some(capture.observation);
                run.diff_baseline = Some(run.input_snapshot.clone());
                run.diff_baseline_observation = run.input_observation.clone();
                run.workspace_plan = None;
                run.workspace_inspection = None;
                run.workspace_activation_attempt = None;
                run.spec_revision += 1;
                run.team = None;
                run.team_risk_owners.clear();
                run.communication_ledger.cancel_queued()?;
                run.checklist_version = checklist_version();
                run.risk_coverage = risk_coverage();
                verification::refresh_provenance(&mut run);
                run.stage = Stage::Discovery;
                run.status = Status::AwaitingStart;
                run.paused_from = None;
                run.diagnostic =
                    Some("Previous approvals and evidence are stale; history retained.".into());
            }
        }
        // Register before committing Running; Cancel cannot race the token registration.
        let token = if launch {
            let token = Arc::new(AtomicBool::new(false));
            self.workers
                .lock()
                .expect("workers mutex")
                .insert(run.id.clone(), token.clone());
            Some(token)
        } else {
            None
        };
        if let Err(error) = store.save(&run, command_kind(&command), Some((key, &bytes))) {
            if launch {
                self.workers.lock().expect("workers mutex").remove(&run.id);
            }
            return Err(error);
        }
        // Stop processes only after cancellation is durable. A failed mutation
        // must not interrupt work that is still authorized by persisted state.
        if matches!(&command, Command::Cancel { .. }) {
            self.signal(&run.id);
        }
        drop(store);
        if matches!(&command, Command::Resume { .. } | Command::Cancel { .. }) {
            self.work_changed.notify_all();
        }
        if let Some(token) = token {
            let runtime = self.clone();
            let captured = run.clone();
            std::thread::spawn(move || {
                if let Some(apply) = source_delivery {
                    runtime.work_delivery(captured, apply, token, delivery_leases);
                    return;
                }
                let lease = lease.expect("launch lease");
                if let Some(scope) = activate_workspace {
                    runtime.work_workspace_activation(captured, scope, token, lease);
                } else if inspect_workspace {
                    runtime.work_workspace_inspection(captured, token, lease);
                } else if diff_review {
                    runtime.work_diff_review(captured, token, lease);
                } else if prepare_workspace {
                    runtime.work_workspace(captured, token, lease);
                } else if team_step {
                    runtime.work_team(
                        captured,
                        token,
                        lease,
                        team_verification_task,
                        team_refresh_task,
                    );
                } else if let Some(previous) = orchestration_previous {
                    runtime.work_orchestration(captured, previous, token, lease);
                } else if let Some((argv, previous)) = supplemental {
                    runtime.work_check(captured, argv, previous, token, lease);
                } else if let Some(artifact) = retry_verification {
                    runtime.work_verification(captured, artifact, token, lease);
                } else {
                    runtime.work(captured, token, lease);
                }
            });
        }
        Ok(run)
    }
    fn signal(&self, run_id: &str) {
        self.check_changed.notify_all();
        if let Some(token) = self.workers.lock().expect("workers mutex").get(run_id) {
            token.store(true, Ordering::SeqCst);
        }
    }
    fn work_check(
        self: Arc<Self>,
        mut run: Run,
        argv: Vec<String>,
        previous: Status,
        token: Arc<AtomicBool>,
        lease: crate::recovery::WorkspaceLease,
    ) {
        let _lease_scope = crate::process::LeaseScope::new(&lease.fds());
        let outcome = (|| -> Result<()> {
            require(
                !token.load(Ordering::SeqCst),
                "supplemental check cancelled before launch",
            )?;
            let root = std::path::Path::new(&run.spec.workspace);
            let captured_inputs =
                crate::workspace::observation::capture(root, &run.spec.input_files)?;
            require(
                crate::workspace::captured_inputs_match(&run, &captured_inputs)?,
                "inputs changed before supplemental check",
            )?;
            let before = captured_inputs.snapshot;
            let snapshot = serde_json::to_string(&before)?;
            let input_hash = hash(&snapshot);
            run.evidence.push(Evidence {
                id: id(),
                artifact_hash: input_hash.clone(),
                spec_revision: run.spec_revision,
                source: "runtime:input-snapshot".into(),
                output_hash: input_hash.clone(),
                output: snapshot,
                success: true,
                input_hash: Some(input_hash.clone()),
            });
            self.persist_progress(&run)?;
            self.await_action(&run)?;
            let _check_resources = self.acquire_check(&token)?;
            let result = crate::workspace::check(root, &argv, token.clone());
            let after_capture = crate::workspace::observation::capture(root, &run.spec.input_files);
            let unchanged = after_capture.as_ref().is_ok_and(|after| {
                after.snapshot == before && after.observation == captured_inputs.observation
            });
            let after = after_capture.map(|capture| capture.snapshot);
            let binding = serde_json::json!({"argv_hash":hash(serde_json::to_vec(&argv)?),
                "before_hash":input_hash,
                "after_hash":after.as_ref().ok().map(|after|serde_json::to_vec(after).map(hash)).transpose()?,
                "unchanged":unchanged}).to_string();
            run.evidence.push(Evidence {
                id: id(),
                artifact_hash: input_hash.clone(),
                spec_revision: run.spec_revision,
                source: "runtime:supplemental-check-inputs".into(),
                output_hash: hash(&binding),
                output: binding,
                success: unchanged,
                input_hash: Some(input_hash.clone()),
            });
            if let Ok((success, output)) = &result {
                let artifact_hash = run
                    .artifacts
                    .iter()
                    .rev()
                    .find(|artifact| {
                        artifact.stage == run.stage && artifact.spec_revision == run.spec_revision
                    })
                    .map_or_else(|| input_hash.clone(), |artifact| artifact.hash.clone());
                run.evidence.push(Evidence {
                    id: id(),
                    artifact_hash,
                    spec_revision: run.spec_revision,
                    source: format!("runner:{}", serde_json::to_string(&argv)?),
                    output_hash: hash(output),
                    output: output.clone(),
                    success: *success && unchanged,
                    input_hash: Some(input_hash),
                });
            }
            result?;
            require(
                unchanged,
                "supplemental check changed or obscured pinned inputs; reconcile before continuing",
            )?;
            require(
                !token.load(Ordering::SeqCst),
                "supplemental check cancelled; effects require reconciliation",
            )?;
            run.status = previous;
            run.reconciliation_required = false;
            Ok(())
        })();
        if let Err(error) = outcome {
            run.status = Status::Blocked;
            run.diagnostic = Some(error.to_string());
        }
        let mut store = self.store.lock().expect("store mutex");
        if let Ok(mut current) = store.get(&run.id)
            && current.spec_revision == run.spec_revision
            && current.attempt == run.attempt
        {
            current.evidence = run.evidence;
            if current.status == Status::Cancelled {
                current.reconciliation_required = true;
            } else if token.load(Ordering::SeqCst) {
                current.reconciliation_required = true;
                current.diagnostic =
                    Some("Supplemental check stopped; effects require reconciliation".into());
                if current.status == Status::Paused {
                    current.paused_from = Some(Status::Blocked);
                } else {
                    current.status = Status::Blocked;
                    current.paused_from = None;
                }
            } else {
                current.reconciliation_required = run.reconciliation_required;
                current.diagnostic = run.diagnostic;
                if current.status == Status::Paused {
                    current.paused_from = Some(run.status);
                } else {
                    current.status = run.status;
                    current.paused_from = None;
                }
            }
            if let Err(error) = store.save(&current, "supplemental_check_finished", None) {
                eprintln!("Cannot persist supplemental check: {error}");
            }
        }
        drop(_lease_scope);
        drop(lease);
        self.workers.lock().expect("workers mutex").remove(&run.id);
    }
    fn work_verification(
        self: Arc<Self>,
        mut run: Run,
        artifact: Artifact,
        token: Arc<AtomicBool>,
        lease: crate::recovery::WorkspaceLease,
    ) {
        let _lease_scope = crate::process::LeaseScope::new(&lease.fds());
        let outcome = (|| -> Result<()> {
            require(
                !token.load(Ordering::SeqCst),
                "verification retry cancelled",
            )?;
            let workspace = std::path::PathBuf::from(&run.spec.workspace);
            let root = workspace.as_path();
            let captured_inputs =
                crate::workspace::observation::capture(root, &run.spec.input_files)?;
            require(
                crate::workspace::captured_inputs_match(&run, &captured_inputs)?,
                "inputs changed before verification retry",
            )?;
            let inputs = captured_inputs.snapshot;
            let output = serde_json::to_string(&inputs)?;
            run.evidence.push(Evidence {
                id: id(),
                artifact_hash: artifact.hash.clone(),
                spec_revision: run.spec_revision,
                source: "runtime:input-snapshot".into(),
                output_hash: hash(&output),
                input_hash: Some(hash(&output)),
                output,
                success: true,
            });
            let evidence = verification::observe(&artifact);
            run.evidence.push(evidence.clone());
            let verifier = providers::build_for(&run, crate::config::RoleKind::Verifier)?;
            self.review_artifact(&mut run, &artifact, &evidence, verifier.as_ref(), token)?;
            require(
                crate::workspace::inputs_current(&run)?,
                "inputs changed during verification retry; reconcile before continuing",
            )?;
            Ok(())
        })();
        drop(_lease_scope);
        self.finish_attempt(run, outcome, Some(lease));
    }
    fn review_artifact(
        &self,
        run: &mut Run,
        artifact: &Artifact,
        evidence: &Evidence,
        verifier: &dyn providers::Provider,
        token: Arc<AtomicBool>,
    ) -> Result<()> {
        run.evidence
            .push(crate::orchestration::observe(run, &artifact.hash)?);
        self.persist_progress(run)?;
        let view = run.clone();
        let (logical, receipt) = self.observed_turn(
            run,
            &view,
            verifier,
            ObservedWork::Verify { artifact, evidence },
            token,
        )?;
        let mut review = verifier.parse_review(&view, artifact, &receipt)?;
        review.verifier_session = logical;
        let result = verification::validate_review(run, artifact, &review);
        if result.is_ok() {
            verification::record_review(run, artifact, &review)?;
        }
        run.reviews.push(review);
        match result {
            Ok(()) => {
                run.status = Status::AwaitingResult;
                run.diagnostic = None;
            }
            Err(error) => {
                run.status = Status::ChangesRequested;
                run.diagnostic = Some(error.to_string());
            }
        }
        Ok(())
    }
    fn observed_turn(
        &self,
        run: &mut Run,
        view: &Run,
        provider: &dyn providers::Provider,
        work: ObservedWork<'_>,
        token: Arc<AtomicBool>,
    ) -> Result<(String, providers::TurnReceipt)> {
        use crate::{
            config::RoleKind,
            sessions::{OutputRef, SessionRecord, SessionScope, TurnRecord},
        };
        self.await_action(run)?;
        let capabilities = provider.probe_cancelled(&token)?;
        let role = match &work {
            ObservedWork::Execute => RoleKind::Executor,
            ObservedWork::Verify { .. } => RoleKind::Verifier,
            ObservedWork::Prepared { role, .. } => *role,
        };
        let turn_id = id();
        let default_agent_id = format!(
            "{:?}:{}",
            view.stage,
            if role == RoleKind::Executor {
                "executor"
            } else {
                "verifier"
            }
        );
        let (agent_id, task_id) = match &work {
            ObservedWork::Prepared {
                agent_id, task_id, ..
            } => (agent_id.to_string(), Some(task_id.to_string())),
            _ => (default_agent_id, None),
        };
        let resume = task_id
            .as_deref()
            .and_then(|id| team::resume::admitted(view, id, role));
        let replacement = task_id
            .as_deref()
            .and_then(|id| team::replacement::admitted(view, id, role));
        let management = task_id
            .as_deref()
            .and_then(|id| orchestration_recovery::admitted(view, id, role));
        require(
            management.is_none() || (resume.is_none() && replacement.is_none()),
            "ambiguous management/task recovery",
        )?;
        if let Some(management) = management {
            require(
                management.handoff_hash == hash(serde_json::to_vec(&management.handoff)?)
                    && view.approvals.iter().any(|a| {
                        a.action == format!("recover-orchestration:{}", task_id.as_ref().unwrap())
                            && a.scope_hash == management.approval_scope
                            && a.spec_revision == view.spec_revision
                    }),
                "management recovery handoff/approval changed",
            )?;
            require(
                management.kind != orchestration_recovery::Kind::Resume || capabilities.resume,
                "management exact resume unsupported; explicitly replace the session",
            )?;
        }
        if role == RoleKind::Executor
            && task_id.as_deref().is_some_and(|id| {
                view.team.as_ref().is_some_and(|t| {
                    t.tasks
                        .iter()
                        .any(|task| task.id == id && task.state == crate::team::TaskState::Resuming)
                })
            })
        {
            require(
                resume.is_some() || replacement.is_some(),
                "executor recovery admission is stale; do not start an ordinary new session",
            )?;
        }
        require(
            resume.is_none() || replacement.is_none(),
            "ambiguous native recovery admission",
        )?;
        let logical = management
            .map(|r| r.session_id.clone())
            .or_else(|| {
                resume
                    .map(|r| r.session_id.clone())
                    .or_else(|| replacement.map(|r| r.new_session_id.clone()))
            })
            .unwrap_or_else(id);
        if let Some(replacement) = replacement {
            require(
                view.approvals.iter().any(|a| {
                    a.action == "replace_task_session"
                        && a.scope_hash == replacement.approval_scope
                        && a.spec_revision == view.spec_revision
                }),
                "session replacement has no current scoped approval",
            )?;
            require(
                replacement.handoff.hash()? == replacement.handoff_hash
                    && replacement.handoff.source_hashes == view.source_hashes()
                    && replacement.handoff.spec_revision == view.spec_revision
                    && Some(&replacement.handoff.configuration_hash)
                        == view.spec.config.as_ref().map(|p| &p.content_hash),
                "replacement handoff changed or its sources/configuration are stale",
            )?;
        }
        if let Some(resume) = resume {
            require(
                capabilities.resume,
                "native exact-ID continuation unsupported; explicit replacement required",
            )?;
            require(
                view.approvals.iter().any(|a| {
                    a.action == "resume_task_session"
                        && a.scope_hash == resume.approval_scope
                        && a.spec_revision == view.spec_revision
                }),
                "native continuation has no current scoped approval",
            )?;
        }
        let mut packet = self.message_packet(run, view, &agent_id, &turn_id)?;
        if let ObservedWork::Prepared { message_ids, .. } = &work {
            if let Some(packet) = &mut packet {
                packet
                    .messages
                    .retain(|m| message_ids.contains(&m.message_id));
            }
            packet = packet.filter(|p| !p.messages.is_empty());
        }
        let native_resume = management
            .filter(|r| r.kind == orchestration_recovery::Kind::Resume)
            .and_then(|r| r.native_session_id.as_deref())
            .or_else(|| resume.map(|r| r.native_session_id.as_str()));
        let invocation = providers::ObservedInvocation {
            turn_id: &turn_id,
            run: view,
            resume: native_resume,
            context_packet: packet.as_ref(),
        };
        let mut prepared = match &work {
            ObservedWork::Execute => provider.prepare_execute(&invocation)?,
            ObservedWork::Verify { artifact, evidence } => {
                provider.prepare_verify(&invocation, artifact, evidence)?
            }
            ObservedWork::Prepared { request, .. } => (*request).clone().with_context_for_agents(
                &invocation,
                task_id
                    .as_ref()
                    .and_then(|id| {
                        view.orchestration_rounds.iter().find(|r| {
                            &r.id == id && role == RoleKind::Verifier && !r.review_parts.is_empty()
                        })
                    })
                    .and_then(|r| r.review_known_agents.as_ref()),
            )?,
        };
        if let Some(management) = management {
            orchestration_recovery::decorate(view, &mut prepared, management)?;
        }
        if management.is_none()
            && view.task_prompt_format == Some(crate::team::models::PromptFormat::ExplicitSchema)
            && task_id.as_ref().is_some_and(|id| {
                view.team
                    .as_ref()
                    .is_some_and(|team| team.tasks.iter().any(|t| &t.id == id))
            })
        {
            crate::team::models::append_wire_schema(&mut prepared.prompt, &prepared.schema)?;
        }
        if let Some(replacement) = replacement {
            team::replacement::context::decorate(view, &mut prepared, replacement)?;
        }
        let scope = SessionScope::for_run(
            view,
            agent_id,
            task_id,
            role,
            capabilities.provider,
            capabilities.version,
        )?;
        let session = if management.is_some_and(|r| r.kind == orchestration_recovery::Kind::Resume)
        {
            let original = &view.session_ledger.sessions[&logical];
            require(
                original.scope == scope && original.native_session_id.as_deref() == native_resume,
                "management session version/scope changed; explicitly replace it",
            )?;
            original.clone()
        } else if let Some(resume) = resume {
            let session = view.session_ledger.sessions.get(&logical).ok_or_else(|| {
                crate::Error::Invalid("native continuation session missing".into())
            })?;
            require(
                session.scope == scope
                    && session.native_session_id.as_ref() == Some(&resume.native_session_id),
                "native continuation scope/version/identity changed; explicit replacement required",
            )?;
            prepared.prompt.push_str(&format!("\nNative continuation checkpoint: {}\nContinue this same task attempt from the retained exact native session. Preserve the original constraints and source bindings. Do not repeat already completed effects or claimed checks. Missing evidence still requires independent validation; resume is not task acceptance.", serde_json::to_string(resume)?));
            session.clone()
        } else {
            SessionRecord::new(logical.clone(), scope)?
        };
        let mut turn = TurnRecord::new(
            turn_id.clone(),
            logical.clone(),
            prepared.prompt_hash(),
            prepared.schema_hash()?,
            packet.as_ref().map(|packet| packet.hash()).transpose()?,
        )?;
        if let ObservedWork::Prepared { task_input, .. } = &work {
            turn.task_input = task_input.cloned();
        }
        let captured = run.clone();
        run.session_ledger = self.checkpoint_session(
            &captured,
            &logical,
            packet.as_ref(),
            true,
            |ledger, persist| {
                if let Some(management) =
                    management.filter(|r| r.kind == orchestration_recovery::Kind::Resume)
                {
                    ledger.reserve_resumed(
                        session,
                        turn,
                        management.previous_turn.as_ref().unwrap(),
                        persist,
                    )
                } else if let Some(resume) = resume {
                    ledger.reserve_resumed(session, turn, &resume.previous_turn_id, persist)
                } else {
                    ledger.reserve(session, turn, persist)
                }
            },
        )?;
        run.active_session = Some(logical.clone());
        let mut events = |event: &providers::ProviderEvent| {
            if let providers::ProviderEvent::ContextPrepared { transport, .. } = event {
                require(
                    transport.bundle.scope.run_id == view.id,
                    "native context belongs to a different run",
                )?;
            }
            run.session_ledger = self.checkpoint_session(
                &captured,
                &logical,
                packet.as_ref(),
                false,
                |ledger, persist| ledger.record_provider_event(&turn_id, event, persist),
            )?;
            Ok(())
        };
        let result = match work {
            ObservedWork::Execute => provider.execute_observed(&invocation, token, &mut events),
            ObservedWork::Verify { artifact, evidence } => {
                provider.verify_observed(&invocation, artifact, evidence, token, &mut events)
            }
            ObservedWork::Prepared { .. } => provider.turn_receipt(
                &providers::TurnRequest {
                    turn_id: &turn_id,
                    run: view,
                    prompt: &prepared.prompt,
                    schema: &prepared.schema,
                    resume: native_resume,
                },
                token,
                &mut events,
            ),
        };
        let receipt = match result {
            Ok(receipt) => receipt,
            Err(error) => {
                let kind = if let crate::Error::Provider(failure) = &error {
                    failure.kind
                } else {
                    providers::FailureKind::Process
                };
                // Read the authoritative state: a sibling may have checkpointed
                // since this worker's last callback. A stopped turn stays stopped.
                if let Ok(ledger) = self.checkpoint_session(
                    &captured,
                    &logical,
                    packet.as_ref(),
                    false,
                    |ledger, persist| {
                        if ledger.turns.get(&turn_id).is_some_and(|turn| {
                            turn.state != crate::sessions::TurnState::Stopped
                                && turn.state != crate::sessions::TurnState::Completed
                        }) {
                            ledger.fail(&turn_id, kind, true, persist)?;
                        }
                        Ok(())
                    },
                ) {
                    run.session_ledger = ledger;
                }
                return Err(error);
            }
        };
        run.session_ledger = self.checkpoint_session(
            &captured,
            &logical,
            packet.as_ref(),
            false,
            |ledger, persist| {
                ledger.complete(
                    &turn_id,
                    &receipt,
                    OutputRef {
                        document: format!("turn:{turn_id}"),
                        hash: hash(&receipt.output),
                    },
                    persist,
                )
            },
        )?;
        let current = self.store.lock().expect("store mutex").get(&run.id)?;
        if let Some(reason) = current
            .agent_outboxes
            .get(&turn_id)
            .and_then(|r| r.rejection.as_ref())
        {
            return Err(crate::Error::Invalid(format!(
                "agent outbox rejected: {reason}"
            )));
        }
        Ok((logical, receipt))
    }
    fn work(
        self: Arc<Self>,
        mut run: Run,
        token: Arc<AtomicBool>,
        lease: crate::recovery::WorkspaceLease,
    ) {
        let _lease_scope = crate::process::LeaseScope::new(&lease.fds());
        let outcome = (|| -> Result<()> {
            run.spec.validate()?;
            let provider = providers::build_for(&run, crate::config::RoleKind::Executor)?;
            let verifier = providers::build_for(&run, crate::config::RoleKind::Verifier)?;
            let workspace = std::path::PathBuf::from(&run.spec.workspace);
            let root = workspace.as_path();
            let captured_inputs =
                crate::workspace::observation::capture(root, &run.spec.input_files)?;
            require(
                crate::workspace::captured_inputs_match(&run, &captured_inputs)?,
                "workspace inputs changed before stage; revise explicitly",
            )?;
            let observed_inputs = captured_inputs.snapshot;
            let input_output = serde_json::to_string(&observed_inputs)?;
            let input_hash = hash(&input_output);
            run.evidence.push(Evidence {
                input_hash: Some(hash(serde_json::to_vec(&run.input_snapshot)?)),
                id: id(),
                artifact_hash: input_hash.clone(),
                spec_revision: run.spec_revision,
                source: "runtime:input-snapshot".into(),
                output_hash: input_hash.clone(),
                output: input_output,
                success: true,
            });
            if run.stage == Stage::Discovery && !run.spec.check_command.is_empty() {
                self.await_action(&run)?;
                let _check_resources = self.acquire_check(&token)?;
                let (success, output) =
                    crate::workspace::check(root, &run.spec.check_command, token.clone())?;
                run.evidence.push(Evidence {
                    input_hash: Some(hash(serde_json::to_vec(&run.input_snapshot)?)),
                    id: id(),
                    artifact_hash: input_hash,
                    spec_revision: run.spec_revision,
                    source: format!(
                        "runner:baseline:{}",
                        serde_json::to_string(&run.spec.check_command)?
                    ),
                    output_hash: hash(&output),
                    output,
                    success,
                });
            }
            self.persist_progress(&run)?;
            run.evidence.push(crate::orchestration::observe(
                &run,
                &hash(serde_json::to_vec(&run.input_snapshot)?),
            )?);
            self.persist_progress(&run)?;
            let view = run.clone();
            let (session_id, receipt) = self.observed_turn(
                &mut run,
                &view,
                provider.as_ref(),
                ObservedWork::Execute,
                token.clone(),
            )?;
            let content = receipt.output;
            let artifact = Artifact {
                id: id(),
                stage: run.stage,
                attempt: run.attempt,
                spec_revision: run.spec_revision,
                session_id,
                hash: hash(&content),
                content,
            };
            let evidence = verification::observe(&artifact);
            run.artifacts.push(artifact.clone());
            run.evidence.push(evidence.clone());
            self.persist_progress(&run)?;
            let validated = crate::transport::decode::<ExecutionOutput>(
                artifact.content.as_bytes(),
            )
            .and_then(|output| {
                verification::validate_output(&run, &output)?;
                require(
                    run.stage == Stage::Execution || output.edits.is_empty(),
                    "file edits are only allowed in Execution",
                )?;
                Ok(output)
            });
            let output = match validated {
                Ok(output) => output,
                Err(error) => {
                    // The completed turn and raw output are already durable. A
                    // rejected report before edit intent is a repairable result,
                    // provided the actual pinned inputs remain unchanged.
                    require(
                        crate::workspace::inputs_current(&run)?,
                        "workspace changed during rejected output; reconcile before repair",
                    )?;
                    run.status = Status::ChangesRequested;
                    run.diagnostic = Some(error.to_string());
                    return Ok(());
                }
            };
            verification::register_artifact(&mut run, &artifact, &output)?;
            if run.stage == Stage::Execution {
                if !output.edits.is_empty() {
                    self.await_action(&run)?;
                }
                run.pending_edits = crate::recovery::prepare(&run, &output.edits)?;
                // Intent is durable before the first filesystem effect.
                self.persist_progress(&run)?;
                for (index, edit) in output.edits.iter().enumerate() {
                    self.await_action(&run)?;
                    require(!token.load(Ordering::SeqCst), "cancelled before write")?;
                    let expected = run.pending_edits[index].before_hash.clone();
                    crate::workspace::apply_edit(
                        std::path::Path::new(&run.spec.workspace),
                        edit,
                        &expected,
                    )?;
                    crate::workspace::observation::record_change(
                        root,
                        &mut run.input_snapshot,
                        &mut run.input_observation,
                        edit,
                        &expected,
                    )?;
                    run.pending_edits[index].state = crate::recovery::EditState::Applied;
                    verification::refresh_provenance(&mut run);
                    // A crash before this commit is resolved from the preceding receipt.
                    self.persist_progress(&run)?;
                }
            }
            if matches!(
                run.stage,
                Stage::Execution | Stage::Validation | Stage::Integration
            ) && !run.spec.check_command.is_empty()
            {
                self.await_action(&run)?;
                let _check_resources = self.acquire_check(&token)?;
                let (success, output) = crate::workspace::check(
                    std::path::Path::new(&run.spec.workspace),
                    &run.spec.check_command,
                    token.clone(),
                )?;
                run.evidence.push(Evidence {
                    input_hash: Some(hash(serde_json::to_vec(&run.input_snapshot)?)),
                    id: id(),
                    artifact_hash: artifact.hash.clone(),
                    spec_revision: run.spec_revision,
                    source: format!("runner:{}", serde_json::to_string(&run.spec.check_command)?),
                    output_hash: hash(&output),
                    output,
                    success,
                });
            }
            let current_input = serde_json::to_string(&run.input_snapshot)?;
            run.evidence.push(Evidence {
                input_hash: Some(hash(serde_json::to_vec(&run.input_snapshot)?)),
                id: id(),
                artifact_hash: artifact.hash.clone(),
                spec_revision: run.spec_revision,
                source: "runtime:input-snapshot".into(),
                output_hash: hash(&current_input),
                output: current_input,
                success: true,
            });
            require(
                crate::workspace::inputs_current(&run)?,
                "check command changed pinned inputs; reconcile before review",
            )?;
            run.evidence
                .push(verification::workspace_diff(&run, &artifact)?);
            self.refresh_claims(&mut run, token.clone())?;
            self.review_artifact(
                &mut run,
                &artifact,
                &evidence,
                verifier.as_ref(),
                token.clone(),
            )?;
            Ok(())
        })();
        drop(_lease_scope);
        self.finish_attempt(run, outcome, Some(lease));
    }
    fn finish_attempt(
        &self,
        run: Run,
        outcome: Result<()>,
        lease: Option<crate::recovery::WorkspaceLease>,
    ) {
        self.finish_attempt_with_session(run, outcome, None, lease);
    }
    fn finish_attempt_with_session(
        &self,
        mut run: Run,
        outcome: Result<()>,
        restore_session: Option<Option<String>>,
        lease: Option<crate::recovery::WorkspaceLease>,
    ) {
        let failed = outcome.is_err();
        if outcome.is_ok() {
            run.reconciliation_required = false;
            run.pending_edits.clear();
        }
        if let Err(error) = outcome {
            if let crate::Error::Provider(failure) = &error {
                run.provider_failure = Some(failure.clone());
            }
            run.status = Status::Blocked;
            run.diagnostic = Some(error.to_string());
        }
        let mut store = self.store.lock().expect("store mutex");
        if let Ok(current) = store.get(&run.id)
            && current.spec_revision == run.spec_revision
            && current.attempt == run.attempt
            && current.status != Status::Cancelled
        {
            run.session_ledger = current.session_ledger;
            run.approvals = current.approvals;
            run.active_session = restore_session.unwrap_or(current.active_session);
            run.communication_ledger = current.communication_ledger;
            run.message_bodies = current.message_bodies;
            run.message_packets = current.message_packets;
            run.agent_outboxes = current.agent_outboxes;
            run.element_comments = current.element_comments;
            run.diff_comments = current.diff_comments;
            if failed && let Err(error) = run.communication_ledger.interrupted(&mut |_, _| Ok(())) {
                eprintln!("Cannot record uncertain message delivery: {error}");
                drop(lease);
                self.workers.lock().expect("workers mutex").remove(&run.id);
                return;
            }
            if current.status == Status::Paused {
                run.paused_from = Some(run.status);
                run.status = Status::Paused;
            }
            if let Err(e) = store.save(&run, "attempt_finished", None) {
                eprintln!("Cannot persist attempt: {e}");
            }
        }
        drop(lease);
        self.workers.lock().expect("workers mutex").remove(&run.id);
    }
    /// A source edit cannot renew an old claim by rebinding its hashes. Fresh
    /// context/model executors publish replacements, each with a dedicated
    /// independent stage verifier, before the primary result is reviewed.
    fn refresh_claims(&self, run: &mut Run, token: Arc<AtomicBool>) -> Result<()> {
        for stage in [Stage::Discovery, Stage::Planning] {
            let required = match stage {
                Stage::Discovery => verification::needs_context_refresh(run),
                Stage::Planning => verification::needs_solution_refresh(run),
                _ => false,
            };
            if !required {
                continue;
            }
            require(
                !token.load(Ordering::SeqCst),
                "cancelled before source refresh",
            )?;
            run.evidence.push(crate::orchestration::observe(
                run,
                &hash(serde_json::to_vec(&run.input_snapshot)?),
            )?);
            self.persist_progress(run)?;
            let mut scoped = run.clone();
            scoped.stage = stage;
            scoped.diagnostic = Some("Pinned source changed. Produce a NEW current context/model; do not relabel old facts or reuse contradicted conclusions. Preserve tracked fact sources and invariant IDs; justify changed conclusions using actual current inputs and runner receipts. This refresh may not edit files.".into());
            let executor = providers::build_for(&scoped, crate::config::RoleKind::Executor)?;
            let (session_id, receipt) = self.observed_turn(
                run,
                &scoped,
                executor.as_ref(),
                ObservedWork::Execute,
                token.clone(),
            )?;
            let content = receipt.output;
            let output: ExecutionOutput = crate::transport::decode(content.as_bytes())?;
            let fresh = Artifact {
                id: id(),
                stage,
                attempt: run.attempt,
                spec_revision: run.spec_revision,
                session_id,
                hash: hash(&content),
                content,
            };
            verification::register_refresh(run, fresh.clone(), &output)?;
            run.evidence
                .push(crate::orchestration::observe(run, &fresh.hash)?);
            self.persist_progress(run)?;
            let view = verification::refresh_review_scope(run, &fresh);
            let receipt = run
                .evidence
                .iter()
                .rev()
                .find(|e| e.source == "runtime:artifact-read" && e.artifact_hash == fresh.hash)
                .cloned()
                .ok_or_else(|| {
                    crate::Error::Invalid("refresh artifact read receipt missing".into())
                })?;
            let verifier = providers::build_for(&view, crate::config::RoleKind::Verifier)?;
            let (logical, provider_receipt) = self.observed_turn(
                run,
                &view,
                verifier.as_ref(),
                ObservedWork::Verify {
                    artifact: &fresh,
                    evidence: &receipt,
                },
                token.clone(),
            )?;
            let mut review = verifier.parse_review(&view, &fresh, &provider_receipt)?;
            review.verifier_session = logical;
            if let Err(error) = verification::accept_refresh_review(run, &fresh, &review) {
                run.reviews.push(review);
                self.persist_progress(run)?;
                return Err(error);
            }
            run.quality = review.quality.clone();
            self.persist_progress(run)?;
        }
        Ok(())
    }
    fn persist_progress(&self, run: &Run) -> Result<()> {
        let mut store = self.store.lock().expect("store mutex");
        let mut current = store.get(&run.id)?;
        require(
            current.status != Status::Cancelled
                && current.spec_revision == run.spec_revision
                && current.attempt == run.attempt,
            "attempt cancelled or stale",
        )?;
        current.artifacts = run.artifacts.clone();
        current.reviews = run.reviews.clone();
        current.risk_coverage = run.risk_coverage.clone();
        current.evidence = run.evidence.clone();
        current.input_snapshot = run.input_snapshot.clone();
        current.input_observation = run.input_observation.clone();
        current.pending_edits = run.pending_edits.clone();
        current.provenance = run.provenance.clone();
        current.provenance_reviews = run.provenance_reviews.clone();
        current.quality_gates = run.quality_gates.clone();
        current.quality = run.quality.clone();
        current.orchestration_rounds = run.orchestration_rounds.clone();
        current.team = run.team.clone();
        current.team_risk_owners = run.team_risk_owners.clone();
        store.save(&current, "artifact_recorded", None)
    }
}
/// A completed acceptance report may be retried when its typed proof is
/// malformed. Substantive rejection still requires the normal repair path.
fn retryable_completed_review(
    run: &Run,
    artifact: &Artifact,
    session: &crate::sessions::SessionRecord,
) -> Result<()> {
    use crate::sessions::TurnState;
    run.session_ledger.validate_for_run(&run.id)?;
    let configured_provider = if let Some(pinned) = &run.spec.config {
        use crate::{config::WorkKind, quality::TaskFamily};
        let work = match run.stage {
            Stage::Discovery => WorkKind::Context,
            Stage::Planning => WorkKind::Planning,
            Stage::Execution => match run.spec.task_family {
                TaskFamily::Software => WorkKind::Implementation,
                TaskFamily::Research => WorkKind::Research,
                TaskFamily::Document => WorkKind::Writing,
                TaskFamily::Analysis => WorkKind::Analysis,
            },
            Stage::Validation => WorkKind::Testing,
            Stage::Integration => WorkKind::Integration,
            Stage::Delivery => WorkKind::Delivery,
        };
        serde_json::to_value(
            pinned
                .config
                .resolve(work, crate::config::RoleKind::Verifier)?
                .provider,
        )?
        .as_str()
        .expect("provider kind is a string")
        .to_owned()
    } else {
        run.spec.provider.clone()
    };
    require(
        session.scope.provider == configured_provider
            && session.scope.matches_run(
                run,
                session.scope.agent_id.clone(),
                session.scope.task_id.clone(),
                crate::config::RoleKind::Verifier,
            )?,
        "completed verifier scope is stale",
    )?;
    let turn = session
        .current_turn_id
        .as_ref()
        .and_then(|id| run.session_ledger.turns.get(id))
        .ok_or_else(|| crate::Error::Invalid("completed verifier turn is missing".into()))?;
    let output = turn
        .output
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("completed verifier output is missing".into()))?;
    let native = session.native_session_id.as_ref().ok_or_else(|| {
        crate::Error::Invalid("completed verifier native identity is missing".into())
    })?;
    require(
        session.native_observed
            && turn.state == TurnState::Completed
            && turn.provider_completed
            && turn.session_id == session.id
            && turn.output_ref.as_ref().is_some_and(|reference| {
                reference.document == format!("turn:{}", turn.id) && reference.hash == hash(output)
            }),
        "completed verifier has no durable bound output",
    )?;
    let author = run
        .session_ledger
        .sessions
        .get(&artifact.session_id)
        .ok_or_else(|| crate::Error::Invalid("artifact author session is missing".into()))?;
    require(
        session.id != author.id
            && (session.scope.provider != author.scope.provider
                || author.native_session_id.as_ref() != Some(native)),
        "completed verification is not independent",
    )?;
    let receipt = providers::TurnReceipt {
        turn_id: turn.id.clone(),
        provider: session.scope.provider.clone(),
        session_id: native.clone(),
        delivered: true,
        completed: true,
        stopped: true,
        output: output.clone(),
        usage: turn.usage.clone(),
    };
    // Parsing is local and does not probe or launch the provider.
    let provider = providers::build_for(run, crate::config::RoleKind::Verifier)?;
    let mut parsed = provider.parse_review(run, artifact, &receipt)?;
    parsed.verifier_session = session.id.clone();
    let stored = run
        .reviews
        .last()
        .ok_or_else(|| crate::Error::Invalid("completed verifier review is missing".into()))?;
    require(
        parsed.artifact_hash == artifact.hash
            && parsed.spec_revision == run.spec_revision
            && parsed.checklist_version == run.checklist_version
            && run.checklist_version == checklist_version()
            && parsed.executor_session == artifact.session_id
            && serde_json::to_value(&parsed)? == serde_json::to_value(stored)?,
        "completed verifier review does not match durable output and current authorship",
    )?;
    require(
        parsed.findings.is_empty()
            && parsed.checks.iter().all(|check| {
                check.status == CheckStatus::Pass
                    || (check.status == CheckStatus::NotApplicable
                        && (check.check_id.starts_with("RISK:")
                            || crate::quality::allows_not_applicable(&check.check_id)))
            }),
        "substantive verifier rejection requires repair",
    )?;
    require(
        verification::validate_review(run, artifact, &parsed).is_err(),
        "valid completed verification cannot be retried",
    )
}

fn record_approval(run: &mut Run, action: &str, scope_hash: &str) {
    run.approvals.push(Approval {
        action: action.into(),
        scope_hash: scope_hash.into(),
        spec_revision: run.spec_revision,
        actor: "local_user".into(),
    });
}

/// Unsettled effects of any run block work in every overlapping workspace.
/// Name the blocking run: only reconciling it (not cancelling) settles them.
fn require_no_overlapping_effects(store: &Store, run: &Run) -> Result<()> {
    match store.all()?.into_iter().find(|other| {
        other.id != run.id
            && other.reconciliation_required
            && crate::recovery::overlaps(&run.spec.workspace, &other.spec.workspace)
    }) {
        Some(other) => Err(crate::Error::Invalid(format!(
            "{OVERLAPPING_EFFECTS} in run {}; open that run and reconcile it first (cancelling does not settle its effects)",
            other.id
        ))),
        None => Ok(()),
    }
}

fn approve(run: &mut Run, action: &str, scope_hash: &str) -> Result<()> {
    require(
        scope_hash == run.scope_hash(action),
        "approval scope is stale",
    )?;
    record_approval(run, action, scope_hash);
    Ok(())
}
fn command_kind(command: &Command) -> &'static str {
    match command {
        Command::InspectDelivery { .. } => "source_delivery_inspection_started",
        Command::ReconcileSourceDelivery { .. } => "source_delivery_reconciled",
        Command::RestoreSourceDeliveryBackup { .. } => "source_delivery_backup_restored",
        Command::DeliverSource { .. } => "source_delivery_intent",
        Command::InspectWorkspace { .. } => "workspace_inspection_started",
        Command::PrepareWorkspace { .. } => "workspace_prepare_intent",
        Command::RestoreWorkspaceResult { .. } => "workspace_result_restored",
        Command::ActivateWorkspace { .. } => "workspace_activated",
        Command::Create { .. } => "run_created",
        Command::BeginIntake { .. } => "intake_started",
        Command::SendMessage { .. } => "message_queued_next_turn",
        Command::Orchestrate { .. } => "orchestration_started",
        Command::RecoverOrchestration { .. } => "orchestration_recovery_started",
        Command::AcceptIntake { .. } => "intake_accepted",
        Command::AcceptTeam { .. } => "team_accepted",
        Command::StepTeam { .. } => "team_step_started",
        Command::AcceptTeamStage { .. } => "team_stage_result_accepted",
        Command::ReconcileWaveEffects { .. } => "wave_effects_observed",
        Command::ReconcileTaskEffects { .. } => "task_effects_observed",
        Command::RestoreTaskResult { .. } => "task_executor_result_restored",
        Command::RestoreTaskCheckResult { .. } => "task_check_result_restored",
        Command::RefreshTaskSources { .. } => "task_source_refresh_started",
        Command::RetryTaskVerification { .. } => "task_verification_retry_started",
        Command::ResumeTaskSession { .. } => "task_native_resume_started",
        Command::ReplaceTaskSession { .. } => "task_session_replacement_started",
        Command::RepairTask { .. } => "task_repair_approved",
        Command::ChangeMode { .. } => "policy_mode_changed",
        Command::ApproveStart { .. } => "stage_approved",
        Command::AcceptResult { .. } => "result_accepted",
        Command::ApproveRepair { .. } => "repair_approved",
        Command::Advance { .. } => "attempt_started",
        Command::Check { .. } => "supplemental_check_started",
        Command::RetryVerification { .. } => "verification_retry_started",
        Command::Pause { .. } => "run_paused",
        Command::Resume { .. } => "run_resumed",
        Command::Cancel { .. } => "run_cancelled",
        Command::Revise { .. } => "spec_revised",
        Command::Reconcile { .. } => "effects_reconciled",
        Command::AddElementComment { .. } => "element_comment_added",
        Command::DeleteElementComment { .. } => "element_comment_deleted",
        Command::ResolveElementComment { .. } => "element_comment_resolved",
        Command::AddDiffComment { .. } => "diff_comment_added",
        Command::DeleteDiffComment { .. } => "diff_comment_deleted",
        Command::ResolveDiffComment { .. } => "diff_comment_resolved",
        Command::ReviewDiff { .. } => "diff_review_intent",
    }
}
