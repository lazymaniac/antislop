//! Explicit verifier-only recovery with current native authorship and immutable executor output.
use super::*;
use crate::sessions::{TurnRecord, TurnState};
use crate::team::{TaskRecord, VerificationRetry};

fn scoped_turn<'a>(
    run: &'a Run,
    task: &TaskRecord,
    session_id: &str,
    verifier: bool,
) -> Result<&'a TurnRecord> {
    let team = run.team.as_ref().unwrap();
    let agent_id = if verifier {
        &task.verifier_id
    } else {
        &task.owner_id
    };
    let agent = team.agents.iter().find(|a| &a.id == agent_id).unwrap();
    let session = run
        .session_ledger
        .sessions
        .get(session_id)
        .ok_or_else(|| crate::Error::Invalid("task recovery session missing".into()))?;
    let mut staged = run.clone();
    staged.stage = match task.stage {
        TaskStage::Discovery => Stage::Discovery,
        TaskStage::Planning => Stage::Planning,
        TaskStage::Execution => Stage::Execution,
        TaskStage::Validation => Stage::Validation,
        TaskStage::Integration => Stage::Integration,
        TaskStage::Delivery => Stage::Delivery,
    };
    let configured = run
        .spec
        .config
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("task configuration missing".into()))?
        .config
        .resolve(agent.role.work_kind(), agent.kind)?
        .provider;
    require(
        session
            .scope
            .matches_run(&staged, agent.id.clone(), Some(task.id.clone()), agent.kind)?
            && serde_json::to_value(configured)?.as_str() == Some(session.scope.provider.as_str()),
        "task recovery session scope is stale or belongs to another role",
    )?;
    session
        .current_turn_id
        .as_ref()
        .and_then(|id| run.session_ledger.turns.get(id))
        .ok_or_else(|| crate::Error::Invalid("task recovery turn missing".into()))
}

fn has_negative_result(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(fields) => {
            (fields.get("material").and_then(serde_json::Value::as_bool) == Some(true)
                && fields.contains_key("question")
                && fields
                    .get("resolution")
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(|v| v.trim().is_empty()))
                || fields.iter().any(|(key, value)| {
                    (key == "accepted" && value.as_bool() == Some(false))
                        || (key == "criteria"
                            && value
                                .as_object()
                                .is_some_and(|v| v.values().any(|v| v.as_bool() == Some(false))))
                        || ((key == "status" || key == "outcome")
                            && matches!(value.as_str(), Some("fail" | "blocked")))
                        || ((key == "findings" || key.starts_with("unresolved"))
                            && !value.as_array().is_some_and(|v| v.is_empty()))
                        || has_negative_result(value)
                })
        }
        serde_json::Value::Array(values) => values.iter().any(has_negative_result),
        _ => false,
    }
}

