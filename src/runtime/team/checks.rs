//! Durable, policy-scoped task checks. Unknown invocation effects are never replayed.
use super::*;
use crate::team::{
    TaskRecord,
    checks::{CheckPhase, PlannedCheck},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckState {
    Intent,
    Completed,
    Unknown,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invocation {
    pub id: String,
    pub task_id: String,
    pub task_attempt: u64,
    pub artifact_id: Option<String>,
    #[serde(default)]
    pub input_snapshot: Option<BTreeMap<String, String>>,
    pub artifact_hash: String,
    pub plan: PlannedCheck,
    pub spec_revision: u64,
    pub configuration_hash: String,
    pub source_hashes: BTreeMap<String, String>,
    pub authorization_scope: String,
    pub before_hash: String,
    pub after_hash: Option<String>,
    pub state: CheckState,
    pub success: bool,
    pub output_hash: Option<String>,
    pub evidence_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restoration: Option<super::check_recovery::Restoration>,
}
pub(super) fn config_hash(run: &Run) -> String {
    run.spec
        .config
        .as_ref()
        .map(|c| c.content_hash.clone())
        .unwrap_or_else(|| hash(serde_json::to_vec(&run.spec).unwrap()))
}
fn matches(
    run: &Run,
    task: &TaskRecord,
    artifact: &Artifact,
    plan: &PlannedCheck,
    invocation: &Invocation,
) -> bool {
    invocation.task_id == task.id
        && invocation.task_attempt == task.attempt
        && invocation.artifact_id.as_ref() == Some(&artifact.id)
        && invocation.artifact_hash == artifact.hash
        && invocation.plan == *plan
        && invocation.spec_revision == run.spec_revision
        && invocation.configuration_hash == config_hash(run)
        && invocation.source_hashes == run.source_hashes()
}
fn authenticate<'a>(run: &'a Run, invocation: &Invocation) -> Result<&'a Evidence> {
    if invocation.restoration.is_some() {
        super::check_recovery::authenticate(run, invocation)?;
    }
    let baseline = invocation.plan.phase == CheckPhase::BeforeExecution;
    let inputs = if baseline {
        invocation
            .input_snapshot
            .as_ref()
            .ok_or_else(|| crate::Error::Invalid("baseline input snapshot missing".into()))?
    } else {
        &run.input_snapshot
    };
    let input_hash = hash(serde_json::to_vec(inputs)?);
    let evidence = invocation
        .evidence_id
        .as_ref()
        .and_then(|id| run.evidence.iter().find(|e| &e.id == id))
        .ok_or_else(|| crate::Error::Invalid("task check output receipt missing".into()))?;
    require(
        invocation.state == CheckState::Completed
            && invocation.before_hash == input_hash
            && invocation.after_hash.as_ref() == Some(&input_hash)
            && invocation.source_hashes == run.source_hashes_for_inputs(inputs)
            && (!baseline
                || (invocation.artifact_id.is_none() && invocation.artifact_hash == input_hash))
            && invocation.spec_revision == run.spec_revision
            && invocation.configuration_hash == config_hash(run)
            && run.approvals.iter().any(|a| {
                a.actor == "local_user"
                    && a.spec_revision == invocation.spec_revision
                    && a.scope_hash == invocation.authorization_scope
                    && matches!(
                        a.action.as_str(),
                        "step_team"
                            | "refresh_task_sources"
                            | "retry_task_verification"
                            | "resume_task_session"
                            | "replace_task_session"
                    )
            })
            && invocation.output_hash.as_ref() == Some(&hash(&evidence.output))
            && evidence.output_hash == hash(&evidence.output)
            && evidence.success == invocation.success
            && evidence.artifact_hash == invocation.artifact_hash
            && evidence.input_hash.as_ref() == Some(&input_hash)
            && evidence.source == runner_source(&invocation.plan)?,
        "task check has no authenticated completed current output/input/approval binding",
    )?;
    let output: serde_json::Value = crate::transport::decode(evidence.output.as_bytes())?;
    require(
        output["argv"] == serde_json::to_value(&invocation.plan.argv)?
            && invocation.success
                == (output["exit_code"] == 0
                    && output["signal"].is_null()
                    && output["truncated"] == false),
        "task check output differs from its actual command or success state",
    )?;
    Ok(evidence)
}
fn runner_source(plan: &PlannedCheck) -> Result<String> {
    Ok(format!(
        "runner:{}{}",
        if plan.phase == CheckPhase::BeforeExecution {
            "baseline:"
        } else {
            ""
        },
        serde_json::to_string(&plan.argv)?
    ))
}
fn expectation_met(run: &Run, invocation: &Invocation) -> Result<bool> {
    let evidence = authenticate(run, invocation)?;
    let output: serde_json::Value = crate::transport::decode(evidence.output.as_bytes())?;
    Ok(output["exit_code"] == invocation.plan.expected_exit_code
        && output["signal"].is_null()
        && output["truncated"] == false)
}
fn baseline_attempt(task: &TaskRecord) -> u64 {
    // Read-only refresh preserves the original reproduction; repair starts a new baseline.
    task.history
        .iter()
        .rev()
        .take_while(|h| h.source_refresh.is_some())
        .last()
        .map(|h| h.attempt)
        .unwrap_or(task.attempt)
}
fn baseline_record<'a>(
    run: &'a Run,
    task: &TaskRecord,
    plan: &PlannedCheck,
) -> Option<&'a Invocation> {
    run.task_checks.iter().find(|i| {
        i.task_id == task.id
            && i.task_attempt == baseline_attempt(task)
            && i.plan == *plan
            && i.spec_revision == run.spec_revision
            && i.configuration_hash == config_hash(run)
    })
}
pub(crate) fn baseline_for_artifact(run: &Run, artifact: &Artifact, evidence: &Evidence) -> bool {
    run.team.as_ref().is_some_and(|team| {
        team.tasks.iter().any(|task| {
            let Some(reference) = task
                .artifact
                .as_ref()
                .filter(|a| a.id == artifact.id)
                .or_else(|| {
                    task.history
                        .iter()
                        .filter_map(|h| h.artifact.as_ref())
                        .find(|a| a.id == artifact.id)
                })
            else {
                return false;
            };
            task.check_plan
                .iter()
                .filter(|p| p.phase == CheckPhase::BeforeExecution)
                .any(|plan| {
                    let Some(record) = baseline_record(run, task, plan) else {
                        return false;
                    };
                    let original = if record.task_attempt == reference.attempt {
                        Some(reference)
                    } else {
                        task.history
                            .iter()
                            .filter_map(|h| h.artifact.as_ref())
                            .find(|a| a.attempt == record.task_attempt)
                    };
                    record.evidence_id.as_ref() == Some(&evidence.id)
                        && authenticate(run, record).is_ok()
                        && original
                            .and_then(|r| effects::binding(run, r).ok())
                            .is_some_and(|b| {
                                Some(&b.input_snapshot) == record.input_snapshot.as_ref()
                                    && b.source_hashes == record.source_hashes
                            })
                })
        })
    })
}
pub(crate) fn baseline_expected(run: &Run, artifact: &Artifact, evidence: &Evidence) -> bool {
    baseline_for_artifact(run, artifact, evidence)
        && run.task_checks.iter().any(|i| {
            i.evidence_id.as_ref() == Some(&evidence.id) && expectation_met(run, i).unwrap_or(false)
        })
}
fn reviewed_record<'a>(
    run: &'a Run,
    task: &TaskRecord,
    artifact: &Artifact,
    plan: &PlannedCheck,
) -> Option<&'a Invocation> {
    if plan.phase == CheckPhase::BeforeExecution {
        baseline_record(run, task, plan)
    } else {
        run.task_checks
            .iter()
            .find(|i| matches(run, task, artifact, plan, i))
    }
}
pub(crate) fn planned_receipt<'a>(
    run: &'a Run,
    task: &TaskRecord,
    artifact: &Artifact,
    check_id: &str,
) -> Result<&'a Evidence> {
    let plan = task
        .check_plan
        .iter()
        .find(|p| p.id == check_id)
        .ok_or_else(|| crate::Error::Invalid("proof references an unapproved task check".into()))?;
    let record = reviewed_record(run, task, artifact, plan).ok_or_else(|| {
        crate::Error::Invalid("proof has no actual planned check invocation".into())
    })?;
    let evidence = authenticate(run, record)?;
    require(
        expectation_met(run, record)?
            && (plan.phase != CheckPhase::BeforeExecution
                || baseline_for_artifact(run, artifact, evidence)),
        "proof invocation has an unexpected outcome or no original execution binding",
    )?;
    Ok(evidence)
}
pub(crate) fn evidence_current(run: &Run, evidence: &Evidence) -> bool {
    run.task_checks.iter().any(|invocation| {
        invocation.evidence_id.as_ref() == Some(&evidence.id)
            && authenticate(run, invocation).is_ok()
    })
}
pub(super) fn retry_safe(run: &Run, task: &TaskRecord, artifact: &Artifact) -> Result<()> {
    for plan in &task.check_plan {
        let invocation = reviewed_record(run, task, artifact, plan);
        if plan.phase == CheckPhase::BeforeExecution {
            let invocation = invocation.ok_or_else(|| {
                crate::Error::Invalid(
                    "missing original baseline cannot be reconstructed by verifier replacement"
                        .into(),
                )
            })?;
            let evidence = authenticate(run, invocation)?;
            require(
                baseline_for_artifact(run, artifact, evidence),
                "retry baseline is not bound to the original executor inputs",
            )?;
        }
        if let Some(invocation) = invocation {
            require(
                expectation_met(run, invocation)?,
                "failed required task check requires repair, not verifier replacement",
            )?;
        }
    }
    Ok(())
}
pub(crate) fn validate_review(
    run: &Run,
    task: &TaskRecord,
    artifact: &Artifact,
    results: &[CheckResult],
    accepted: bool,
) -> Result<()> {
    let role = run
        .team
        .as_ref()
        .unwrap()
        .agents
        .iter()
        .find(|a| a.id == task.owner_id)
        .unwrap()
        .role;
    crate::team::checks::validate_plan(task, true, role, run.spec.task_family)?;
    for plan in &task.check_plan {
        let invocation = reviewed_record(run, task, artifact, plan).ok_or_else(|| {
            crate::Error::Invalid(
                "planned task check was not actually executed on this artifact/source".into(),
            )
        })?;
        let evidence = authenticate(run, invocation)?;
        let result = results
            .iter()
            .find(|r| r.check_id == plan.review_id())
            .ok_or_else(|| {
                crate::Error::Invalid("review omitted a planned runner result".into())
            })?;
        require(
            (plan.phase != CheckPhase::BeforeExecution
                || baseline_for_artifact(run, artifact, evidence))
                && result.evidence_ids.contains(&evidence.id)
                && if expectation_met(run, invocation)? {
                    result.status == CheckStatus::Pass
                } else {
                    !accepted && matches!(result.status, CheckStatus::Fail | CheckStatus::Blocked)
                },
            "review contradicts or omits an actual required task check result",
        )?;
    }
    if accepted {
        for gate_id in crate::team::checks::runner_gates(role, run.spec.task_family) {
            let result = results
                .iter()
                .find(|r| r.check_id == gate_id)
                .ok_or_else(|| crate::Error::Invalid("runner gate result absent".into()))?;
            require(
                task.check_plan
                    .iter()
                    .filter(|p| {
                        p.phase == CheckPhase::BeforeVerification && p.gate_ids.contains(&gate_id)
                    })
                    .any(|p| {
                        run.task_checks
                            .iter()
                            .find(|i| matches(run, task, artifact, p, i))
                            .is_some_and(|i| {
                                i.success
                                    && i.evidence_id
                                        .as_ref()
                                        .is_some_and(|id| result.evidence_ids.contains(id))
                                    && authenticate(run, i).is_ok()
                            })
                    }),
                "runner gate cites no successful check from its approved task plan",
            )?;
        }
    }
    Ok(())
}

