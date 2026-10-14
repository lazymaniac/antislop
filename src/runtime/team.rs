//! Runtime-supervised team wave scheduler and task execution bridge.
pub mod adjudicate;
pub mod check_recovery;
pub mod checks;
pub mod effects;
pub(crate) mod refresh;
pub mod replacement;
pub mod restore;
pub mod resume;
pub(crate) mod retry;
pub mod stage;
mod tasks;
#[cfg(test)]
mod tests;
use super::{ObservedWork, Runtime};
use crate::{
    Result,
    config::{Mode, RoleKind},
    domain::{Artifact, CheckResult, CheckStatus, Evidence, FileEdit, Review, Run, Stage, Status},
    hash, id, providers,
    quality::TaskFamily,
    require,
    team::{
        ArtifactRef, CatalogRole, CompletedTask, GateBinding, GateContext, ReviewDecision,
        SchedulerEvent, TaskInvocation, TaskOperation, TaskOutcome, TaskStage, TaskState,
        WorkerProgress, role_quality_gates,
    },
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Default)]
struct SharedTaskState {
    artifacts: BTreeMap<String, ProducedArtifact>,
    reviews: BTreeMap<String, ProducedReview>,
}

struct ProducedArtifact {
    id: String,
    stage: Stage,
    attempt: u64,
    session_id: String,
    content: String,
    content_hash: String,
    evidence_id: String,
    evidence_output: String,
}

struct ProducedReview {
    decision: ReviewDecision,
    review: Review,
    evidence_id: String,
    evidence_output: String,
}

pub(crate) fn gate_context(run: &Run) -> GateContext {
    let source_hashes = run.source_hashes();
    let current_results = stage::current_results(run).unwrap_or_default();
    let mut gates = BTreeMap::new();
    let mut gate_tasks = BTreeMap::new();
    for (key, assessment) in &run.quality_gates {
        let owner = current_results.values().find(|r| {
            r.artifact_hash == assessment.artifact_hash && r.review_hash == assessment.review_hash
        });
        if !crate::quality::assessment_is_current(run, assessment, &source_hashes)
            || (run.team.is_some() && owner.is_none())
        {
            continue;
        }
        let binding = GateBinding {
            spec_revision: assessment.spec_revision,
            source_hashes: assessment.source_hashes.clone(),
            evidence_refs: assessment.evidence_ids.iter().cloned().collect(),
            passed: assessment.status == CheckStatus::Pass,
        };
        if let Some(result) = owner {
            gate_tasks.insert(key.clone(), result.task_id.clone());
            gate_tasks.insert(assessment.gate_id.clone(), result.task_id.clone());
        }
        gates.insert(key.clone(), binding.clone());
        gates.insert(assessment.gate_id.clone(), binding);
    }
    GateContext {
        spec_revision: run.spec_revision,
        source_hashes,
        gates,
        gate_tasks,
        accepted_results: (run
            .spec
            .config
            .as_ref()
            .is_none_or(|c| c.config.policy.mode == Mode::Supervised))
        .then(|| stage::accepted_ids_for(run, &current_results)),
    }
}