fn previous_session(run: &Run, task_id: &str) -> Result<Option<String>> {
    require(
        !run.reconciliation_required && run.pending_edits.is_empty(),
        "reconcile before task verification recovery",
    )?;
    require(
        matches!(
            run.status,
            Status::Ready
                | Status::AwaitingResult
                | Status::Blocked
                | Status::ChangesRequested
                | Status::AwaitingStart
        ),
        "run is not stopped for task verification recovery",
    )?;
    require(
        run.checklist_version == crate::domain::checklist_version(),
        "task checklist changed",
    )?;
    run.session_ledger.validate_for_run(&run.id)?;
    let team = run
        .team
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("no team installed".into()))?;
    team.validate(&run.spec)?;
    require(
        !team.checkpoint_uncertain,
        "scheduler checkpoint requires independent recovery",
    )?;
    let task = team
        .tasks
        .iter()
        .find(|t| t.id == task_id)
        .ok_or_else(|| crate::Error::Invalid("unknown task".into()))?;
    require(
        task.state == TaskState::Blocked && task.review.is_none(),
        "only incomplete task verification can be replaced",
    )?;
    let artifact = task
        .artifact
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("task executor artifact missing".into()))?;
    require(
        artifact.source_hashes == run.source_hashes(),
        "task artifact sources changed; revalidate first",
    )?;
    let pinned = run
        .artifacts
        .iter()
        .find(|a| a.id == artifact.id)
        .ok_or_else(|| crate::Error::Invalid("task artifact content missing".into()))?;
    require(
        pinned.hash == artifact.content_hash
            && hash(&pinned.content) == pinned.hash
            && pinned.session_id == artifact.session_id
            && pinned.attempt == task.attempt
            && pinned.spec_revision == run.spec_revision,
        "task artifact identity or content mismatch",
    )?;
    let output: crate::team::models::Output = crate::transport::decode(pinned.content.as_bytes())?;
    let role = team
        .agents
        .iter()
        .find(|a| a.id == task.owner_id)
        .unwrap()
        .role;
    crate::team::models::validate_output(run, task, role, &output)?;
    let execution = effects::binding(run, artifact)?;
    effects::lineage(run, task)?;
    let mut original = run.clone();
    original.input_snapshot = execution.input_snapshot.clone();
    checks::retry_safe(run, task, pinned)?;
    let executor = scoped_turn(&original, task, &artifact.session_id, false)?;
    let session = &run.session_ledger.sessions[&artifact.session_id];
    require(
        session.native_observed
            && session.native_session_id.is_some()
            && pinned.stage == session.scope.stage
            && executor.state == TurnState::Completed
            && executor.provider_completed
            && executor.output.as_ref() == Some(&pinned.content)
            && executor.output_ref.as_ref().is_some_and(|r| {
                r.document == format!("turn:{}", executor.id) && r.hash == pinned.hash
            }),
        "task executor has no authenticated completed output",
    )?;
    for evidence_id in &artifact.evidence_refs {
        let e = run
            .evidence
            .iter()
            .find(|e| &e.id == evidence_id)
            .ok_or_else(|| crate::Error::Invalid("task artifact receipt missing".into()))?;
        require(
            e.success
                && e.source == "runtime:artifact-read"
                && e.spec_revision == run.spec_revision
                && e.artifact_hash == pinned.hash
                && e.output_hash == hash(&e.output)
                && e.output == pinned.content
                && e.input_hash.as_ref() == Some(&hash(serde_json::to_vec(&run.input_snapshot)?)),
            "task artifact receipt is stale or unauthenticated",
        )?;
    }
    // Durable turn order distinguishes this executor attempt from old repair rounds.
    let start = run
        .session_ledger
        .turn_order
        .iter()
        .position(|id| id == &executor.id)
        .ok_or_else(|| {
            crate::Error::Invalid("executor chronology missing; recovery is ambiguous".into())
        })?;
    let mut previous = None;
    for turn_id in &run.session_ledger.turn_order[start + 1..] {
        let turn = &run.session_ledger.turns[turn_id];
        let session = &run.session_ledger.sessions[&turn.session_id];
        if session.scope.task_id.as_deref() != Some(task_id)
            || session.scope.role != RoleKind::Verifier
        {
            continue;
        }
        scoped_turn(run, task, &session.id, true)?;
        require(
            !matches!(turn.state, TurnState::Pending | TurnState::Sent),
            "verifier turn is still active",
        )?;
        if let Some(raw) = &turn.output {
            let value: serde_json::Value = crate::transport::decode(raw.as_bytes())?;
            require(
                !has_negative_result(&value),
                "verifier reported a defect or rejection; replacement cannot erase it",
            )?;
            for (field, expected) in [
                ("output_hash", artifact.content_hash.clone()),
                ("input_hash", hash(serde_json::to_vec(&run.input_snapshot)?)),
                ("checklist_version", run.checklist_version.clone()),
            ] {
                require(
                    value
                        .get(field)
                        .is_none_or(|v| v.as_str() == Some(&expected)),
                    "prior verifier output claims another artifact, input or checklist",
                )?;
            }
            require(
                value
                    .pointer("/quality/synthetic")
                    .and_then(serde_json::Value::as_bool)
                    != Some(true),
                "synthetic report cannot authorize native task recovery",
            )?;
            crate::team::changes::validate_retry_claims(run, task, pinned, &value)?;
            let model_hashes = crate::team::models::model_hashes(&output);
            if let Ok(quality) =
                serde_json::from_value::<crate::quality::QualityArtifacts>(value["quality"].clone())
            {
                crate::quality::validate_planning_binding(
                    &quality,
                    output.solution.as_ref().map(|s| &s.model),
                )?;
            }
            if let Some(model_review) = value
                .pointer("/quality/model_review")
                .filter(|v| !v.is_null())
            {
                for (field, kind) in [("context_hash", "context"), ("solution_hash", "solution")] {
                    require(
                        model_review
                            .get(field)
                            .is_none_or(|v| v == &model_hashes[kind]),
                        "prior verifier inspection claims another model",
                    )?;
                }
                if let Some(facts) = model_review
                    .pointer("/independent_context/facts")
                    .and_then(serde_json::Value::as_array)
                {
                    let sources = run.source_hashes();
                    require(
                        facts.iter().all(|fact| {
                            fact["source_ref"]
                                .as_str()
                                .and_then(|source| sources.get(source))
                                .is_some_and(|digest| fact["source_hash"].as_str() == Some(digest))
                        }),
                        "prior verifier model inspection cited unknown or stale source facts",
                    )?;
                }
            }
            if let Some(checks) = value.get("checks").and_then(serde_json::Value::as_array) {
                for check in checks {
                    if let Some(ids) = check
                        .get("evidence_ids")
                        .and_then(serde_json::Value::as_array)
                    {
                        require(
                            ids.iter().all(|id| {
                                run.evidence.iter().any(|e| {
                                    Some(e.id.as_str()) == id.as_str()
                                        && (e.success || checks::baseline_expected(run, pinned, e))
                                        && e.spec_revision == run.spec_revision
                                        && hash(&e.output) == e.output_hash
                                        && crate::verification::evidence_is_current(run, pinned, e)
                                })
                            }),
                            "prior verifier cited nonexistent or stale evidence",
                        )?;
                    }
                }
            }
        }
        previous = Some(session.id.clone());
    }
    let mut candidate = team.clone();
    candidate
        .tasks
        .iter_mut()
        .find(|t| t.id == task_id)
        .unwrap()
        .state = TaskState::AwaitingVerification;
    candidate.verification_ready(&run.spec, &gate_context(run), task_id)?;
    Ok(previous)
}

pub(crate) fn scope(run: &Run, task_id: &str) -> Result<String> {
    previous_session(run, task_id)?;
    bound_scope(run, task_id)
}
fn bound_scope(run: &Run, task_id: &str) -> Result<String> {
    // Bind state, native outputs, queued inputs, policy and evidence together.
    Ok(hash(serde_json::to_vec(&(
        "retry_task_verification_v1",
        task_id,
        run,
    ))?))
}

pub(crate) fn prepare(run: &mut Run, task_id: &str, expected_scope: &str) -> Result<()> {
    let previous_session = previous_session(run, task_id)?;
    require(
        bound_scope(run, task_id)? == expected_scope,
        "task verification recovery scope mismatch",
    )?;
    let team = run.team.as_mut().unwrap();
    let task = team.tasks.iter().find(|t| t.id == task_id).unwrap();
    let retry = VerificationRetry {
        id: id(),
        attempt: task.attempt,
        artifact_hash: task.artifact.as_ref().unwrap().content_hash.clone(),
        previous_session,
        approval_scope: expected_scope.to_string(),
    };
    team.prepare_verification_retry(task_id, retry)?;
    team.validate(&run.spec)
}
