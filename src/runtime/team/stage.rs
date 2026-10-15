//! Explicit acceptance of independently verified logical-stage result packages.
use super::*;
use crate::team::{TaskRecord, TaskStage};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultBinding {
    pub task_id: String,
    pub task_attempt: u64,
    pub task_hash: String,
    pub artifact_id: String,
    pub artifact_hash: String,
    pub review_hash: String,
    pub dependency_hashes: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultApproval {
    pub id: String,
    pub stage: TaskStage,
    pub spec_revision: u64,
    pub team_revision: u64,
    pub checklist_version: String,
    pub configuration_hash: String,
    pub source_hashes: BTreeMap<String, String>,
    pub results: Vec<ResultBinding>,
    pub scope_hash: String,
    pub actor: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingResult {
    pub stage: TaskStage,
    pub task_ids: Vec<String>,
    pub results: Vec<ResultBinding>,
    pub scope_hash: String,
}
pub fn stage_key(stage: TaskStage) -> String {
    serde_json::to_value(stage)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}
fn configuration(run: &Run) -> String {
    run.spec
        .config
        .as_ref()
        .map(|c| c.content_hash.clone())
        .unwrap_or_else(|| hash(serde_json::to_vec(&run.spec).unwrap()))
}
fn binding(run: &Run, task: &TaskRecord) -> Result<ResultBinding> {
    let (artifact, review) = authenticated_result(run, task)?;
    Ok(ResultBinding {
        task_id: task.id.clone(),
        task_attempt: task.attempt,
        task_hash: hash(serde_json::to_vec(task)?),
        artifact_id: artifact.id.clone(),
        artifact_hash: artifact.hash.clone(),
        review_hash: hash(serde_json::to_vec(review)?),
        dependency_hashes: task.artifact.as_ref().unwrap().dependency_hashes.clone(),
    })
}
fn approval_scope(run_id: &str, approval: &ResultApproval) -> String {
    hash(
        serde_json::to_vec(&(
            run_id,
            "accept_team_stage",
            approval.stage,
            approval.spec_revision,
            approval.team_revision,
            &approval.checklist_version,
            &approval.configuration_hash,
            &approval.source_hashes,
            &approval.results,
        ))
        .unwrap(),
    )
}
fn record_current(run: &Run, approval: &ResultApproval) -> bool {
    approval.actor == "local_user"
        && approval.spec_revision == run.spec_revision
        && run
            .team
            .as_ref()
            .is_some_and(|t| approval.team_revision == t.revision)
        && approval.checklist_version == run.checklist_version
        && run.mode_configuration_matches(&approval.configuration_hash)
        && approval.source_hashes == run.source_hashes()
        && approval.scope_hash == approval_scope(&run.id, approval)
        && run.approvals.iter().any(|a| {
            a.actor == "local_user"
                && a.action == "accept_team_stage"
                && a.spec_revision == approval.spec_revision
                && a.scope_hash == approval.scope_hash
        })
}
/// Valid only for the supplied immutable run; never reused across snapshots.
pub(super) fn current_results(run: &Run) -> Result<BTreeMap<String, ResultBinding>> {
    require(
        run.checklist_version == crate::domain::checklist_version(),
        "stage result checklist changed; explicit revision required",
    )?;
    let team = run
        .team
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("no installed team".into()))?;
    team.validate(&run.spec)?;
    require(
        team.spec_revision == run.spec_revision,
        "stage result team revision is stale",
    )?;
    Ok(team
        .tasks
        .iter()
        .filter(|t| t.state == TaskState::Verified)
        .filter_map(|t| binding(run, t).ok().map(|b| (t.id.clone(), b)))
        .collect())
}
pub(super) fn accepted_ids_for(
    run: &Run,
    results: &BTreeMap<String, ResultBinding>,
) -> BTreeSet<String> {
    let mut accepted = BTreeSet::new();
    let Some(team) = &run.team else {
        return accepted;
    };
    for approval in run
        .team_result_approvals
        .iter()
        .filter(|a| record_current(run, a))
    {
        for result in &approval.results {
            if results.get(&result.task_id) == Some(result)
                && team
                    .tasks
                    .iter()
                    .any(|t| t.id == result.task_id && t.stage == approval.stage)
            {
                accepted.insert(result.task_id.clone());
            }
        }
    }
    accepted
}
fn candidate(run: &Run, stage: TaskStage, task_ids: &[String]) -> Result<ResultApproval> {
    candidate_for(run, stage, task_ids, &current_results(run)?)
}
fn candidate_for(
    run: &Run,
    stage: TaskStage,
    task_ids: &[String],
    current: &BTreeMap<String, ResultBinding>,
) -> Result<ResultApproval> {
    require(
        !run.reconciliation_required
            && run
                .pending_edits
                .iter()
                .all(|e| e.state == crate::recovery::EditState::Applied),
        "reconcile before accepting stage results",
    )?;
    let team = run
        .team
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("no installed team".into()))?;
    require(
        !team.checkpoint_uncertain && !task_ids.is_empty(),
        "uncertain or empty stage result package",
    )?;
    let accepted = accepted_ids_for(run, current);
    let mut seen = BTreeSet::new();
    let mut results = Vec::new();
    for task_id in task_ids {
        require(
            seen.insert(task_id) && !accepted.contains(task_id),
            "duplicate or already accepted stage result",
        )?;
        let task = team
            .tasks
            .iter()
            .find(|t| &t.id == task_id && t.stage == stage)
            .ok_or_else(|| {
                crate::Error::Invalid(
                    "stage result task missing or belongs to another stage".into(),
                )
            })?;
        results.push(current.get(&task.id).cloned().ok_or_else(|| {
            crate::Error::Invalid("stage result is stale or unauthenticated".into())
        })?);
    }
    results.sort_by(|a, b| a.task_id.cmp(&b.task_id));
    let mut approval = ResultApproval {
        id: id(),
        stage,
        spec_revision: run.spec_revision,
        team_revision: team.revision,
        checklist_version: run.checklist_version.clone(),
        configuration_hash: configuration(run),
        source_hashes: run.source_hashes(),
        results,
        scope_hash: String::new(),
        actor: "local_user".into(),
    };
    approval.scope_hash = approval_scope(&run.id, &approval);
    Ok(approval)
}
pub fn scope(run: &Run, stage: TaskStage, task_ids: &[String]) -> Result<String> {
    Ok(candidate(run, stage, task_ids)?.scope_hash)
}
pub fn pending(run: &Run) -> BTreeMap<String, PendingResult> {
    let mut grouped: BTreeMap<String, (TaskStage, Vec<String>)> = BTreeMap::new();
    let Ok(current) = current_results(run) else {
        return BTreeMap::new();
    };
    let accepted = accepted_ids_for(run, &current);
    if let Some(team) = &run.team {
        for task in team
            .tasks
            .iter()
            .filter(|t| t.state == TaskState::Verified && !accepted.contains(&t.id))
        {
            if current.contains_key(&task.id) {
                grouped
                    .entry(stage_key(task.stage))
                    .or_insert((task.stage, vec![]))
                    .1
                    .push(task.id.clone());
            }
        }
    }
    grouped
        .into_iter()
        .filter_map(|(key, (stage, mut task_ids))| {
            task_ids.sort();
            candidate_for(run, stage, &task_ids, &current)
                .ok()
                .map(|a| {
                    (
                        key,
                        PendingResult {
                            stage,
                            task_ids,
                            results: a.results,
                            scope_hash: a.scope_hash,
                        },
                    )
                })
        })
        .collect()
}
pub(crate) fn accept(
    run: &mut Run,
    stage: TaskStage,
    task_ids: &[String],
    scope_hash: &str,
) -> Result<()> {
    require(
        matches!(
            run.status,
            Status::Ready | Status::AwaitingResult | Status::Blocked | Status::ChangesRequested
        ),
        "run does not await a stage result acceptance",
    )?;
    let current = current_results(run)?;
    let approval = candidate_for(run, stage, task_ids, &current)?;
    require(
        approval.scope_hash == scope_hash,
        "stage result package changed since it was displayed",
    )?;
    run.approvals.push(crate::domain::Approval {
        action: "accept_team_stage".into(),
        scope_hash: scope_hash.into(),
        spec_revision: run.spec_revision,
        actor: "local_user".into(),
    });
    run.team_result_approvals.push(approval);
    let accepted = accepted_ids_for(run, &current);
    if run.status == Status::AwaitingResult {
        run.status = if current.keys().all(|id| accepted.contains(id)) {
            Status::Ready
        } else {
            Status::AwaitingResult
        };
        if run.status == Status::Ready {
            run.diagnostic = None;
        }
    }
    if run
        .team
        .as_ref()
        .unwrap()
        .tasks
        .iter()
        .all(|t| t.state == TaskState::Verified)
    {
        require_final_gates(run)?;
        if run
            .team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .all(|t| accepted.contains(&t.id))
        {
            run.status = Status::Completed;
        }
    }
    Ok(())
}
pub(super) fn require_final_gates(run: &Run) -> Result<()> {
    require_final_gates_in(run.spec.task_family, &gate_context(run))
}