impl Runtime {
    /// Serialize mediated writes and commit intent before touching the workspace.
    /// Unrelated task turns may finish concurrently; no clone may replace these receipts.
    #[cfg(test)]
    fn apply_task_edits(
        &self,
        captured: &Run,
        edits: &[FileEdit],
        token: &AtomicBool,
    ) -> Result<()> {
        if edits.is_empty() {
            return Ok(());
        }
        let mut store = self.active_store(captured)?;
        let mut current = store.get(&captured.id)?;
        require(
            !token.load(Ordering::SeqCst),
            "team step cancelled before write",
        )?;
        require(
            current.input_snapshot == captured.input_snapshot,
            "another task changed these inputs; replan before writing",
        )?;
        let receipts = crate::recovery::prepare(&current, edits)?;
        let offset = current.pending_edits.len();
        current.pending_edits.extend(receipts);
        store.save(&current, "team_edit_intent", None)?;
        for (index, edit) in edits.iter().enumerate() {
            require(
                !token.load(Ordering::SeqCst),
                "team step cancelled before write",
            )?;
            crate::workspace::replace(
                Path::new(&current.spec.workspace),
                &edit.path,
                &current.pending_edits[offset + index].before_hash,
                &edit.content,
            )?;
            current
                .input_snapshot
                .insert(edit.path.clone(), edit.content.clone());
            current.pending_edits[offset + index].state = crate::recovery::EditState::Applied;
            crate::verification::refresh_provenance(&mut current);
            store.save(&current, "team_edit_applied", None)?;
        }
        Ok(())
    }
    pub(super) fn work_team(
        self: Arc<Self>,
        mut run: Run,
        token: Arc<AtomicBool>,
        lease: crate::recovery::WorkspaceLease,
        verification_task: Option<String>,
        refresh_task: Option<String>,
    ) {
        let lease_fds = lease.fds();
        let _lease_scope = crate::process::LeaseScope::new(&lease_fds);
        let mut scheduler_uncertain = false;
        let outcome = (|| -> Result<()> {
            require(!token.load(Ordering::SeqCst), "team step cancelled")?;
            let mut team = run
                .team
                .clone()
                .ok_or_else(|| crate::Error::Invalid("no team installed".into()))?;
            let mut context = gate_context(&run);

            loop {
                if token.load(Ordering::SeqCst) {
                    return Err(crate::Error::Invalid("team execution cancelled".into()));
                }

                let shared = Arc::new(Mutex::new(SharedTaskState::default()));

                // Observe the actual workspace once for this immutable wave.
                self.await_action(&run)?;
                let snapshot = serde_json::to_string(&run.input_snapshot)?;
                run.evidence.push(Evidence {
                    id: id(),
                    artifact_hash: hash(&snapshot),
                    spec_revision: run.spec_revision,
                    source: "runtime:input-snapshot".into(),
                    output_hash: hash(&snapshot),
                    output: snapshot.clone(),
                    success: true,
                    input_hash: Some(hash(&snapshot)),
                });
                {
                    let mut store = self.store.lock().expect("store mutex");
                    let mut current = store.get(&run.id)?;
                    require(
                        current.spec_revision == run.spec_revision
                            && current.attempt == run.attempt,
                        "team wave is stale",
                    )?;
                    current.evidence.push(run.evidence.last().unwrap().clone());
                    store.save(&current, "team_inputs_observed", None)?;
                }
                let view = run.clone();

                let runner =
                    |invocation: &TaskInvocation,
                     progress: &dyn Fn(WorkerProgress) -> Result<()>| {
                        // Each scheduler worker is a new thread. Guardians must inherit
                        // the leases here as well as on the baseline coordinator thread.
                        let _worker_lease_scope = crate::process::LeaseScope::new(&lease_fds);
                        let mut view = view.clone();
                        view.stage = match invocation.task.stage {
                            TaskStage::Discovery => Stage::Discovery,
                            TaskStage::Planning => Stage::Planning,
                            TaskStage::Execution => Stage::Execution,
                            TaskStage::Validation => Stage::Validation,
                            TaskStage::Integration => Stage::Integration,
                            TaskStage::Delivery => Stage::Delivery,
                        };
                        let observed = self.store.lock().expect("store mutex").get(&view.id)?;
                        view.task_checks = observed.task_checks;
                        view.evidence = observed.evidence;
                        match invocation.operation {
                            TaskOperation::Execute => self.execute_team_task(
                                &view,
                                invocation,
                                progress,
                                token.clone(),
                                &shared,
                            ),
                            TaskOperation::Verify => self.verify_team_task(
                                &view,
                                invocation,
                                progress,
                                token.clone(),
                                &shared,
                            ),
                        }
                    };
                let checkpoint = |current_team: &crate::team::TeamPlan, event: &SchedulerEvent| {
                    let result = self
                        .checkpoint_team(&run, current_team, event, &shared)
                        .and_then(|()| {
                            if let SchedulerEvent::WaveStarted { task_ids } = event {
                                // All baselines finish before the scheduler spawns any native worker.
                                for task in current_team.tasks.iter().filter(|t| {
                                    task_ids.contains(&t.id) && t.state == TaskState::Running
                                }) {
                                    let role = current_team
                                        .agents
                                        .iter()
                                        .find(|a| a.id == task.owner_id)
                                        .unwrap()
                                        .role;
                                    self.task_baselines(&run, task, role, token.clone())?;
                                }
                            }
                            Ok(())
                        });
                    if result.is_err() {
                        token.store(true, Ordering::SeqCst);
                        self.work_changed.notify_all();
                    }
                    result
                };
                let wave_result = if let Some(task_id) = &verification_task {
                    team.execute_verification(&run.spec, &context, task_id, runner, checkpoint)
                } else if let Some(task_id) = &refresh_task {
                    team.execute_selected(&run.spec, &context, task_id, runner, checkpoint)
                } else {
                    team.execute_ready(&run.spec, &context, runner, checkpoint)
                };

                scheduler_uncertain |= team.checkpoint_uncertain;
                let completed: Vec<CompletedTask> = wave_result?;

                // Read authoritative task/artifact/edit checkpoints; never adopt unobserved external edits.
                run = self.store.lock().expect("store mutex").get(&run.id)?;
                require(
                    crate::workspace::read_inputs(
                        Path::new(&run.spec.workspace),
                        &run.spec.input_files,
                    )? == run.input_snapshot,
                    "workspace changed during task wave; reconcile before continuing",
                )?;
                context = gate_context(&run);
                let stale = team.invalidate_sources(&context);
                if !stale.is_empty() {
                    let mut store = self.store.lock().expect("store mutex");
                    let mut current = store.get(&run.id)?;
                    require(
                        current.status != Status::Cancelled,
                        "cancelled before source invalidation",
                    )?;
                    current.team = Some(team.clone());
                    store.save(&current, "task_sources_invalidated", None)?;
                    run = current;
                }
                let autonomous = run
                    .spec
                    .config
                    .as_ref()
                    .is_some_and(|c| c.config.policy.mode == Mode::Autonomous);

                // Check termination conditions
                if team.tasks.iter().all(|t| t.state == TaskState::Verified) {
                    stage::require_final_gates_in(run.spec.task_family, &context)?;
                    run.status = if autonomous
                        || context.accepted_results.as_ref().is_some_and(|accepted| {
                            team.tasks.iter().all(|t| accepted.contains(&t.id))
                        }) {
                        Status::Completed
                    } else {
                        Status::AwaitingResult
                    };
                    run.reconciliation_required = false;
                    break;
                }
                if let Some(failed) = team
                    .tasks
                    .iter()
                    .find(|t| t.state == TaskState::ChangesRequested)
                {
                    run.status = Status::ChangesRequested;
                    run.diagnostic = Some(format!(
                        "Task {} requested changes: {:?}",
                        failed.id,
                        failed.review.as_ref().map(|r| &r.finding_ids)
                    ));
                    run.reconciliation_required = false;
                    break;
                }
                if let Some(blocked) = team.tasks.iter().find(|t| t.state == TaskState::Blocked) {
                    run.status = Status::Blocked;
                    run.diagnostic = blocked.diagnostic.clone();
                    run.reconciliation_required = true;
                    break;
                }

                if team.tasks.iter().any(|t| t.state == TaskState::Stale) {
                    run.status = Status::Blocked;
                    run.diagnostic = Some("Managed changes invalidated task sources/dependencies; authorize read-only source refresh, then independent review".into());
                    run.reconciliation_required = false;
                    break;
                }
                if completed.is_empty() {
                    // No ready tasks could be scheduled
                    run.reconciliation_required = false;
                    run.status = if !stage::pending(&run).is_empty() {
                        Status::AwaitingResult
                    } else {
                        Status::Blocked
                    };
                    run.diagnostic = Some(if run.status == Status::AwaitingResult {
                        "Verified stage results await explicit user acceptance".into()
                    } else {
                        "No scheduler tasks are ready in the current wave".into()
                    });
                    run.reconciliation_required = false;
                    break;
                }

                if !autonomous || verification_task.is_some() || refresh_task.is_some() {
                    // In supervised mode, each step runs one wave
                    run.reconciliation_required = false;
                    run.status = if stage::pending(&run).is_empty() {
                        Status::Ready
                    } else {
                        Status::AwaitingResult
                    };
                    break;
                }
            }

            Ok(())
        })();

        if let Err(error) = outcome {
            run.status = Status::Blocked;
            run.diagnostic = Some(error.to_string());
            run.reconciliation_required = true;
        }

        let mut store = self.store.lock().expect("store mutex");
        if let Ok(mut current) = store.get(&run.id)
            && current.spec_revision == run.spec_revision
            && current.attempt == run.attempt
            && current.status != Status::Cancelled
        {
            if current.status == Status::Paused {
                current.paused_from = Some(run.status);
            } else {
                current.status = run.status;
            }
            current.reconciliation_required = run.reconciliation_required;
            if run.reconciliation_required
                && let Some(team) = &mut current.team
            {
                team.checkpoint_uncertain |= scheduler_uncertain;
                team.recover_interrupted();
            }
            current.diagnostic = run.diagnostic;
            if let Err(error) = store.save(&current, "team_step_finished", None) {
                eprintln!("Cannot persist team step completion: {error}");
            }
        }
        // Dispatch shares the store lock: publish idle only after ownership is released.
        drop(_lease_scope);
        drop(lease);
        self.workers.lock().expect("workers mutex").remove(&run.id);
    }
}

