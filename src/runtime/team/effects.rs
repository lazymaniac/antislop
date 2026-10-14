//! Immutable execution inputs and write-ahead effect ownership. No effect replay.
use super::*;
use crate::{
    recovery::{EditReceipt, EditState},
    sessions::{SessionScope, TurnState},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionState {
    Intent,
    Applied,
    ObservedApplied,
    Committed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionBinding {
    pub id: String,
    pub task_id: String,
    pub attempt: u64,
    pub wave_attempt: u64,
    pub artifact_hash: String,
    pub scope: SessionScope,
    pub session_id: String,
    pub input_snapshot: BTreeMap<String, String>,
    pub source_hashes: BTreeMap<String, String>,
    pub dependency_hashes: BTreeMap<String, String>,
    pub after_snapshot: Option<BTreeMap<String, String>>,
    pub receipts: Vec<EditReceipt>,
    pub state: ExecutionState,
    #[serde(default)]
    pub reconciliation: Option<super::adjudicate::ObservationReceipt>,
}

pub(super) fn binding<'a>(run: &'a Run, artifact: &ArtifactRef) -> Result<&'a ExecutionBinding> {
    let execution = artifact
        .execution_id
        .as_ref()
        .and_then(|id| run.task_executions.iter().find(|e| &e.id == id))
        .ok_or_else(|| {
            crate::Error::Invalid("task execution input/effect binding missing".into())
        })?;
    require(
        execution.state == ExecutionState::Committed
            && execution.task_id == artifact.task_id
            && execution.attempt == artifact.attempt
            && execution.artifact_hash == artifact.content_hash
            && execution.session_id == artifact.session_id
            && execution.scope.agent_id == artifact.created_by
            && execution.scope.task_id.as_deref() == Some(&artifact.task_id)
            && execution.source_hashes == artifact.source_hashes
            && execution.dependency_hashes == artifact.dependency_hashes
            && execution.scope.spec_revision == artifact.spec_revision,
        "task execution input/effect binding is incomplete or belongs to another result",
    )?;
    require(
        run.source_hashes_for_inputs(&execution.input_snapshot) == execution.source_hashes,
        "execution original source hashes are not its captured inputs",
    )?;
    authenticate(run, execution)?;
    Ok(execution)
}
pub(super) fn proposal(
    run: &Run,
    execution: &ExecutionBinding,
) -> Result<crate::team::models::Output> {
    run.session_ledger.validate()?;
    let session = run
        .session_ledger
        .sessions
        .get(&execution.session_id)
        .ok_or_else(|| crate::Error::Invalid("execution native session missing".into()))?;
    let turn = session
        .current_turn_id
        .as_ref()
        .and_then(|id| run.session_ledger.turns.get(id))
        .ok_or_else(|| crate::Error::Invalid("execution native turn missing".into()))?;
    let raw = turn
        .output
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("execution native output missing".into()))?;
    require(
        session.scope == execution.scope
            && execution.scope.input_hash == hash(serde_json::to_vec(&execution.input_snapshot)?)
            && session.native_observed
            && session.native_session_id.is_some()
            && turn.state == TurnState::Completed
            && turn.provider_completed
            && hash(raw) == execution.artifact_hash
            && turn.output_ref.as_ref().is_some_and(|r| {
                r.document == format!("turn:{}", turn.id) && r.hash == execution.artifact_hash
            }),
        "execution is not bound to its observed native output and original inputs",
    )?;
    crate::transport::decode(raw.as_bytes())
}

pub(super) fn authenticate(run: &Run, execution: &ExecutionBinding) -> Result<()> {
    let output = proposal(run, execution)?;
    require(
        execution.state != ExecutionState::ObservedApplied || execution.reconciliation.is_some(),
        "observed effects require their user observation receipt",
    )?;
    if execution.reconciliation.is_some() {
        super::adjudicate::authenticate_receipt(run, execution)?;
    }
    require(
        output.edits.len() == execution.receipts.len(),
        "execution proposed edits differ from owned receipts",
    )?;
    for (edit, receipt) in output.edits.iter().zip(&execution.receipts) {
        require(
            receipt.path == edit.path
                && receipt.state == EditState::Applied
                && execution
                    .input_snapshot
                    .get(&edit.path)
                    .is_some_and(|text| hash(text) == receipt.before_hash)
                && hash(&edit.content) == receipt.after_hash
                && execution
                    .after_snapshot
                    .as_ref()
                    .and_then(|s| s.get(&edit.path))
                    .is_some_and(|text| hash(text) == receipt.after_hash),
            "execution receipt does not prove its exact before/after effect",
        )?;
    }
    require(
        execution.after_snapshot.is_some(),
        "execution after snapshot missing",
    )
}
pub(super) fn lineage(run: &Run, task: &crate::team::TaskRecord) -> Result<()> {
    for historical in &task.history {
        if let Some(artifact) = &historical.artifact {
            binding(run, artifact)?;
        }
    }
    Ok(())
}

