//! Admission of retained native output with no proposed edits and no effect
//! checkpoint. Never reconstruct a write, repeat inference or claim verification.
use super::*;
use crate::{
    sessions::TurnRecord,
    team::{AgentRecord, TaskRecord},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskInput {
    version: String,
    pub task_id: String,
    attempt: u64,
    contract_hash: String,
    specification_hash: String,
    agent_hash: String,
    dependency_hashes: BTreeMap<String, String>,
    view_hash: Option<String>,
}
fn contract(mut value: serde_json::Value) -> Result<String> {
    let fields = value
        .as_object_mut()
        .ok_or_else(|| crate::Error::Invalid("task contract is not an object".into()))?;
    for key in ["state", "artifact", "review", "diagnostic"] {
        fields.remove(key);
    }
    Ok(hash(serde_json::to_vec(&value)?))
}
fn specification(mut value: serde_json::Value) -> Result<String> {
    value
        .as_object_mut()
        .ok_or_else(|| crate::Error::Invalid("task specification is not an object".into()))?
        .remove("config");
    Ok(hash(serde_json::to_vec(&value)?))
}
impl TaskInput {
    pub(crate) fn capture(run: &Run, task: &TaskRecord) -> Result<Self> {
        let owner = run
            .team
            .as_ref()
            .unwrap()
            .agents
            .iter()
            .find(|a| a.id == task.owner_id)
            .unwrap();
        Ok(Self {
            version: "task-turn-input-v1".into(),
            task_id: task.id.clone(),
            attempt: task.attempt,
            contract_hash: contract(crate::runtime::team::replacement::task_context(task)?)?,
            specification_hash: specification(serde_json::to_value(&run.spec)?)?,
            agent_hash: hash(serde_json::to_vec(&(&owner.id, owner.role, owner.kind))?),
            dependency_hashes: run.team.as_ref().unwrap().dependency_bindings(task)?,
            view_hash: if views::enabled(run) {
                Some(hash(serde_json::to_vec(views::task_view(run, task)?.1)?))
            } else {
                None
            },
        })
    }
    pub(crate) fn validate(&self) -> Result<()> {
        require(
            self.version == "task-turn-input-v1"
                && !self.task_id.trim().is_empty()
                && self.attempt > 0
                && crate::sessions::digest(&self.contract_hash)
                && crate::sessions::digest(&self.specification_hash)
                && crate::sessions::digest(&self.agent_hash)
                && self
                    .dependency_hashes
                    .iter()
                    .all(|(id, h)| !id.trim().is_empty() && crate::sessions::digest(h))
                && self
                    .view_hash
                    .as_deref()
                    .is_none_or(crate::sessions::digest),
            "invalid native task input checkpoint",
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Admission {
    version: String,
    turn_id: String,
    scope_hash: String,
    input: TaskInput,
}

fn legacy_input(
    run: &Run,
    task: &TaskRecord,
    owner: &AgentRecord,
    turn: &TurnRecord,
) -> Result<TaskInput> {
    let context = turn.context.as_ref().ok_or_else(|| {
        crate::Error::Invalid(
            "legacy completed output has no retained task input checkpoint or full pinned prompt"
                .into(),
        )
    })?;
    context.validate()?;
    context.complete()?;
    let prompt = &context.bundle.documents[&turn.prompt_hash].utf8;
    let header = super::super::tasks::prompt_header(run, task, owner);
    let payload = prompt
        .strip_prefix(&header)
        .ok_or_else(|| crate::Error::Invalid("retained task prompt contract changed".into()))?;
    let value = serde_json::Value::deserialize(&mut serde_json::Deserializer::from_str(payload))?;
    let expected = TaskInput::capture(run, task)?;
    require(
        specification(value["spec"].clone())? == expected.specification_hash
            && value["spec_revision"] == run.spec_revision,
        "retained native specification changed",
    )?;
    require(
        contract(value["task"].clone())? == expected.contract_hash,
        "retained native task contract changed",
    )?;
    let dependencies: Vec<TaskRecord> = serde_json::from_value(value["dependencies"].clone())?;
    let mut bindings = BTreeMap::new();
    for dependency in dependencies {
        let artifact = dependency
            .artifact
            .as_ref()
            .ok_or_else(|| crate::Error::Invalid("retained dependency artifact missing".into()))?;
        let review = dependency
            .review
            .as_ref()
            .filter(|r| r.accepted)
            .ok_or_else(|| crate::Error::Invalid("retained dependency review missing".into()))?;
        require(
            bindings
                .insert(
                    dependency.id,
                    hash(serde_json::to_vec(&(artifact, review))?),
                )
                .is_none(),
            "ambiguous retained dependencies",
        )?;
    }
    require(
        bindings == expected.dependency_hashes,
        "retained native dependencies changed",
    )?;
    if views::enabled(run) {
        require(
            value["sources"]["commit"] == views::task_view(run, task)?.1.start,
            "retained task read view changed",
        )?;
    } else {
        require(
            value["sources"] == serde_json::to_value(run.sources())?,
            "retained task sources changed",
        )?;
    }
    Ok(expected)
}

pub(super) fn candidate(run: &Run, task: &TaskRecord) -> Result<effects::ExecutionBinding> {
    require(
        task.write_scopes.is_empty() && task.attempt == task.history.len() as u64 + 1,
        "unbound completed output recovery requires a current read-only task",
    )?;
    require(
        crate::workspace::inputs_current(run)?
            && !run.check_resources.records().any(|r| r.unresolved()),
        "read-only output requires current sources and settled resources",
    )?;
    run.session_ledger.validate_for_run(&run.id)?;
    let team = run.team.as_ref().unwrap();
    let owner = team.agents.iter().find(|a| a.id == task.owner_id).unwrap();
    let turn = run
        .session_ledger
        .turn_order
        .iter()
        .rev()
        .filter_map(|id| run.session_ledger.turns.get(id))
        .find(|t| {
            let s = &run.session_ledger.sessions[&t.session_id];
            s.scope.task_id.as_deref() == Some(&task.id) && s.scope.role == owner.kind
        })
        .ok_or_else(|| crate::Error::Invalid("no retained executor output".into()))?;
    require(
        owner.session_id.as_ref() == Some(&turn.session_id),
        "retained output is not the latest task executor",
    )?;
    let session = &run.session_ledger.sessions[&turn.session_id];
    require(
        session.current_turn_id.as_ref() == Some(&turn.id) && session.state == TurnState::Completed,
        "retained task session is incomplete",
    )?;
    let expected = TaskInput::capture(run, task)?;
    expected.validate()?;
    require(
        turn.schema_hash == hash(serde_json::to_vec(&crate::team::models::output_schema())?),
        "retained task response schema changed",
    )?;
    if let Some(input) = &turn.task_input {
        require(
            input == &expected,
            "native task input, dependencies or read view changed",
        )?;
    } else {
        require(
            legacy_input(run, task, owner, turn)? == expected,
            "legacy task input changed",
        )?;
    }
    let raw = turn
        .output
        .as_deref()
        .ok_or_else(|| crate::Error::Invalid("completed native output is missing".into()))?;
    let output: crate::team::models::Output = crate::transport::decode(raw.as_bytes())?;
    require(
        output.edits.is_empty(),
        "unbound output proposes edits; no write may be reconstructed",
    )?;
    let view = if views::enabled(run) {
        let v = views::task_view(run, task)?.1;
        require(
            v.worktree.is_none()
                && v.result.is_none()
                && v.integration.is_none()
                && v.start == v.view,
            "read-only task view has uncertain effects",
        )?;
        Some(views::ViewEffect {
            worktree: None,
            start: v.start.clone(),
            before: BTreeMap::new(),
            result: Some(v.start.clone()),
        })
    } else {
        None
    };
    let execution = effects::ExecutionBinding {
        id: id(),
        task_id: task.id.clone(),
        attempt: task.attempt,
        wave_attempt: run.attempt,
        artifact_hash: hash(raw),
        scope: session.scope.clone(),
        session_id: session.id.clone(),
        input_snapshot: run.input_snapshot.clone(),
        input_observation: run.input_observation.clone(),
        source_hashes: run.source_hashes(),
        dependency_hashes: expected.dependency_hashes.clone(),
        after_snapshot: view.is_none().then(|| run.input_snapshot.clone()),
        receipts: vec![],
        state: effects::ExecutionState::Committed,
        reconciliation: None,
        view,
        read_only_admission: Some(Admission {
            version: "readonly-task-admission-v1".into(),
            turn_id: turn.id.clone(),
            scope_hash: super::scope_hash(run, &task.id)?,
            input: expected,
        }),
    };
    effects::proposal(run, &execution)?;
    Ok(execution)
}

pub(crate) fn authenticate(
    run: &Run,
    execution: &effects::ExecutionBinding,
    output: &crate::team::models::Output,
) -> Result<()> {
    let admission = execution.read_only_admission.as_ref().unwrap();
    admission.input.validate()?;
    let turn = run
        .session_ledger
        .turns
        .get(&admission.turn_id)
        .ok_or_else(|| crate::Error::Invalid("read-only completion turn missing".into()))?;
    require(
        admission.version == "readonly-task-admission-v1"
            && crate::sessions::digest(&admission.scope_hash)
            && admission.input.task_id == execution.task_id
            && admission.input.attempt == execution.attempt
            && admission.input.dependency_hashes == execution.dependency_hashes
            && turn.session_id == execution.session_id
            && run.session_ledger.sessions[&execution.session_id]
                .current_turn_id
                .as_ref()
                == Some(&admission.turn_id)
            && turn
                .task_input
                .as_ref()
                .is_none_or(|input| input == &admission.input)
            && (turn.task_input.is_some() || turn.context.is_some())
            && execution.state == effects::ExecutionState::Committed
            && execution.reconciliation.is_none()
            && output.edits.is_empty()
            && execution.receipts.is_empty()
            && run
                .task_result_restorations
                .iter()
                .filter(|r| {
                    r.execution_id == execution.id
                        && r.task_id == execution.task_id
                        && r.task_attempt == execution.attempt
                        && r.session_id == execution.session_id
                        && r.turn_id == admission.turn_id
                        && r.artifact_hash == execution.artifact_hash
                        && r.scope_hash == admission.scope_hash
                        && r.actor == "local_user"
                })
                .count()
                == 1
            && run.approvals.iter().any(|a| {
                a.action == "restore_task_result"
                    && a.actor == "local_user"
                    && a.spec_revision == execution.scope.spec_revision
                    && a.scope_hash == admission.scope_hash
            }),
        "read-only task result has no exact native completion and scoped admission receipt",
    )?;
    require(
        match &execution.view {
            Some(v) => {
                v.worktree.is_none()
                    && v.before.is_empty()
                    && v.result.as_ref() == Some(&v.start)
                    && execution.after_snapshot.is_none()
            }
            None => execution.after_snapshot.as_ref() == Some(&execution.input_snapshot),
        },
        "read-only admission cannot claim file effects",
    )
}
