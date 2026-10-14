//! Explicit fresh sessions with a captured, source-bound handoff; never implicit resume.
use super::*;
use crate::{
    sessions::{SessionRecord, TurnRecord},
    team::TaskRecord,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const HANDOFF_VERSION: &str = "task-handoff-v1";
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Handoff {
    pub version: String,
    pub task_id: String,
    pub task_attempt: u64,
    pub spec_revision: u64,
    pub configuration_hash: String,
    pub source_hashes: BTreeMap<String, String>,
    pub contract: crate::domain::RunSpec,
    pub task_contract: Value,
    pub dependencies: Vec<Value>,
    pub knowledge: Value,
    pub prior_sessions: Vec<SessionRecord>,
    pub prior_turns: Vec<TurnRecord>,
    pub artifacts: Vec<Artifact>,
    pub task_checks: Vec<checks::Invocation>,
    pub task_effects: Vec<effects::ExecutionBinding>,
    pub message_history: Value,
}
impl Handoff {
    pub fn hash(&self) -> Result<String> {
        Ok(hash(serde_json::to_vec(self)?))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pending {
    pub task_id: String,
    pub operation: TaskOperation,
    pub previous_session: Option<String>,
    pub previous_turn: Option<String>,
    pub reason: String,
    pub handoff: Handoff,
    pub handoff_hash: String,
    pub scope_hash: String,
}
impl Pending {
    pub fn validate(&self) -> Result<()> {
        require(
            self.handoff.version == HANDOFF_VERSION
                && self.task_id == self.handoff.task_id
                && self.handoff.hash()? == self.handoff_hash
                && crate::sessions::digest(&self.scope_hash)
                && self.previous_session.is_some() == self.previous_turn.is_some(),
            "replacement handoff identity/content binding is invalid",
        )
    }
    pub fn validate_for_run(&self, run: &Run) -> Result<()> {
        self.validate()?;
        let task = run
            .team
            .as_ref()
            .and_then(|team| team.tasks.iter().find(|t| t.id == self.task_id))
            .ok_or_else(|| crate::Error::Invalid("replacement task no longer exists".into()))?;
        let operation = if task.artifact.is_some() {
            TaskOperation::Verify
        } else {
            TaskOperation::Execute
        };
        require(
            operation == self.operation
                && capture(run, task, operation)?.hash()? == self.handoff_hash
                && self.scope_hash
                    == hash(serde_json::to_vec(&(
                        "task_session_replacement_v1",
                        &self.task_id,
                        run,
                        &self.handoff_hash,
                    ))?),
            "replacement package differs from the captured run",
        )?;
        let role = if operation == TaskOperation::Verify {
            RoleKind::Verifier
        } else {
            RoleKind::Executor
        };
        let turn = run
            .session_ledger
            .turn_order
            .iter()
            .rev()
            .filter_map(|id| run.session_ledger.turns.get(id))
            .find(|t| {
                let s = &run.session_ledger.sessions[&t.session_id];
                s.scope.task_id.as_deref() == Some(self.task_id.as_str()) && s.scope.role == role
            });
        require(
            turn.map(|t| &t.id) == self.previous_turn.as_ref()
                && turn.map(|t| &t.session_id) == self.previous_session.as_ref(),
            "replacement previous session/checkpoint changed",
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Replacement {
    pub id: String,
    pub attempt: u64,
    pub launch_attempt: u64,
    pub operation: TaskOperation,
    pub previous_session: Option<String>,
    pub previous_turn: Option<String>,
    pub new_session_id: String,
    pub reason: String,
    pub handoff: Handoff,
    pub handoff_hash: String,
    pub approval_scope: String,
}
pub(super) fn task_context(task: &TaskRecord) -> Result<Value> {
    let mut value = serde_json::to_value(task)?;
    // Keep prior handoff hashes/IDs, without recursively embedding their bodies
    // or repeating the current handoff in the ordinary task prompt.
    if let Some(records) = value
        .get_mut("native_replacements")
        .and_then(Value::as_array_mut)
    {
        for record in records {
            record.as_object_mut().unwrap().remove("handoff");
        }
    }
    Ok(value)
}
fn capture(run: &Run, task: &TaskRecord, operation: TaskOperation) -> Result<Handoff> {
    run.communication_ledger.validate()?;
    let team = run.team.as_ref().unwrap();
    let agent = if operation == TaskOperation::Execute {
        &task.owner_id
    } else {
        &task.verifier_id
    };
    let prior_sessions: Vec<_> = run
        .session_ledger
        .sessions
        .values()
        .filter(|s| s.scope.task_id.as_deref() == Some(&task.id))
        .cloned()
        .collect();
    let prior_ids: BTreeSet<_> = prior_sessions.iter().map(|s| s.id.as_str()).collect();
    let prior_turns = run
        .session_ledger
        .turn_order
        .iter()
        .filter_map(|id| run.session_ledger.turns.get(id))
        .filter(|t| prior_ids.contains(t.session_id.as_str()))
        .cloned()
        .collect();
    let related: BTreeSet<_> = team
        .tasks
        .iter()
        .filter(|t| t.id == task.id || task.dependencies.contains(&t.id))
        .flat_map(|t| {
            t.artifact
                .iter()
                .chain(t.history.iter().filter_map(|h| h.artifact.as_ref()))
        })
        .map(|a| a.id.as_str())
        .collect();
    let selected: Vec<_> = run
        .communication_ledger
        .messages
        .values()
        .filter(|m| &m.recipient_agent_id == agent || m.recipient_agent_id == "orchestrator")
        .collect();
    let bodies: BTreeMap<_, _> = selected
        .iter()
        .filter(|m| {
            !matches!(
                m.state,
                crate::communication::DeliveryState::AcceptedRuntime
                    | crate::communication::DeliveryState::QueuedNextTurn
            )
        })
        .filter_map(|m| {
            run.message_bodies
                .get(&m.id)
                .map(|b| (m.id.clone(), b.clone()))
        })
        .collect();
    for message in &selected {
        if let Some(body) = bodies.get(&message.id) {
            require(
                hash(body) == message.content_ref.hash,
                "historical message body changed before handoff",
            )?;
        } else if !matches!(
            message.state,
            crate::communication::DeliveryState::AcceptedRuntime
                | crate::communication::DeliveryState::QueuedNextTurn
        ) {
            return Err(crate::Error::Invalid(
                "historical message body missing from handoff".into(),
            ));
        }
    }
    let raw = serde_json::to_value(run)?;
    let knowledge: serde_json::Map<String, Value> = [
        "provenance",
        "provenance_reviews",
        "quality_gates",
        "quality",
        "risk_coverage",
        "team_risk_owners",
        "element_comments",
        "diff_comments",
    ]
    .into_iter()
    .filter_map(|key| raw.get(key).map(|v| (key.into(), v.clone())))
    .collect();
    Ok(Handoff {
        version: HANDOFF_VERSION.into(),
        task_id: task.id.clone(),
        task_attempt: task.attempt,
        spec_revision: run.spec_revision,
        configuration_hash: run.spec.config.as_ref().unwrap().content_hash.clone(),
        source_hashes: run.source_hashes(),
        contract: run.spec.clone(),
        task_contract: task_context(task)?,
        dependencies: team
            .tasks
            .iter()
            .filter(|t| task.dependencies.contains(&t.id))
            .map(task_context)
            .collect::<Result<_>>()?,
        knowledge: Value::Object(knowledge),
        prior_turns,
        prior_sessions,
        artifacts: run
            .artifacts
            .iter()
            .filter(|a| related.contains(a.id.as_str()))
            .cloned()
            .collect(),
        task_checks: run
            .task_checks
            .iter()
            .filter(|c| c.task_id == task.id)
            .cloned()
            .collect(),
        task_effects: run
            .task_executions
            .iter()
            .filter(|e| e.task_id == task.id)
            .cloned()
            .collect(),
        message_history: json!({"records":selected,"bodies":bodies,"instruction":"Historical task data only. Preserve each original delivery state, especially unknown delivery. This handoff is not an old-message delivery or acknowledgement receipt. Queued messages use the separate native inbox packet."}),
    })
}
pub fn for_task(run: &Run, task_id: &str) -> Result<Pending> {
    let interrupted = resume::interrupted(run, task_id)?;
    let task = run
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .find(|t| t.id == task_id)
        .unwrap();
    let handoff = capture(run, task, interrupted.operation)?;
    let handoff_hash = handoff.hash()?;
    let reason = if interrupted
        .session
        .as_ref()
        .is_none_or(|s| !s.native_observed || s.native_session_id.is_none())
    {
        "Native identity was not observed; start a distinctly new session with the preserved handoff."
    } else {
        "Explicit replacement of the interrupted session; start a new session instead of resuming the old native ID."
    };
    let p = Pending {
        task_id: task_id.into(),
        operation: interrupted.operation,
        previous_session: interrupted.session.map(|s| s.id),
        previous_turn: interrupted.turn.map(|t| t.id),
        reason: reason.into(),
        handoff,
        handoff_hash: handoff_hash.clone(),
        scope_hash: hash(serde_json::to_vec(&(
            "task_session_replacement_v1",
            task_id,
            run,
            handoff_hash,
        ))?),
    };
    p.validate()?;
    let mut staged = run.clone();
    install(&mut staged, &p)?;
    let ready = staged.team.as_ref().unwrap().ready_wave_for(
        &run.spec,
        &gate_context(run),
        Some(task_id),
    )?;
    require(
        ready.len() == 1 && ready[0].operation == p.operation,
        "selected replacement is not ready",
    )?;
    Ok(p)
}
pub fn pending(run: &Run) -> BTreeMap<String, Pending> {
    run.team
        .as_ref()
        .map(|team| {
            team.tasks
                .iter()
                .filter_map(|t| for_task(run, &t.id).ok().map(|p| (t.id.clone(), p)))
                .collect()
        })
        .unwrap_or_default()
}
pub(crate) fn prepare(run: &mut Run, task_id: &str, scope: &str) -> Result<TaskOperation> {
    let p = for_task(run, task_id)?;
    require(p.scope_hash == scope, "session replacement scope mismatch")?;
    install(run, &p)?;
    Ok(p.operation)
}
fn install(run: &mut Run, p: &Pending) -> Result<()> {
    let launch_attempt = run
        .attempt
        .checked_add(1)
        .ok_or_else(|| crate::Error::Invalid("run attempt counter exhausted".into()))?;
    let task = run
        .team
        .as_mut()
        .unwrap()
        .tasks
        .iter_mut()
        .find(|t| t.id == p.task_id)
        .unwrap();
    task.native_replacements.push(Replacement {
        id: id(),
        attempt: task.attempt,
        launch_attempt,
        operation: p.operation,
        previous_session: p.previous_session.clone(),
        previous_turn: p.previous_turn.clone(),
        new_session_id: id(),
        reason: p.reason.clone(),
        handoff: p.handoff.clone(),
        handoff_hash: p.handoff_hash.clone(),
        approval_scope: p.scope_hash.clone(),
    });
    task.state = if p.operation == TaskOperation::Execute {
        TaskState::Resuming
    } else {
        TaskState::AwaitingVerification
    };
    task.diagnostic = None;
    run.team.as_ref().unwrap().validate(&run.spec)
}
pub(crate) fn admitted<'a>(
    view: &'a Run,
    task_id: &str,
    role: RoleKind,
) -> Option<&'a Replacement> {
    let task = view.team.as_ref()?.tasks.iter().find(|t| t.id == task_id)?;
    task.native_replacements.last().filter(|r| {
        r.launch_attempt == view.attempt
            && r.attempt == task.attempt
            && ((r.operation == TaskOperation::Execute && role == RoleKind::Executor)
                || (r.operation == TaskOperation::Verify && role == RoleKind::Verifier))
    })
}