impl Runtime {
    pub(super) fn apply_task_output(
        &self,
        captured: &Run,
        invocation: &TaskInvocation,
        session_id: &str,
        raw: &str,
        edits: &[FileEdit],
        token: &AtomicBool,
    ) -> Result<String> {
        let mut store = self.active_store(captured)?;
        let mut current = store.get(&captured.id)?;
        require(
            !token.load(Ordering::SeqCst),
            "task cancelled before execution binding",
        )?;
        require(
            serde_json::to_value(&current.spec)? == serde_json::to_value(&captured.spec)?,
            "task configuration or policy changed before effect admission",
        )?;
        let session = current
            .session_ledger
            .sessions
            .get(session_id)
            .ok_or_else(|| crate::Error::Invalid("task execution session missing".into()))?;
        require(
            session.scope
                == SessionScope::for_run(
                    captured,
                    invocation.agent.id.clone(),
                    Some(invocation.task.id.clone()),
                    invocation.agent.kind,
                    session.scope.provider.clone(),
                    session.scope.binary_version.clone(),
                )?,
            "task execution scope does not match its captured inputs",
        )?;
        let turn = session
            .current_turn_id
            .as_ref()
            .and_then(|id| current.session_ledger.turns.get(id))
            .ok_or_else(|| {
                crate::Error::Invalid("task completed turn missing before write".into())
            })?;
        let parsed: crate::team::models::Output = crate::transport::decode(raw.as_bytes())?;
        require(
            session.native_observed
                && turn.state == TurnState::Completed
                && turn.provider_completed
                && turn.output.as_deref() == Some(raw)
                && turn
                    .output_ref
                    .as_ref()
                    .is_some_and(|r| r.hash == hash(raw))
                && serde_json::to_value(&parsed.edits)? == serde_json::to_value(edits)?,
            "task writes are not the observed native proposal",
        )?;
        for (path, current_text) in &current.input_snapshot {
            if captured.input_snapshot.get(path) != Some(current_text) {
                require(
                    current.task_executions.iter().any(|e| {
                        e.wave_attempt == captured.attempt
                            && matches!(
                                e.state,
                                ExecutionState::Applied | ExecutionState::Committed
                            )
                            && e.receipts.iter().any(|r| {
                                &r.path == path
                                    && r.state == EditState::Applied
                                    && r.after_hash == hash(current_text)
                            })
                    }),
                    "changed task inputs have no completed managed effect proof",
                )?;
            }
        }
        require(
            crate::workspace::read_inputs(
                Path::new(&current.spec.workspace),
                &current.spec.input_files,
            )? == current.input_snapshot,
            "workspace has unobserved changes before task writes",
        )?;
        // Concurrent disjoint managed writes may update other files, never this task's expected write bytes.
        for edit in edits {
            require(
                current.input_snapshot.get(&edit.path) == captured.input_snapshot.get(&edit.path),
                "task write conflicts with a newer managed or user edit",
            )?;
        }
        let receipts = crate::recovery::prepare(&current, edits)?;
        let offset = current.pending_edits.len();
        let execution_id = id();
        let index = current.task_executions.len();
        current.task_executions.push(ExecutionBinding {
            id: execution_id.clone(),
            task_id: invocation.task.id.clone(),
            attempt: invocation.task.attempt,
            wave_attempt: captured.attempt,
            artifact_hash: hash(raw),
            scope: session.scope.clone(),
            session_id: session_id.to_string(),
            input_snapshot: captured.input_snapshot.clone(),
            source_hashes: captured.source_hashes(),
            dependency_hashes: invocation.dependency_hashes.clone(),
            after_snapshot: None,
            receipts: receipts.clone(),
            state: ExecutionState::Intent,
            reconciliation: None,
        });
        current.pending_edits.extend(receipts);
        // Intent and ownership are durable before any filesystem replacement.
        store.save(&current, "task_execution_intent", None)?;
        for (receipt_index, edit) in edits.iter().enumerate() {
            require(!token.load(Ordering::SeqCst), "task cancelled before write")?;
            crate::workspace::replace(
                Path::new(&current.spec.workspace),
                &edit.path,
                &current.pending_edits[offset + receipt_index].before_hash,
                &edit.content,
            )?;
            current
                .input_snapshot
                .insert(edit.path.clone(), edit.content.clone());
            current.pending_edits[offset + receipt_index].state = EditState::Applied;
            current.task_executions[index].receipts[receipt_index].state = EditState::Applied;
            crate::verification::refresh_provenance(&mut current);
            store.save(&current, "task_execution_edit_applied", None)?;
        }
        require(
            crate::workspace::read_inputs(
                Path::new(&current.spec.workspace),
                &current.spec.input_files,
            )? == current.input_snapshot,
            "workspace changed while committing task effects",
        )?;
        current.task_executions[index].after_snapshot = Some(current.input_snapshot.clone());
        current.task_executions[index].state = ExecutionState::Applied;
        authenticate(&current, &current.task_executions[index])?;
        store.save(&current, "task_execution_applied", None)?;
        Ok(execution_id)
    }
}
