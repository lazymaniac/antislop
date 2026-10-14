//! User-confirmed observed after-state of uncertain mediated file edits. Never replay writes.
use super::*;
use crate::{
    recovery::{EditReceipt, EditState},
    sessions::SessionScope,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObservationReceipt {
    pub id: String,
    pub actor: String,
    pub scope_hash: String,
    pub original_receipts: Vec<EditReceipt>,
    pub observed_snapshot: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cohort: Option<CohortBinding>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CohortMember {
    pub execution_id: String,
    pub scope_hash: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CohortBinding {
    pub scope_hash: String,
    pub members: Vec<CohortMember>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingWave {
    pub wave_attempt: u64,
    pub task_ids: Vec<String>,
    pub scope_hash: String,
}
fn cohort_scope(run_id: &str, members: &[CohortMember]) -> Result<String> {
    Ok(hash(serde_json::to_vec(&(
        "observe_wave_effects_v1",
        run_id,
        members,
    ))?))
}
fn receipt_scope(
    run_id: &str,
    execution: &effects::ExecutionBinding,
    receipt: &ObservationReceipt,
) -> Result<String> {
    Ok(hash(serde_json::to_vec(&(
        "observe_task_effects_v1",
        run_id,
        &execution.id,
        &execution.task_id,
        execution.attempt,
        execution.wave_attempt,
        &execution.artifact_hash,
        &execution.scope,
        &execution.session_id,
        &execution.input_snapshot,
        &execution.source_hashes,
        &execution.dependency_hashes,
        &receipt.original_receipts,
        &receipt.observed_snapshot,
    ))?))
}
fn approved(
    run: &Run,
    execution: &effects::ExecutionBinding,
    receipt: &ObservationReceipt,
) -> Result<bool> {
    let (action, scope) = if let Some(cohort) = &receipt.cohort {
        require(
            !cohort.members.is_empty()
                && cohort.scope_hash == cohort_scope(&run.id, &cohort.members)?
                && cohort
                    .members
                    .iter()
                    .any(|m| m.execution_id == execution.id && m.scope_hash == receipt.scope_hash)
                && cohort
                    .members
                    .windows(2)
                    .all(|w| w[0].execution_id < w[1].execution_id),
            "effect cohort membership/scope changed",
        )?;
        for member in &cohort.members {
            let other = run
                .task_executions
                .iter()
                .find(|e| e.id == member.execution_id)
                .ok_or_else(|| crate::Error::Invalid("effect cohort member missing".into()))?;
            let observed = other.reconciliation.as_ref().ok_or_else(|| {
                crate::Error::Invalid("effect cohort member observation missing".into())
            })?;
            require(
                matches!(
                    other.state,
                    effects::ExecutionState::ObservedApplied | effects::ExecutionState::Committed
                ) && observed.original_receipts.len() == other.receipts.len()
                    && observed.original_receipts.iter().zip(&other.receipts).all(
                        |(original, current)| {
                            original.path == current.path
                                && original.before_hash == current.before_hash
                                && original.after_hash == current.after_hash
                                && current.state == EditState::Applied
                        },
                    ),
                "effect cohort member intent/outcome changed",
            )?;
            let proposal = effects::proposal(run, other)?;
            require(
                proposal.edits.len() == other.receipts.len()
                    && proposal
                        .edits
                        .iter()
                        .zip(&other.receipts)
                        .all(|(edit, effect)| {
                            edit.path == effect.path
                                && hash(&edit.content) == effect.after_hash
                                && other
                                    .input_snapshot
                                    .get(&effect.path)
                                    .is_some_and(|text| hash(text) == effect.before_hash)
                                && observed
                                    .observed_snapshot
                                    .get(&effect.path)
                                    .is_some_and(|text| hash(text) == effect.after_hash)
                        }),
                "effect cohort native proposal/after-state changed",
            )?;
            require(
                other.wave_attempt == execution.wave_attempt
                    && other.scope.spec_revision == execution.scope.spec_revision
                    && observed.actor == "local_user"
                    && observed.scope_hash == member.scope_hash
                    && observed.scope_hash == receipt_scope(&run.id, other, observed)?
                    && observed.cohort.as_ref() == Some(cohort)
                    && observed.observed_snapshot == receipt.observed_snapshot
                    && other.after_snapshot.as_ref() == Some(&observed.observed_snapshot),
                "effect cohort member proof changed",
            )?;
            authenticate_evidence(run, other, observed)?;
        }
        ("reconcile_wave_effects", cohort.scope_hash.as_str())
    } else {
        ("reconcile_task_effects", receipt.scope_hash.as_str())
    };
    Ok(run.approvals.iter().any(|a| {
        a.actor == "local_user"
            && a.action == action
            && a.scope_hash == scope
            && a.spec_revision == execution.scope.spec_revision
    }))
}
fn authenticate_evidence(
    run: &Run,
    execution: &effects::ExecutionBinding,
    receipt: &ObservationReceipt,
) -> Result<()> {
    let output = serde_json::to_string(receipt)?;
    let input_hash = hash(serde_json::to_vec(&receipt.observed_snapshot)?);
    require(
        run.evidence.iter().any(|e| {
            e.source == format!("runtime:observed-task-effects:{}", execution.id)
                && e.success
                && e.spec_revision == execution.scope.spec_revision
                && e.artifact_hash == execution.artifact_hash
                && e.output == output
                && e.output_hash == hash(&output)
                && e.input_hash.as_ref() == Some(&input_hash)
        }),
        "effect observation has no exact retained evidence",
    )
}
pub(super) fn authenticate_receipt(run: &Run, execution: &effects::ExecutionBinding) -> Result<()> {
    let receipt = execution
        .reconciliation
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("effect observation receipt missing".into()))?;
    require(
        receipt.actor == "local_user"
            && receipt.scope_hash == receipt_scope(&run.id, execution, receipt)?
            && execution.after_snapshot.as_ref() == Some(&receipt.observed_snapshot)
            && receipt.original_receipts.len() == execution.receipts.len()
            && receipt
                .original_receipts
                .iter()
                .zip(&execution.receipts)
                .all(|(original, current)| {
                    original.path == current.path
                        && original.before_hash == current.before_hash
                        && original.after_hash == current.after_hash
                })
            && approved(run, execution, receipt)?,
        "observed effect state has no exact user approval and retained intent",
    )?;
    authenticate_evidence(run, execution, receipt)
}

fn owned_candidate(
    run: &Run,
    task_id: &str,
    observed: &BTreeMap<String, String>,
) -> Result<(effects::ExecutionBinding, ObservationReceipt)> {
    let team = run.team.as_ref().unwrap();
    let task = team
        .tasks
        .iter()
        .find(|t| t.id == task_id)
        .ok_or_else(|| crate::Error::Invalid("unknown effect owner".into()))?;
    require(
        task.state == TaskState::Blocked && task.artifact.is_none() && task.review.is_none(),
        "effect observation cannot replace a result or review",
    )?;
    let candidates = run
        .task_executions
        .iter()
        .filter(|e| e.task_id == task_id && e.attempt == task.attempt)
        .collect::<Vec<_>>();
    require(candidates.len() == 1, "effect intent missing or ambiguous")?;
    let execution = candidates[0];
    require(
        execution.state == effects::ExecutionState::Intent
            && execution.reconciliation.is_none()
            && execution.after_snapshot.is_none()
            && !execution.receipts.is_empty()
            && execution.source_hashes == run.source_hashes_for_inputs(&execution.input_snapshot)
            && execution.dependency_hashes == team.dependency_bindings(task)?,
        "effect intent scope/outcome changed",
    )?;
    let output = effects::proposal(run, execution)?;
    let owner = team.agents.iter().find(|a| a.id == task.owner_id).unwrap();
    require(
        owner.session_id.as_ref() == Some(&execution.session_id),
        "effect owner native session changed",
    )?;
    let mut original = run.clone();
    original.input_snapshot = execution.input_snapshot.clone();
    original.stage = match task.stage {
        TaskStage::Discovery => Stage::Discovery,
        TaskStage::Planning => Stage::Planning,
        TaskStage::Execution => Stage::Execution,
        TaskStage::Validation => Stage::Validation,
        TaskStage::Integration => Stage::Integration,
        TaskStage::Delivery => Stage::Delivery,
    };
    let provider = run
        .spec
        .config
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("effect configuration missing".into()))?
        .config
        .resolve(owner.role.work_kind(), owner.kind)?
        .provider;
    require(
        execution.scope
            == SessionScope::for_run(
                &original,
                owner.id.clone(),
                Some(task.id.clone()),
                owner.kind,
                execution.scope.provider.clone(),
                execution.scope.binary_version.clone(),
            )?
            && serde_json::to_value(provider)?.as_str() == Some(execution.scope.provider.as_str()),
        "effect native scope/configuration changed",
    )?;
    crate::team::models::validate_output(&original, task, owner.role, &output)?;
    effects::lineage(run, task)?;
    require(
        output.edits.len() == execution.receipts.len(),
        "native edits do not match intent",
    )?;
    let mut paths = BTreeSet::new();
    for (edit, receipt) in output.edits.iter().zip(&execution.receipts) {
        require(
            paths.insert(&receipt.path)
                && edit.path == receipt.path
                && run.spec.editable_files.contains(&edit.path)
                && task.write_scopes.iter().any(|scope| {
                    edit.path == *scope || edit.path.starts_with(&format!("{scope}/"))
                })
                && execution
                    .input_snapshot
                    .get(&receipt.path)
                    .is_some_and(|text| hash(text) == receipt.before_hash)
                && hash(&edit.content) == receipt.after_hash
                && observed
                    .get(&receipt.path)
                    .is_some_and(|text| hash(text) == receipt.after_hash)
                && matches!(receipt.state, EditState::Pending | EditState::Applied),
            "owned file is unapplied, conflicting or no longer matches the approved intent",
        )?;
        require(
            run.pending_edits.iter().any(|pending| {
                pending.path == receipt.path
                    && pending.before_hash == receipt.before_hash
                    && pending.after_hash == receipt.after_hash
            }),
            "owned pending intent is missing",
        )?;
    }
    let mut receipt = ObservationReceipt {
        id: id(),
        actor: "local_user".into(),
        scope_hash: String::new(),
        original_receipts: execution.receipts.clone(),
        observed_snapshot: observed.clone(),
        cohort: None,
    };
    receipt.scope_hash = receipt_scope(&run.id, execution, &receipt)?;
    Ok((execution.clone(), receipt))
}
fn candidates(
    run: &Run,
    task_ids: &[String],
) -> Result<Vec<(effects::ExecutionBinding, ObservationReceipt)>> {
    require(
        run.reconciliation_required && run.status == Status::Blocked,
        "effect adjudication requires interrupted stopped work",
    )?;
    require(
        run.checklist_version == crate::domain::checklist_version(),
        "effect checklist changed; explicitly revise",
    )?;
    let team = run
        .team
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("no installed team for effect adjudication".into()))?;
    team.validate(&run.spec)?;
    require(
        team.spec_revision == run.spec_revision,
        "effect team revision is stale",
    )?;
    require(
        !task_ids.is_empty() && task_ids.iter().collect::<BTreeSet<_>>().len() == task_ids.len(),
        "empty or duplicate effect cohort",
    )?;
    let observed =
        crate::workspace::read_inputs(Path::new(&run.spec.workspace), &run.spec.input_files)?;
    require(
        observed.len() == run.input_snapshot.len(),
        "pinned input inventory changed",
    )?;
    let mut selected = task_ids
        .iter()
        .map(|id| owned_candidate(run, id, &observed))
        .collect::<Result<Vec<_>>>()?;
    selected.sort_by(|a, b| a.0.id.cmp(&b.0.id));
    let wave = selected[0].0.wave_attempt;
    require(
        selected
            .iter()
            .all(|(execution, _)| execution.wave_attempt == wave),
        "effect cohort spans different original waves",
    )?;
    let mut paths = BTreeSet::new();
    for (execution, _) in &selected {
        for receipt in &execution.receipts {
            require(
                paths.insert(&receipt.path),
                "effect cohort has ambiguous overlapping ownership",
            )?;
        }
    }
    for (path, text) in &observed {
        if run.input_snapshot.get(path) == Some(text) || paths.contains(path) {
            continue;
        }
        require(
            run.task_executions.iter().any(|other| {
                other.wave_attempt == wave
                    && matches!(
                        other.state,
                        effects::ExecutionState::Applied
                            | effects::ExecutionState::ObservedApplied
                            | effects::ExecutionState::Committed
                    )
                    && effects::authenticate(run, other).is_ok()
                    && other.receipts.iter().any(|r| {
                        r.path == *path
                            && r.state == EditState::Applied
                            && r.after_hash == hash(text)
                    })
            }),
            "unselected or unowned workspace changes require complete cohort reconciliation",
        )?;
    }
    Ok(selected)
}
fn candidate(run: &Run, task_id: &str) -> Result<(effects::ExecutionBinding, ObservationReceipt)> {
    Ok(candidates(run, &[task_id.to_owned()])?.remove(0))
}

pub(crate) fn scope(run: &Run, task_id: &str) -> Result<String> {
    Ok(candidate(run, task_id)?.1.scope_hash)
}
pub(crate) fn scopes(run: &Run) -> BTreeMap<String, String> {
    run.team
        .as_ref()
        .map(|t| {
            t.tasks
                .iter()
                .filter(|task| task.state == TaskState::Blocked && task.artifact.is_none())
                .filter_map(|task| scope(run, &task.id).ok().map(|s| (task.id.clone(), s)))
                .collect()
        })
        .unwrap_or_default()
}
pub(crate) fn reconcile(run: &mut Run, task_id: &str, expected_scope: &str) -> Result<()> {
    let selected = vec![candidate(run, task_id)?];
    require(
        selected[0].1.scope_hash == expected_scope,
        "observed task effect scope changed",
    )?;
    apply_observations(run, selected, "reconcile_task_effects", expected_scope)
}
pub(crate) fn wave_scope(run: &Run, task_ids: &[String]) -> Result<String> {
    let selected = candidates(run, task_ids)?;
    cohort_scope(
        &run.id,
        &selected
            .iter()
            .map(|(e, r)| CohortMember {
                execution_id: e.id.clone(),
                scope_hash: r.scope_hash.clone(),
            })
            .collect::<Vec<_>>(),
    )
}
pub fn pending_waves(run: &Run) -> BTreeMap<String, PendingWave> {
    let mut waves: BTreeMap<u64, Vec<String>> = BTreeMap::new();
    for execution in run
        .task_executions
        .iter()
        .filter(|e| e.state == effects::ExecutionState::Intent && !e.receipts.is_empty())
    {
        waves
            .entry(execution.wave_attempt)
            .or_default()
            .push(execution.task_id.clone());
    }
    waves
        .into_iter()
        .filter_map(|(wave, mut ids)| {
            ids.sort();
            wave_scope(run, &ids).ok().map(|scope| {
                (
                    wave.to_string(),
                    PendingWave {
                        wave_attempt: wave,
                        task_ids: ids,
                        scope_hash: scope,
                    },
                )
            })
        })
        .collect()
}
pub(crate) fn reconcile_wave(
    run: &mut Run,
    task_ids: &[String],
    expected_scope: &str,
) -> Result<()> {
    let mut selected = candidates(run, task_ids)?;
    let members = selected
        .iter()
        .map(|(e, r)| CohortMember {
            execution_id: e.id.clone(),
            scope_hash: r.scope_hash.clone(),
        })
        .collect::<Vec<_>>();
    let scope = cohort_scope(&run.id, &members)?;
    require(
        scope == expected_scope,
        "observed wave effect scope changed",
    )?;
    let cohort = CohortBinding {
        scope_hash: scope,
        members,
    };
    for (_, receipt) in &mut selected {
        receipt.cohort = Some(cohort.clone());
    }
    apply_observations(run, selected, "reconcile_wave_effects", expected_scope)
}
fn apply_observations(
    run: &mut Run,
    selected: Vec<(effects::ExecutionBinding, ObservationReceipt)>,
    action: &str,
    expected_scope: &str,
) -> Result<()> {
    let observed = selected[0].1.observed_snapshot.clone();
    for (execution, receipt) in &selected {
        let index = run
            .task_executions
            .iter()
            .position(|e| e.id == execution.id)
            .unwrap();
        for effect in &mut run.task_executions[index].receipts {
            effect.state = EditState::Applied;
        }
        run.task_executions[index].after_snapshot = Some(observed.clone());
        run.task_executions[index].state = effects::ExecutionState::ObservedApplied;
        run.task_executions[index].reconciliation = Some(receipt.clone());
        for pending in &mut run.pending_edits {
            if execution.receipts.iter().any(|e| {
                e.path == pending.path
                    && e.before_hash == pending.before_hash
                    && e.after_hash == pending.after_hash
            }) {
                pending.state = EditState::Applied;
            }
        }
        let output = serde_json::to_string(receipt)?;
        run.evidence.push(Evidence {
            id: id(),
            artifact_hash: execution.artifact_hash.clone(),
            spec_revision: run.spec_revision,
            source: format!("runtime:observed-task-effects:{}", execution.id),
            output_hash: hash(&output),
            output,
            success: true,
            input_hash: Some(hash(serde_json::to_vec(&observed)?)),
        });
    }
    run.input_snapshot = observed;
    run.approvals.push(crate::domain::Approval {
        actor: "local_user".into(),
        action: action.into(),
        scope_hash: expected_scope.into(),
        spec_revision: run.spec_revision,
    });
    for (execution, _) in &selected {
        effects::authenticate(
            run,
            run.task_executions
                .iter()
                .find(|e| e.id == execution.id)
                .unwrap(),
        )?;
    }
    let settled = run
        .pending_edits
        .iter()
        .all(|e| e.state == EditState::Applied)
        && !run
            .task_executions
            .iter()
            .any(|e| e.state == effects::ExecutionState::Intent)
        && !run.task_checks.iter().any(|c| {
            matches!(
                c.state,
                checks::CheckState::Intent | checks::CheckState::Unknown
            )
        })
        && !run.session_ledger.turns.values().any(|t| {
            matches!(
                t.state,
                crate::sessions::TurnState::PendingUnknown
                    | crate::sessions::TurnState::Pending
                    | crate::sessions::TurnState::Sent
            )
        });
    if settled {
        run.pending_edits.clear();
    }
    run.reconciliation_required = !settled;
    let context = gate_context(run);
    run.team.as_mut().unwrap().invalidate_sources(&context);
    crate::verification::refresh_provenance(run);
    run.diagnostic = Some("Approved file after-state observed without repeating writes; original effect origins and complete selected cohort retained. Restore completed results, then independently refresh/review current sources".into());
    Ok(())
}