pub fn delivery_binding(run: &Run) -> Result<String> {
    require(
        run.status == Status::Completed && !run.reconciliation_required,
        "delivery requires a completed reconciled task",
    )?;
    let results = current_results(run)?;
    let accepted = accepted_ids_for(run, &results);
    require(
        run.team
            .as_ref()
            .unwrap()
            .tasks
            .iter()
            .all(|task| results.contains_key(&task.id) && accepted.contains(&task.id)),
        "delivery requires independently authenticated and accepted results for the entire team",
    )?;
    require_final_gates(run)?;
    Ok(hash(serde_json::to_vec(&(
        &run.id,
        run.spec_revision,
        run.source_hashes(),
        results,
        &run.team_result_approvals,
    ))?))
}
pub(super) fn require_final_gates_in(family: TaskFamily, context: &GateContext) -> Result<()> {
    let missing = crate::quality::gate_definitions()
        .iter()
        .filter(|g| !g.software_only || family == TaskFamily::Software)
        .filter_map(|g| {
            let key = crate::quality::gate_check_id(&g.gate_id, g.phase);
            let current = context.gates.get(&key).is_some_and(|gate| {
                gate.passed
                    && gate.spec_revision == context.spec_revision
                    && gate.source_hashes == context.source_hashes
                    && !gate.evidence_refs.is_empty()
            });
            (!current).then_some(key)
        })
        .collect::<Vec<_>>();
    require(
        missing.is_empty(),
        &format!(
            "final quality gates are incomplete or stale: {}",
            missing.join(", ")
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn final_acceptance_requires_every_exact_gate_phase_and_current_positive_evidence() {
        let mut context = GateContext {
            spec_revision: 1,
            source_hashes: BTreeMap::from([("source".into(), hash("source"))]),
            gates: BTreeMap::new(),
            gate_tasks: BTreeMap::new(),
            accepted_results: Some(BTreeSet::new()),
        };
        let binding = GateBinding {
            spec_revision: context.spec_revision,
            source_hashes: context.source_hashes.clone(),
            evidence_refs: BTreeSet::from(["observed-receipt".into()]),
            passed: true,
        };
        for gate in crate::quality::gate_definitions() {
            context.gates.insert(
                crate::quality::gate_check_id(&gate.gate_id, gate.phase),
                binding.clone(),
            );
            context.gates.insert(gate.gate_id, binding.clone());
        }
        assert!(require_final_gates_in(TaskFamily::Software, &context).is_ok());
        let qualified = crate::quality::gate_check_id(
            "architecture_verified",
            crate::quality::GatePhase::Delivery,
        );
        context.gates.remove(&qualified);
        assert!(context.gates.contains_key("architecture_verified"));
        assert!(
            require_final_gates_in(TaskFamily::Software, &context)
                .unwrap_err()
                .to_string()
                .contains(&qualified),
            "design evidence cannot replace delivery evidence through a short alias"
        );
        context.gates.insert(qualified.clone(), binding.clone());
        context.gates.get_mut(&qualified).unwrap().passed = false;
        assert!(require_final_gates_in(TaskFamily::Software, &context).is_err());
        context.gates.insert(qualified.clone(), binding.clone());
        context
            .gates
            .get_mut(&qualified)
            .unwrap()
            .source_hashes
            .clear();
        assert!(require_final_gates_in(TaskFamily::Software, &context).is_err());
        context.gates.insert(qualified.clone(), binding);
        context
            .gates
            .get_mut(&qualified)
            .unwrap()
            .evidence_refs
            .clear();
        assert!(require_final_gates_in(TaskFamily::Software, &context).is_err());
    }
}