fn queued_message_ids(run: &Run, agent: &str) -> Vec<String> {
    use crate::communication::DeliveryState;
    run.communication_ledger
        .messages
        .values()
        .filter(|m| {
            m.recipient_agent_id == agent
                && matches!(
                    m.state,
                    DeliveryState::AcceptedRuntime | DeliveryState::QueuedNextTurn
                )
        })
        .map(|m| m.id.clone())
        .collect()
}

fn task_checks(run: &Run, invocation: &TaskInvocation) -> BTreeSet<String> {
    use CatalogRole::*;
    let role = match invocation.agent.role {
        Orchestrator => "orchestration",
        ContextAnalyst => "context",
        RequirementsAnalyst => "requirements",
        Planner => "plan",
        OptionsAnalyst => "options",
        Architect => "architecture",
        DependencyAnalyst => "dependency",
        Implementer => "code",
        Refactorer => "refactor",
        StaticAnalysisOperator => "static",
        ComplianceAuditor => "compliance",
        TestDesigner => "test_design",
        TestImplementer => "test",
        SecurityAnalyst => "security",
        PerformanceEngineer => "performance",
        Integrator => "integration",
        Researcher => "research",
        DataAnalyst => "analysis",
        Writer => "editorial",
        DeliveryAuthor => "delivery",
    };
    let mut refresh_checks = BTreeSet::new();
    if let Some(previous) = invocation
        .task
        .history
        .last()
        .filter(|h| h.source_refresh.is_some())
        .and_then(|h| h.artifact.as_ref())
    {
        refresh_checks.insert(format!("SOURCE_REFRESH:{}", previous.id));
    }
    let mut checks: BTreeSet<_> = (1..=8)
        .map(|n| format!("V{n:02}"))
        .chain(refresh_checks)
        .collect();
    checks.extend(invocation.task.check_plan.iter().map(|p| p.review_id()));
    if let Some(contract) = &invocation.task.change_contract {
        checks.insert("CHANGE:classification".into());
        checks.extend(contract.bugfixes.iter().map(|b| format!("BUGFIX:{}", b.id)));
    }
    checks.extend(invocation.task.criterion_ids.iter().cloned());
    checks.extend(
        invocation
            .task
            .plan_item_ids
            .iter()
            .map(|id| format!("PLAN:{id}")),
    );
    checks.extend(
        crate::quality::catalog()
            .into_iter()
            .filter(|c| c.role == role && c.family.is_none_or(|f| f == run.spec.task_family))
            .map(|c| c.check_id),
    );
    checks.extend(
        run.team_risk_owners
            .iter()
            .filter(|r| r.task_id == invocation.task.id)
            .map(|r| format!("RISK:{}", r.risk_id)),
    );
    checks.extend(
        role_quality_gates(invocation.agent.role, run.spec.task_family)
            .into_iter()
            .map(|(id, phase)| crate::quality::gate_check_id(id, phase)),
    );
    checks
}

