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
    /// Task-worktree effects; `None` for edits applied in the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<super::views::ViewEffect>,
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
    run.session_ledger.validate_for_run(&run.id)?;
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
    if let Some(view) = &execution.view {
        for (edit, receipt) in output.edits.iter().zip(&execution.receipts) {
            require(
                receipt.path == edit.path
                    && receipt.state == EditState::Applied
                    && receipt.operation == edit.operation
                    && view.before.get(&edit.path) == Some(&receipt.before_hash)
                    && super::views::after_hash(edit) == receipt.after_hash,
                "execution receipt does not prove its exact worktree effect",
            )?;
        }
        return require(
            view.before.len() == execution.receipts.len()
                && view.result.is_some()
                && execution.after_snapshot.is_none(),
            "worktree execution result commit missing",
        );
    }
    for (edit, receipt) in output.edits.iter().zip(&execution.receipts) {
        require(
            receipt.path == edit.path
                && receipt.state == EditState::Applied
                && receipt.operation == edit.operation
                && crate::recovery::matches_version(
                    Path::new(&run.spec.workspace),
                    &execution.input_snapshot,
                    receipt,
                    true,
                )?
                && crate::workspace::inputs::version_hash(crate::workspace::inputs::after_text(
                    edit,
                )) == receipt.after_hash
                && execution.after_snapshot.as_ref().is_some_and(|s| {
                    crate::recovery::matches_version(
                        Path::new(&run.spec.workspace),
                        s,
                        receipt,
                        false,
                    )
                    .unwrap_or(false)
                }),
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
    /// Admit an observed native proposal for effects: same attempt, policy,
    /// session scope and exact completed output.
    pub(super) fn admit_task_output(
        &self,
        captured: &Run,
        invocation: &TaskInvocation,
        session_id: &str,
        raw: &str,
        edits: &[FileEdit],
        token: &AtomicBool,
    ) -> Result<(
        std::sync::MutexGuard<'_, crate::storage::Store>,
        Run,
        SessionScope,
    )> {
        let store = self.active_store(captured)?;
        let current = store.get(&captured.id)?;
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
        let scope = session.scope.clone();
        Ok((store, current, scope))
    }
    pub(super) fn apply_task_output(
        &self,
        captured: &Run,
        invocation: &TaskInvocation,
        session_id: &str,
        raw: &str,
        edits: &[FileEdit],
        token: &AtomicBool,
    ) -> Result<String> {
        let (mut store, mut current, scope) =
            self.admit_task_output(captured, invocation, session_id, raw, edits, token)?;
        let mut proven_inputs = captured.input_snapshot.clone();
        for execution in &current.task_executions {
            if execution.wave_attempt != captured.attempt
                || !matches!(
                    execution.state,
                    ExecutionState::Applied | ExecutionState::Committed
                )
            {
                continue;
            }
            authenticate(&current, execution)?;
            for receipt in &execution.receipts {
                let snapshot = execution
                    .after_snapshot
                    .as_ref()
                    .ok_or_else(|| crate::Error::Invalid("managed after version missing".into()))?;
                let text = crate::workspace::inputs::captured_text(
                    Path::new(&current.spec.workspace),
                    snapshot,
                    &receipt.path,
                )?;
                let edit = FileEdit {
                    path: receipt.path.clone(),
                    content: text.unwrap_or_default(),
                    operation: receipt.operation,
                };
                crate::workspace::inputs::record_change(
                    Path::new(&current.spec.workspace),
                    &mut proven_inputs,
                    &edit,
                    &receipt.before_hash,
                )?;
            }
        }
        require(
            proven_inputs == current.input_snapshot,
            "changed task inputs have no completed managed effect proof",
        )?;
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
                crate::workspace::inputs::captured_text(
                    Path::new(&current.spec.workspace),
                    &current.input_snapshot,
                    &edit.path,
                )? == crate::workspace::inputs::captured_text(
                    Path::new(&captured.spec.workspace),
                    &captured.input_snapshot,
                    &edit.path,
                )?,
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
            scope,
            session_id: session_id.to_string(),
            input_snapshot: captured.input_snapshot.clone(),
            source_hashes: captured.source_hashes(),
            dependency_hashes: invocation.dependency_hashes.clone(),
            after_snapshot: None,
            receipts: receipts.clone(),
            state: ExecutionState::Intent,
            reconciliation: None,
            view: None,
        });
        current.pending_edits.extend(receipts);
        // Intent and ownership are durable before any filesystem replacement.
        store.save(&current, "task_execution_intent", None)?;
        for (receipt_index, edit) in edits.iter().enumerate() {
            require(!token.load(Ordering::SeqCst), "task cancelled before write")?;
            crate::workspace::apply_edit(
                Path::new(&current.spec.workspace),
                edit,
                &current.pending_edits[offset + receipt_index].before_hash,
            )?;
            crate::workspace::inputs::record_change(
                Path::new(&current.spec.workspace),
                &mut current.input_snapshot,
                edit,
                &current.pending_edits[offset + receipt_index].before_hash,
            )?;
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
