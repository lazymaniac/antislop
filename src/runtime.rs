use crate::{Result, domain::*, hash, id, providers, require, storage::Store, verification};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Create {
        spec: RunSpec,
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
}
impl Command {
    fn run_id(&self) -> Option<&str> {
        match self {
            Self::Create { .. } => None,
            Self::ApproveStart { run_id, .. }
            | Self::AcceptResult { run_id, .. }
            | Self::ApproveRepair { run_id, .. }
            | Self::Advance { run_id }
            | Self::Pause { run_id }
            | Self::Resume { run_id }
            | Self::Cancel { run_id }
            | Self::Revise { run_id, .. }
            | Self::Reconcile { run_id, .. } => Some(run_id),
        }
    }
}
pub struct Runtime {
    pub store: Mutex<Store>,
    workers: Mutex<HashMap<String, Arc<AtomicBool>>>,
}
impl Runtime {
    pub fn new(mut store: Store) -> Result<Self> {
        for mut run in store.all()? {
            if run.checklist_version != checklist_version()
                && !matches!(run.status, Status::Completed | Status::Cancelled)
            {
                run.status = Status::Blocked;
                run.diagnostic = Some(
                    "Pinned checklist changed; explicitly revise the run before continuing.".into(),
                );
                store.save(&run, "checklist_migration_required", None)?;
            }
            if run.status == Status::Running || run.paused_from == Some(Status::Running) {
                if run.status == Status::Paused {
                    run.paused_from = Some(Status::Blocked);
                } else {
                    run.status = Status::Blocked;
                }
                run.reconciliation_required = true;
                if let Err(error) = crate::recovery::inspect(&mut run) {
                    run.diagnostic = Some(format!("Cannot inspect interrupted edits: {error}"));
                }
                run.diagnostic = Some("Interrupted attempt: reconcile workspace and persisted provider_session_id before authorizing a new attempt. No automatic replay.".into());
                store.save(&run, "recovery_required", None)?;
            }
        }
        Ok(Self {
            store: Mutex::new(store),
            workers: Mutex::new(HashMap::new()),
        })
    }
    pub fn dispatch(self: &Arc<Self>, key: &str, command: Command) -> Result<Run> {
        require(!key.trim().is_empty(), "idempotency key required")?;
        let bytes = serde_json::to_vec(&command)?;
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
        } else {
            store.get(command.run_id().expect("run command"))?
        };
        require(
            matches!(&command, Command::Revise { .. })
                || run.checklist_version == checklist_version(),
            "pinned checklist changed; explicit revision required",
        )?;
        let terminal = matches!(run.status, Status::Completed | Status::Cancelled);
        require(!terminal || matches!(&command, Command::Reconcile { .. }), "run is terminal")?;
        let mut launch = false;
        let mut lease = None;
        match &command {
            Command::Create { .. } => (),
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
                    crate::workspace::read_inputs(
                        std::path::Path::new(&run.spec.workspace),
                        &run.spec.input_files,
                    )? == run.input_snapshot,
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
                require(!run.reconciliation_required, "reconcile interrupted effects before approving repair")?;
                require(
                    matches!(run.status, Status::ChangesRequested | Status::Blocked),
                    "not awaiting repair",
                )?;
                require(crate::workspace::read_inputs(std::path::Path::new(&run.spec.workspace), &run.spec.input_files)? == run.input_snapshot,
                    "workspace changed; explicitly revise before repair")?;
                approve(&mut run, "repair", scope_hash)?;
                run.status = Status::Ready;
            }
            Command::Advance { .. } => {
                require(run.status == Status::Ready, "stage or repair not approved")?;
                require(!run.reconciliation_required, "reconciliation is required")?;
                for other in store.all()? {
                    if other.id != run.id && other.reconciliation_required && crate::recovery::overlaps(&run.spec.workspace, &other.spec.workspace) {
                        return Err(crate::Error::Invalid("overlapping workspace has unresolved effects; reconcile it first".into()));
                    }
                }
                lease = Some(crate::recovery::WorkspaceLease::acquire(&run.spec.workspace)?);
                run.reconciliation_required = true;
                run.status = Status::Running;
                run.attempt += 1;
                run.active_session = None;
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
                run.status = Status::Cancelled;
                run.paused_from = None;
                self.signal(&run.id);
            }
            Command::Reconcile { scope_hash, input_hash, .. } => {
                require(run.reconciliation_required, "run does not require reconciliation")?;
                require(!self.workers.lock().expect("workers mutex").contains_key(&run.id), "worker is still stopping")?;
                let _lease = crate::recovery::WorkspaceLease::acquire(&run.spec.workspace)?;
                let observation = crate::recovery::observe(&run)?;
                require(observation.hash == *input_hash, "observed workspace hash is stale")?;
                approve(&mut run, "reconcile", scope_hash)?;
                crate::recovery::inspect(&mut run)?;
                let output = serde_json::json!({"edits":run.pending_edits,"observed_inputs":observation.inputs}).to_string();
                run.evidence.push(Evidence { id:id(), artifact_hash:hash(&output), spec_revision:run.spec_revision,
                    source:"runtime:edit-reconciliation".into(), output_hash:hash(&output), output, success:true });
                let unreadable = observation.readable.is_none();
                if observation.readable.as_ref() != Some(&run.input_snapshot) || !run.pending_edits.is_empty() {
                    if let Some(current) = observation.readable { run.input_snapshot = current; }
                    run.spec_revision += 1;
                    run.risk_coverage = risk_coverage();
                    if !terminal {
                        run.stage = Stage::Discovery;
                        run.status = if unreadable { Status::Blocked } else { Status::AwaitingStart };
                        run.paused_from = None;
                    }
                }
                run.pending_edits.clear();
                run.reconciliation_required = false;
                verification::refresh_provenance(&mut run);
                run.diagnostic = Some(if unreadable {
                    "Effects reconciled without replay. Explicitly revise the pinned input set before continuing; some files are missing or unreadable."
                } else { "Effects reconciled without replay; prior artifacts retained as history." }.into());
            }
            Command::Revise {
                spec,
                expected_revision,
                ..
            } => {
                require(run.spec_revision == *expected_revision, "revision conflict")?;
                require(!run.reconciliation_required, "reconcile interrupted effects before revision")?;
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
                run.spec.workspace = std::fs::canonicalize(&spec.workspace)?
                    .to_string_lossy()
                    .into_owned();
                run.input_snapshot = crate::workspace::read_inputs(
                    std::path::Path::new(&run.spec.workspace),
                    &spec.input_files,
                )?;
                run.spec_revision += 1;
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
        drop(store);
        if let Some(token) = token {
            let runtime = self.clone();
            let captured = run.clone();
            std::thread::spawn(move || runtime.work(captured, token, lease.expect("launch lease")));
        }
        Ok(run)
    }
    fn signal(&self, run_id: &str) {
        if let Some(token) = self.workers.lock().expect("workers mutex").get(run_id) {
            token.store(true, Ordering::SeqCst);
        }
    }
    fn session(&self, captured: &Run, session: &str) -> Result<()> {
        let mut store = self.store.lock().expect("store mutex");
        let mut current = store.get(&captured.id)?;
        require(
            current.spec_revision == captured.spec_revision
                && current.attempt == captured.attempt
                && current.status != Status::Cancelled,
            "attempt is no longer active",
        )?;
        current.active_session = Some(session.into());
        store.save(&current, "provider_session_started", None)
    }
    fn work(self: Arc<Self>, mut run: Run, token: Arc<AtomicBool>, lease: crate::recovery::WorkspaceLease) {
        let _lease_scope = crate::process::LeaseScope::new(&lease.fds());
        let outcome = (|| -> Result<()> {
            let provider = providers::build(&run.spec.provider)?;
            let workspace = std::path::PathBuf::from(&run.spec.workspace);
            let root = workspace.as_path();
            let observed_inputs = crate::workspace::read_inputs(root, &run.spec.input_files)?;
            require(
                observed_inputs == run.input_snapshot,
                "workspace inputs changed before stage; revise explicitly",
            )?;
            let input_output = serde_json::to_string(&observed_inputs)?;
            let input_hash = hash(&input_output);
            run.evidence.push(Evidence {
                id: id(),
                artifact_hash: input_hash.clone(),
                spec_revision: run.spec_revision,
                source: "runtime:input-snapshot".into(),
                output_hash: input_hash.clone(),
                output: input_output,
                success: true,
            });
            if run.stage == Stage::Discovery && !run.spec.check_command.is_empty() {
                let (success, output) =
                    crate::workspace::check(root, &run.spec.check_command, token.clone())?;
                run.evidence.push(Evidence {
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

            let (session_id, content) = provider.execute(&run, token.clone(), &mut |session| {
                self.session(&run, session)
            })?;
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
            let output: ExecutionOutput = serde_json::from_str(&artifact.content)?;
            verification::validate_output(&run, &output)?;
            require(
                run.stage == Stage::Execution || output.edits.is_empty(),
                "file edits are only allowed in Execution",
            )?;
            verification::register_artifact(&mut run, &artifact, &output)?;
            if run.stage == Stage::Execution {
                run.pending_edits = crate::recovery::prepare(&run, &output.edits)?;
                // Intent is durable before the first filesystem effect.
                self.persist_progress(&run)?;
                for (index, edit) in output.edits.iter().enumerate() {
                    require(!token.load(Ordering::SeqCst), "cancelled before write")?;
                    let expected = run.pending_edits[index].before_hash.clone();
                    crate::workspace::replace(std::path::Path::new(&run.spec.workspace), &edit.path, &expected, &edit.content)?;
                    run.input_snapshot.insert(edit.path.clone(), edit.content.clone());
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
                let (success, output) = crate::workspace::check(
                    std::path::Path::new(&run.spec.workspace),
                    &run.spec.check_command,
                    token.clone(),
                )?;
                run.evidence.push(Evidence {
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
                id: id(),
                artifact_hash: artifact.hash.clone(),
                spec_revision: run.spec_revision,
                source: "runtime:input-snapshot".into(),
                output_hash: hash(&current_input),
                output: current_input,
                success: true,
            });
            require(
                crate::workspace::read_inputs(root, &run.spec.input_files)? == run.input_snapshot,
                "check command changed pinned inputs; reconcile before review",
            )?;
            run.evidence
                .push(verification::workspace_diff(&run, &artifact)?);
            // Persist executor output before opening the independent verifier session.
            self.persist_progress(&run)?;
            let review =
                provider.verify(&run, &artifact, &evidence, token.clone(), &mut |session| {
                    self.session(&run, session)
                })?;
            let result = verification::validate_review(&run, &artifact, &review);

            if result.is_ok() {
                verification::record_review(&mut run, &artifact, &review)?;
            }
            run.reviews.push(review);
            match result {
                Ok(()) => {
                    run.status = Status::AwaitingResult;
                    run.diagnostic = None;
                }
                Err(e) => {
                    run.status = Status::ChangesRequested;
                    run.diagnostic = Some(e.to_string());
                }
            }
            Ok(())
        })();
        if outcome.is_ok() {
            run.reconciliation_required = false;
            run.pending_edits.clear();
        }
        if let Err(error) = outcome {
            run.status = Status::Blocked;
            run.diagnostic = Some(error.to_string());
        }
        let mut store = self.store.lock().expect("store mutex");
        if let Ok(current) = store.get(&run.id)
            && current.spec_revision == run.spec_revision
            && current.attempt == run.attempt
            && current.status != Status::Cancelled
        {
            run.approvals = current.approvals;
            run.active_session = current.active_session;
            if current.status == Status::Paused {
                run.paused_from = Some(run.status);
                run.status = Status::Paused;
            }
            if let Err(e) = store.save(&run, "attempt_finished", None) {
                eprintln!("Cannot persist attempt: {e}");
            }
        }
        self.workers.lock().expect("workers mutex").remove(&run.id);
    }
    fn persist_progress(&self, run: &Run) -> Result<()> {
        let mut store = self.store.lock().expect("store mutex");
        let mut current = store.get(&run.id)?;
        require(
            current.status != Status::Cancelled && current.spec_revision == run.spec_revision && current.attempt == run.attempt,
            "attempt cancelled or stale",
        )?;
        current.artifacts = run.artifacts.clone();
        current.evidence = run.evidence.clone();
        current.input_snapshot = run.input_snapshot.clone();
        current.pending_edits = run.pending_edits.clone();
        current.provenance = run.provenance.clone();
        store.save(&current, "artifact_recorded", None)
    }
}
fn approve(run: &mut Run, action: &str, scope_hash: &str) -> Result<()> {
    require(
        scope_hash == run.scope_hash(action),
        "approval scope is stale",
    )?;
    run.approvals.push(Approval {
        action: action.into(),
        scope_hash: scope_hash.into(),
        spec_revision: run.spec_revision,
        actor: "local_user".into(),
    });
    Ok(())
}
fn command_kind(command: &Command) -> &'static str {
    match command {
        Command::Create { .. } => "run_created",
        Command::ApproveStart { .. } => "stage_approved",
        Command::AcceptResult { .. } => "result_accepted",
        Command::ApproveRepair { .. } => "repair_approved",
        Command::Advance { .. } => "attempt_started",
        Command::Pause { .. } => "run_paused",
        Command::Resume { .. } => "run_resumed",
        Command::Cancel { .. } => "run_cancelled",
        Command::Revise { .. } => "spec_revised",
        Command::Reconcile { .. } => "effects_reconciled",
    }
}