pub(crate) struct CheckLease<'a> {
    runtime: &'a Runtime,
}
impl Drop for CheckLease<'_> {
    fn drop(&mut self) {
        *self
            .runtime
            .check_active
            .lock()
            .expect("check admission mutex") = false;
        self.runtime.check_changed.notify_all();
    }
}
impl Runtime {
    pub(crate) fn acquire_check(&self, token: &AtomicBool) -> Result<CheckLease<'_>> {
        let mut active = self.check_active.lock().expect("check admission mutex");
        loop {
            require(
                !token.load(Ordering::SeqCst),
                "cancelled while waiting for shared check resources",
            )?;
            if !*active {
                *active = true;
                return Ok(CheckLease { runtime: self });
            }
            active = self
                .check_changed
                .wait_timeout(active, std::time::Duration::from_millis(100))
                .expect("check admission mutex")
                .0;
        }
    }
    pub(super) fn task_checks(
        &self,
        view: &Run,
        invocation: &TaskInvocation,
        token: Arc<AtomicBool>,
    ) -> Result<Run> {
        self.run_task_checks(
            view,
            &invocation.task,
            invocation.agent.role,
            CheckPhase::BeforeVerification,
            token,
        )
    }
    pub(super) fn task_baselines(
        &self,
        view: &Run,
        task: &TaskRecord,
        role: CatalogRole,
        token: Arc<AtomicBool>,
    ) -> Result<Run> {
        if task
            .history
            .last()
            .is_some_and(|h| h.source_refresh.is_some())
        {
            return self.store.lock().expect("store mutex").get(&view.id);
        }
        self.run_task_checks(view, task, role, CheckPhase::BeforeExecution, token)
    }
    pub(super) fn require_baselines(&self, view: &Run, task: &TaskRecord) -> Result<()> {
        for plan in task
            .check_plan
            .iter()
            .filter(|p| p.phase == CheckPhase::BeforeExecution)
        {
            let record = baseline_record(view, task, plan).ok_or_else(|| {
                crate::Error::Invalid("baseline was not observed before executor admission".into())
            })?;
            require(
                expectation_met(view, record)?,
                "baseline did not observe its approved expected exit code; executor refused",
            )?;
            if !task
                .history
                .last()
                .is_some_and(|h| h.source_refresh.is_some())
            {
                require(
                    record.input_snapshot.as_ref() == Some(&view.input_snapshot),
                    "executor inputs differ from the observed baseline",
                )?;
            } else {
                let original = task
                    .history
                    .iter()
                    .filter_map(|h| h.artifact.as_ref())
                    .find(|a| a.attempt == record.task_attempt)
                    .ok_or_else(|| {
                        crate::Error::Invalid("baseline original artifact missing".into())
                    })?;
                require(
                    Some(&effects::binding(view, original)?.input_snapshot)
                        == record.input_snapshot.as_ref(),
                    "refresh baseline differs from the original execution",
                )?;
            }
        }
        Ok(())
    }
    fn run_task_checks(
        &self,
        view: &Run,
        task: &TaskRecord,
        role: CatalogRole,
        phase: CheckPhase,
        token: Arc<AtomicBool>,
    ) -> Result<Run> {
        crate::team::checks::validate_plan(task, true, role, view.spec.task_family)?;
        crate::team::changes::validate_plan(task, true, role, view.spec.task_family)?;
        let artifact = if phase == CheckPhase::BeforeVerification {
            Some(
                task.artifact
                    .as_ref()
                    .and_then(|r| view.artifacts.iter().find(|a| a.id == r.id))
                    .ok_or_else(|| {
                        crate::Error::Invalid("task check has no current artifact".into())
                    })?,
            )
        } else {
            None
        };
        let artifact_hash = artifact
            .map(|a| a.hash.clone())
            .unwrap_or_else(|| hash(serde_json::to_vec(&view.input_snapshot).unwrap()));
        for plan in task.check_plan.iter().filter(|p| p.phase == phase) {
            self.await_action(view)?;
            let _resources = self.acquire_check(&token)?;
            self.await_action(view)?;
            if let Some(config) = &view.spec.config {
                config
                    .config
                    .policy
                    .authorize_local(crate::config::LocalAction::Check, true)?;
            }
            let check_id;
            {
                let mut store = self.active_store(view)?;
                let mut current = store.get(&view.id)?;
                require(
                    current.source_hashes() == view.source_hashes()
                        && config_hash(&current) == config_hash(view),
                    "task check sources/policy changed",
                )?;
                if let Some(record) = if let Some(artifact) = artifact {
                    current
                        .task_checks
                        .iter()
                        .find(|i| matches(&current, task, artifact, plan, i))
                } else {
                    baseline_record(&current, task, plan)
                } {
                    authenticate(&current, record)?;
                    continue;
                }
                let before = crate::workspace::read_inputs(
                    Path::new(&view.spec.workspace),
                    &view.spec.input_files,
                )?;
                require(
                    before == view.input_snapshot,
                    "workspace changed before planned task check",
                )?;
                let before_hash = hash(serde_json::to_vec(&before)?);
                let authorization = current
                    .approvals
                    .iter()
                    .rev()
                    .find(|a| {
                        a.actor == "local_user"
                            && a.spec_revision == current.spec_revision
                            && matches!(
                                a.action.as_str(),
                                "step_team"
                                    | "refresh_task_sources"
                                    | "retry_task_verification"
                                    | "resume_task_session"
                                    | "replace_task_session"
                            )
                    })
                    .ok_or_else(|| {
                        crate::Error::Invalid(
                            "task check requires scoped team wave approval".into(),
                        )
                    })?;
                check_id = id();
                current.task_checks.push(Invocation {
                    id: check_id.clone(),
                    task_id: task.id.clone(),
                    task_attempt: task.attempt,
                    artifact_id: artifact.map(|a| a.id.clone()),
                    input_snapshot: (phase == CheckPhase::BeforeExecution)
                        .then(|| view.input_snapshot.clone()),
                    artifact_hash: artifact_hash.clone(),
                    plan: plan.clone(),
                    spec_revision: view.spec_revision,
                    configuration_hash: config_hash(view),
                    source_hashes: view.source_hashes(),
                    authorization_scope: authorization.scope_hash.clone(),
                    before_hash,
                    after_hash: None,
                    state: CheckState::Intent,
                    success: false,
                    output_hash: None,
                    evidence_id: None,
                    restoration: None,
                });
                store.save(&current, "task_check_intent", None)?;
            }
            let result =
                crate::workspace::check(Path::new(&view.spec.workspace), &plan.argv, token.clone());
            let after = crate::workspace::read_inputs(
                Path::new(&view.spec.workspace),
                &view.spec.input_files,
            );
            let unchanged = after.as_ref().is_ok_and(|s| s == &view.input_snapshot);
            let mut store = self.store.lock().expect("store mutex");
            let mut current = store.get(&view.id)?;
            require(
                current.spec_revision == view.spec_revision && current.attempt == view.attempt,
                "task check completion is stale",
            )?;
            if let Ok((success, output)) = &result {
                let intent = current
                    .task_checks
                    .iter()
                    .find(|i| i.id == check_id)
                    .unwrap()
                    .clone();
                current
                    .task_check_outcomes
                    .push(super::check_recovery::Outcome {
                        id: id(),
                        invocation_id: check_id.clone(),
                        intent_hash: super::check_recovery::intent_hash(&intent)?,
                        original_intent: intent,
                        output: output.clone(),
                        output_hash: hash(output),
                        process_success: *success,
                        observed_after: after.as_ref().ok().cloned(),
                        baseline_barrier: if plan.phase == CheckPhase::BeforeExecution {
                            Some(super::check_recovery::BaselineBarrier {
                                team: current.team.as_ref().unwrap().clone(),
                                native_ledger_hash: hash(serde_json::to_vec(
                                    &current.session_ledger,
                                )?),
                                execution_ledger_hash: hash(serde_json::to_vec(
                                    &current.task_executions,
                                )?),
                                run_attempt: current.attempt,
                            })
                        } else {
                            None
                        },
                    });
                store.save(&current, "task_check_output_captured", None)?;
            }
            let record = current
                .task_checks
                .iter_mut()
                .find(|i| i.id == check_id)
                .unwrap();
            record.after_hash = after
                .as_ref()
                .ok()
                .map(|s| serde_json::to_vec(s).map(hash))
                .transpose()?;
            if let Ok((success, output)) = &result {
                let evidence_id = id();
                record.output_hash = Some(hash(output));
                record.evidence_id = Some(evidence_id.clone());
                record.success = *success && unchanged;
                record.state = if unchanged {
                    CheckState::Completed
                } else {
                    CheckState::Unknown
                };
                current.evidence.push(Evidence {
                    id: evidence_id,
                    artifact_hash: artifact_hash.clone(),
                    spec_revision: view.spec_revision,
                    source: runner_source(plan)?,
                    output_hash: hash(output),
                    output: output.clone(),
                    success: *success && unchanged,
                    input_hash: Some(hash(serde_json::to_vec(&view.input_snapshot)?)),
                });
            } else {
                record.state = CheckState::Unknown;
            }
            if !unchanged || result.is_err() {
                current.reconciliation_required = true;
            }
            store.save(&current, "task_check_finished", None)?;
            result?;
            require(
                unchanged,
                "planned task check changed or obscured pinned inputs; reconcile before proceeding",
            )?;
        }
        self.store.lock().expect("store mutex").get(&view.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_shared_check_admission_cancels_without_waiting_for_the_active_check() {
        let root = tempfile::tempdir().unwrap();
        let runtime = Arc::new(
            Runtime::new(crate::storage::Store::open(&root.path().join("state.db")).unwrap())
                .unwrap(),
        );
        let active = AtomicBool::new(false);
        let _held = runtime.acquire_check(&active).unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_runtime = runtime.clone();
        let worker_token = cancelled.clone();
        let (sender, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let stopped = worker_runtime.acquire_check(&worker_token).is_err();
            sender.send(stopped).unwrap();
        });
        cancelled.store(true, Ordering::SeqCst);
        runtime.check_changed.notify_all();
        assert!(
            receiver
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
        );
        worker.join().unwrap();
    }
}