fn validate_task_checks(
    run: &Run,
    artifact: &Artifact,
    expected: &BTreeSet<String>,
    checks: &[CheckResult],
    accepted: bool,
) -> Result<()> {
    let mut seen = BTreeSet::new();
    for check in checks {
        require(
            expected.contains(&check.check_id)
                && seen.insert(check.check_id.clone())
                && !check.method.trim().is_empty()
                && !check.rationale.trim().is_empty()
                && !check.evidence_ids.is_empty(),
            "incomplete, invented or duplicate task check",
        )?;
        for id in &check.evidence_ids {
            require(
                run.evidence.iter().any(|e| {
                    e.id == *id
                        && (e.success
                            || checks::baseline_expected(run, artifact, e)
                            || (!accepted && checks::evidence_current(run, e)))
                        && e.output_hash == hash(&e.output)
                        && crate::verification::evidence_is_current(run, artifact, e)
                }),
                "task check cites missing, stale, corrupted or failed evidence",
            )?;
        }
        if accepted {
            require(
                check.check_id.starts_with("BASELINE:")
                    || check.evidence_ids.iter().any(|id| {
                        run.evidence.iter().any(|e| {
                            e.id == *id
                                && !e.source.starts_with("runner:baseline:")
                                && e.success
                                && crate::verification::evidence_is_current(run, artifact, e)
                        })
                    }),
                "historical baseline alone cannot establish a current positive task check",
            )?;
            require(
                check.status == CheckStatus::Pass
                    || (check.status == CheckStatus::NotApplicable
                        && (check.check_id.starts_with("RISK:")
                            || crate::quality::allows_not_applicable(&check.check_id))),
                "accepted task review contains a failing, blocked or prohibited N/A check",
            )?;
        }
    }
    require(
        seen == *expected,
        "task review checklist coverage incomplete",
    )
}

fn authenticate_pair(
    run: &Run,
    artifact: &ArtifactRef,
    verifier: &str,
    output: &str,
) -> Result<()> {
    use crate::sessions::TurnState;
    run.session_ledger.validate()?;
    let task = run
        .team
        .as_ref()
        .and_then(|t| t.tasks.iter().find(|t| t.id == artifact.task_id))
        .ok_or_else(|| crate::Error::Invalid("task authorship has no installed task".into()))?;
    let execution = effects::binding(run, artifact)?;
    effects::lineage(run, task)?;
    let mut native = BTreeSet::new();
    let owner_kind = run
        .team
        .as_ref()
        .unwrap()
        .agents
        .iter()
        .find(|a| a.id == task.owner_id)
        .unwrap()
        .kind;
    let owner_role = run
        .team
        .as_ref()
        .unwrap()
        .agents
        .iter()
        .find(|a| a.id == task.owner_id)
        .unwrap()
        .role;
    let mut scoped = run.clone();
    scoped.stage = match task.stage {
        TaskStage::Discovery => Stage::Discovery,
        TaskStage::Planning => Stage::Planning,
        TaskStage::Execution => Stage::Execution,
        TaskStage::Validation => Stage::Validation,
        TaskStage::Integration => Stage::Integration,
        TaskStage::Delivery => Stage::Delivery,
    };
    for (session_id, agent, role) in [
        (artifact.session_id.as_str(), &task.owner_id, owner_kind),
        (verifier, &task.verifier_id, RoleKind::Verifier),
    ] {
        let session = run
            .session_ledger
            .sessions
            .get(session_id)
            .ok_or_else(|| crate::Error::Invalid("task authorship session missing".into()))?;
        let turn = session
            .current_turn_id
            .as_ref()
            .and_then(|id| run.session_ledger.turns.get(id))
            .ok_or_else(|| crate::Error::Invalid("task authorship turn missing".into()))?;
        let raw = turn
            .output
            .as_ref()
            .ok_or_else(|| crate::Error::Invalid("task authorship output missing".into()))?;
        let configured = run
            .spec
            .config
            .as_ref()
            .ok_or_else(|| crate::Error::Invalid("task configuration missing".into()))?
            .config
            .resolve(owner_role.work_kind(), role)?
            .provider;
        let mut author_scope = scoped.clone();
        if role != RoleKind::Verifier {
            author_scope.input_snapshot = execution.input_snapshot.clone();
        }
        require(
            session.scope
                == crate::sessions::SessionScope::for_run(
                    &author_scope,
                    agent.clone(),
                    Some(task.id.clone()),
                    role,
                    session.scope.provider.clone(),
                    session.scope.binary_version.clone(),
                )?
                && serde_json::to_value(configured)?.as_str()
                    == Some(session.scope.provider.as_str())
                && session.scope.agent_id == *agent
                && session.scope.role == role
                && session.scope.task_id.as_ref() == Some(&task.id)
                && session.scope.spec_revision == run.spec_revision
                && session.scope.input_hash
                    == hash(serde_json::to_vec(&author_scope.input_snapshot)?)
                && session.native_observed
                && turn.state == TurnState::Completed
                && turn.provider_completed
                && turn.output_ref.as_ref().is_some_and(|r| {
                    r.document == format!("turn:{}", turn.id) && r.hash == hash(raw)
                })
                && if role == RoleKind::Verifier {
                    raw == output
                } else {
                    hash(raw) == artifact.content_hash
                },
            "task output is not bound to independent completed current authorship",
        )?;
        let raw_native = session
            .native_session_id
            .as_ref()
            .ok_or_else(|| crate::Error::Invalid("task native identity missing".into()))?;
        let canonical = uuid::Uuid::parse_str(raw_native)
            .map(|u| u.to_string())
            .unwrap_or(raw_native.clone());
        require(
            native.insert((session.scope.provider.clone(), canonical)),
            "task review reused the executor native session",
        )?;
    }
    Ok(())
}

pub(super) fn authenticated_result<'a>(
    run: &'a Run,
    task: &crate::team::TaskRecord,
) -> Result<(&'a Artifact, &'a Review)> {
    require(
        task.state == TaskState::Verified && task.review.as_ref().is_some_and(|r| r.accepted),
        "stage result has no positive independent task review",
    )?;
    let team = run
        .team
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("no installed result owner".into()))?;
    let reference = task
        .artifact
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("stage result artifact missing".into()))?;
    require(
        reference.source_hashes == run.source_hashes()
            && reference.dependency_hashes == team.dependency_bindings(task)?,
        "stage result sources/dependencies changed",
    )?;
    let review = run
        .reviews
        .iter()
        .find(|r| {
            r.verifier_session == task.review.as_ref().unwrap().session_id
                && r.artifact_hash == reference.content_hash
        })
        .ok_or_else(|| crate::Error::Invalid("stage result review missing".into()))?;
    let decision = task.review.as_ref().unwrap();
    require(
        decision.verifier_id == task.verifier_id
            && decision.spec_revision == run.spec_revision
            && decision.artifact_hash == reference.content_hash
            && decision.finding_ids.is_empty()
            && review.spec_revision == run.spec_revision
            && review.checklist_version == run.checklist_version
            && review.executor_session == reference.session_id
            && review.findings.is_empty(),
        "stored stage result authorship/findings contradict its native report",
    )?;
    let artifact = reference;
    let raw = run
        .session_ledger
        .sessions
        .get(&review.verifier_session)
        .and_then(|s| s.current_turn_id.as_ref())
        .and_then(|id| run.session_ledger.turns.get(id))
        .and_then(|t| t.output.as_ref())
        .ok_or_else(|| crate::Error::Invalid("gate native report missing".into()))?;
    authenticate_pair(run, artifact, &review.verifier_session, raw)?;
    let report: serde_json::Value = crate::transport::decode(raw.as_bytes())?;
    let native_quality: crate::quality::QualityArtifacts =
        serde_json::from_value(report["quality"].clone())?;
    let criteria = report["criteria"]
        .as_object()
        .ok_or_else(|| crate::Error::Invalid("gate criteria missing".into()))?;
    require(
        report["accepted"] == true
            && report["findings"].as_array().is_some_and(|f| f.is_empty())
            && criteria.keys().cloned().collect::<BTreeSet<_>>() == task.criterion_ids
            && criteria.values().all(|v| v.as_bool() == Some(true))
            && report["checks"] == serde_json::to_value(&review.checks)?
            && serde_json::to_value(native_quality)? == serde_json::to_value(&review.quality)?
            && report["output_hash"] == artifact.content_hash
            && report["checklist_version"] == run.checklist_version
            && report["input_hash"] == hash(serde_json::to_vec(&run.input_snapshot)?),
        "gate does not match the completed task report",
    )?;
    let pinned = run
        .artifacts
        .iter()
        .find(|a| a.id == artifact.id)
        .ok_or_else(|| crate::Error::Invalid("gate pinned task artifact missing".into()))?;
    require(
        pinned.hash == reference.content_hash
            && hash(&pinned.content) == pinned.hash
            && pinned.spec_revision == reference.spec_revision
            && pinned.session_id == reference.session_id
            && pinned.attempt == reference.attempt,
        "stage result pinned artifact identity changed",
    )?;
    let execution = effects::binding(run, reference)?;
    let original_input = hash(serde_json::to_vec(&execution.input_snapshot)?);
    let current_input = hash(serde_json::to_vec(&run.input_snapshot)?);
    for (refs, source, output, input_hash) in [
        (
            &reference.evidence_refs,
            "runtime:artifact-read".to_owned(),
            pinned.content.as_str(),
            &original_input,
        ),
        (
            &decision.evidence_refs,
            format!("task:{}:verifier", task.id),
            raw.as_str(),
            &current_input,
        ),
    ] {
        require(
            refs.len() == 1
                && refs.iter().all(|id| {
                    run.evidence.iter().any(|e| {
                        e.id == *id
                            && e.source == source
                            && e.success
                            && e.spec_revision == run.spec_revision
                            && e.artifact_hash == pinned.hash
                            && e.output == output
                            && e.output_hash == hash(output)
                            && e.input_hash.as_ref() == Some(input_hash)
                    })
                }),
            "stage result native receipt missing or corrupted",
        )?;
    }
    let agent = team
        .agents
        .iter()
        .find(|a| a.id == task.verifier_id)
        .ok_or_else(|| crate::Error::Invalid("result verifier owner missing".into()))?;
    let invocation = TaskInvocation {
        task: task.clone(),
        agent: agent.clone(),
        operation: TaskOperation::Verify,
        dependency_hashes: reference.dependency_hashes.clone(),
        cancelled: Arc::new(AtomicBool::new(false)),
    };
    validate_task_checks(
        run,
        pinned,
        &task_checks(run, &invocation),
        &review.checks,
        true,
    )?;
    require(
        task.review.as_ref().unwrap().criteria
            == task
                .criterion_ids
                .iter()
                .map(|id| (id.clone(), true))
                .collect(),
        "stored task criteria contradict native result",
    )?;
    checks::validate_review(run, task, pinned, &review.checks, true)?;
    crate::quality::validate_task_review(run, pinned, review)?;
    Ok((pinned, review))
}

fn assigned_risks(run: &Run, task_id: &str) -> Result<serde_json::Value> {
    let ids: BTreeSet<_> = run
        .team_risk_owners
        .iter()
        .filter(|r| r.task_id == task_id)
        .map(|r| r.risk_id.as_str())
        .collect();
    let catalog: serde_json::Value =
        crate::transport::decode(include_bytes!("../../fixtures/risks.json"))?;
    let risks = catalog["risks"]
        .as_array()
        .ok_or_else(|| crate::Error::Invalid("risk catalog malformed".into()))?;
    Ok(serde_json::json!(
        risks
            .iter()
            .filter(|risk| risk["id"].as_str().is_some_and(|id| ids.contains(id)))
            .collect::<Vec<_>>()
    ))
}
