use architect::{
    Error,
    config::{Config, RoleKind, WorkKind},
    domain::RunSpec,
    hash,
    quality::TaskFamily,
    team::*,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Barrier, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

fn spec(count: usize) -> RunSpec {
    serde_json::from_value(serde_json::json!({
        "task_family":"research", "objective":"Investigate every listed requirement", "workspace":".",
        "provider":"fake", "model":null,
        "requirements":(0..count).map(|i|serde_json::json!({"id":format!("R{i}"),"source_ref":format!("user:{i}"),"original_text":format!("Required claim {i}"),"criteria":{"verified":"Support with actual sources"}})).collect::<Vec<_>>(),
        "plan_items":(0..count).map(|i|serde_json::json!({"id":format!("P{i}"),"dependencies":[],"requirement_ids":[format!("R{i}")],"description":format!("Investigate claim {i}")})).collect::<Vec<_>>()
    })).unwrap()
}
fn context(team: &TeamPlan) -> GateContext {
    let sources = team
        .tasks
        .iter()
        .flat_map(|task| &task.source_dependencies)
        .map(|source| (source.clone(), hash(source)))
        .collect::<BTreeMap<_, _>>();
    GateContext {
        accepted_results: None,
        gate_tasks: BTreeMap::new(),
        spec_revision: team.spec_revision,
        source_hashes: sources.clone(),
        gates: team
            .tasks
            .iter()
            .flat_map(|t| t.required_gates.iter())
            .map(|id| {
                (
                    id.clone(),
                    GateBinding {
                        spec_revision: team.spec_revision,
                        source_hashes: sources.clone(),
                        evidence_refs: BTreeSet::from(["actual-gate-evidence".into()]),
                        passed: true,
                    },
                )
            })
            .collect(),
    }
}
fn session(invocation: &TaskInvocation) -> String {
    format!(
        "session-{}-{}-{:?}",
        invocation.agent.id, invocation.task.attempt, invocation.operation
    )
}
fn outcome(invocation: &TaskInvocation, context: &GateContext) -> TaskOutcome {
    match invocation.operation {
        TaskOperation::Execute => TaskOutcome::Executed(ArtifactRef {
            execution_id: None,
            dependency_hashes: invocation.dependency_hashes.clone(),
            id: format!(
                "artifact-{}-{}",
                invocation.task.id, invocation.task.attempt
            ),
            task_id: invocation.task.id.clone(),
            created_by: invocation.agent.id.clone(),
            session_id: session(invocation),
            attempt: invocation.task.attempt,
            spec_revision: context.spec_revision,
            content_hash: hash(format!(
                "{}:{}",
                invocation.task.id, invocation.task.attempt
            )),
            source_hashes: context.source_hashes.clone(),
            evidence_refs: BTreeSet::from(["runner-evidence".into()]),
        }),
        TaskOperation::Verify => TaskOutcome::Reviewed(ReviewDecision {
            verifier_id: invocation.agent.id.clone(),
            session_id: session(invocation),
            artifact_hash: invocation
                .task
                .artifact
                .as_ref()
                .unwrap()
                .content_hash
                .clone(),
            spec_revision: context.spec_revision,
            criteria: invocation
                .task
                .criterion_ids
                .iter()
                .map(|id| (id.clone(), true))
                .collect(),
            evidence_refs: BTreeSet::from(["independent-review-evidence".into()]),
            finding_ids: BTreeSet::new(),
            accepted: true,
        }),
    }
}
fn run_wave(team: &mut TeamPlan, spec: &RunSpec, context: &GateContext) -> Vec<CompletedTask> {
    team.execute_ready(
        spec,
        context,
        |invocation, progress| {
            progress(WorkerProgress::SessionStarted {
                session_id: session(invocation),
            })?;
            Ok(outcome(invocation, context))
        },
        |_, _| Ok(()),
    )
    .unwrap()
}

#[test]
fn catalog_templates_and_bootstrap_preserve_required_pairs() {
    assert_eq!(CatalogRole::ALL.len(), 20);
    assert_eq!(TemplateKind::ALL.len(), 8);
    for template in TemplateKind::ALL {
        let mut source = spec(2);
        source.task_family = template.family();
        let team = TeamPlan::bootstrap_template(template, &source).unwrap();
        team.validate(&source).unwrap();
        assert_eq!(
            team.agents
                .iter()
                .filter(|a| a.kind == RoleKind::Verifier)
                .count()
                * 2,
            team.agents.len()
        );
        if source.task_family == TaskFamily::Software {
            for role in [
                CatalogRole::Architect,
                CatalogRole::DependencyAnalyst,
                CatalogRole::Implementer,
                CatalogRole::Refactorer,
                CatalogRole::StaticAnalysisOperator,
                CatalogRole::TestDesigner,
                CatalogRole::TestImplementer,
                CatalogRole::Integrator,
            ] {
                assert!(
                    team.agents
                        .iter()
                        .any(|a| a.role == role && a.kind != RoleKind::Verifier)
                );
            }
        }
    }
}
#[test]
fn full_original_coverage_is_conjunctive_100_tasks_cannot_drop_one() {
    let source = spec(100);
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let index = team
        .tasks
        .iter()
        .position(|t| t.plan_item_ids.contains("P50"))
        .unwrap();
    let missing = team.tasks.remove(index);
    team.agents
        .retain(|agent| agent.id != missing.owner_id && agent.id != missing.verifier_id);
    for task in &mut team.tasks {
        task.dependencies.remove(&missing.id);
    }
    let error = team.validate(&source).unwrap_err().to_string();
    assert!(error.contains("coverage incomplete"), "{error}");
}
#[test]
fn dedicated_verifier_cannot_be_shared_or_use_executor_session() {
    let source = spec(1);
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let first = team.tasks[0].clone();
    let second = team.tasks[1].clone();
    team.agents
        .iter_mut()
        .find(|a| a.id == second.owner_id)
        .unwrap()
        .verifier_id = Some(first.verifier_id.clone());
    team.tasks[1].verifier_id = first.verifier_id.clone();
    assert!(team.validate(&source).is_err());
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    team.agents[0].session_id = Some("shared".into());
    team.agents[1].session_id = Some("shared".into());
    assert!(team.validate(&source).is_err());
}
#[test]
fn task_and_delegation_cycles_reject_but_communication_cycles_are_valid() {
    let source = spec(1);
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let a = team.tasks[0].id.clone();
    let b = team.tasks[1].id.clone();
    team.tasks[0].dependencies.insert(b);
    team.tasks[1].dependencies.insert(a);
    assert!(team.validate(&source).is_err());
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let a = team.agents[0].id.clone();
    let b = team.agents[2].id.clone();
    team.agents[0].supervisor_id = Some(b.clone());
    team.agents[2].supervisor_id = Some(a.clone());
    assert!(team.validate(&source).is_err());
    team.agents[0].supervisor_id = None;
    team.agents[2].supervisor_id = Some(a.clone());
    team.communicates_with.insert((a.clone(), b.clone()));
    team.communicates_with.insert((b, a));
    team.validate(&source).unwrap();
}
#[test]
fn original_user_plan_dependencies_cannot_be_removed() {
    let mut source = spec(2);
    source.plan_items[1].dependencies.insert("P0".into());
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let before = team
        .tasks
        .iter()
        .find(|t| t.plan_item_ids.contains("P0"))
        .unwrap()
        .id
        .clone();
    team.tasks
        .iter_mut()
        .find(|t| t.plan_item_ids.contains("P1"))
        .unwrap()
        .dependencies
        .remove(&before);
    assert!(
        team.validate(&source)
            .unwrap_err()
            .to_string()
            .contains("silently weakened")
    );
}
#[test]
fn exact_config_routes_include_orchestration_verifier_and_all_team_roles() {
    let source = spec(1);
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let configured = Config::from_toml(include_str!("../examples/architect.toml")).unwrap();
    team.validate_all(&source, &configured).unwrap();
    let verifier = team
        .agents
        .iter()
        .find(|a| a.role == CatalogRole::Orchestrator && a.kind == RoleKind::Verifier)
        .unwrap();
    assert_eq!(
        verifier.profile.as_ref().unwrap().work_kind,
        WorkKind::Orchestration
    );
    assert!(
        verifier
            .profile
            .as_ref()
            .unwrap()
            .requires_independent_session
    );
}
#[test]
fn source_change_blocks_old_team_even_if_ids_stay_the_same() {
    let source = spec(1);
    let team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let mut changed = source.clone();
    changed.requirements[0]
        .original_text
        .push_str(" and preserve an exception");
    assert!(
        team.validate(&changed)
            .unwrap_err()
            .to_string()
            .contains("specification changed")
    );
}
#[test]
fn disjoint_ready_tasks_actually_run_concurrently() {
    let source = spec(1);
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let context = context(&team);
    let size = team.ready_wave(&source, &context).unwrap().len();
    assert!(size >= 3);
    let barrier = Barrier::new(size);
    let active = AtomicUsize::new(0);
    let peak = AtomicUsize::new(0);
    team.execute_ready(
        &source,
        &context,
        |invocation, progress| {
            progress(WorkerProgress::SessionStarted {
                session_id: session(invocation),
            })?;
            let count = active.fetch_add(1, Ordering::SeqCst) + 1;
            peak.fetch_max(count, Ordering::SeqCst);
            barrier.wait();
            active.fetch_sub(1, Ordering::SeqCst);
            Ok(outcome(invocation, &context))
        },
        |_, _| Ok(()),
    )
    .unwrap();
    assert_eq!(peak.load(Ordering::SeqCst), size);
}
#[test]
fn overlapping_parent_paths_are_serialized_and_disjoint_paths_share_wave() {
    let source = spec(1);
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let ready = team.ready_wave(&source, &context(&team)).unwrap();
    assert_eq!(ready.len(), 3);
    for (invocation, path) in ready
        .iter()
        .zip(["src", "src/nested/file.rs", "docs/report.md"])
    {
        let task = team
            .tasks
            .iter_mut()
            .find(|t| t.id == invocation.task.id)
            .unwrap();
        task.write_scopes.insert(path.into());
        task.integration = IntegrationStrategy::InPlace;
        task.required_gates.extend([
            "context_ready".into(),
            "solution_selected".into(),
            "plan_ready".into(),
        ]);
    }
    let context = context(&team);
    let wave = team.ready_wave(&source, &context).unwrap();
    assert_eq!(wave.len(), 2);
    let barrier = Barrier::new(2);
    run_wave_with_barrier(&mut team, &source, &context, &barrier);
    let second = team.ready_wave(&source, &context).unwrap();
    assert!(second.iter().any(|i| i.operation == TaskOperation::Execute
        && i.task.write_scopes.contains("src/nested/file.rs")));
    assert!(
        !scopes_overlap(
            &BTreeSet::from(["src/a".into()]),
            &BTreeSet::from(["src/ab".into()])
        )
        .unwrap()
    );
    assert!(
        scopes_overlap(
            &BTreeSet::from([".".into()]),
            &BTreeSet::from(["any/file".into()])
        )
        .unwrap()
    );
    assert!(scopes_overlap(&BTreeSet::from(["../outside".into()]), &BTreeSet::new()).is_err());
}
fn run_wave_with_barrier(
    team: &mut TeamPlan,
    source: &RunSpec,
    context: &GateContext,
    barrier: &Barrier,
) {
    team.execute_ready(
        source,
        context,
        |invocation, progress| {
            progress(WorkerProgress::SessionStarted {
                session_id: session(invocation),
            })?;
            barrier.wait();
            Ok(outcome(invocation, context))
        },
        |_, _| Ok(()),
    )
    .unwrap();
}
#[test]
fn artifacts_must_be_pinned_before_verification_and_every_criterion_reviewed() {
    let source = spec(1);
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let context = context(&team);
    assert!(
        team.ready_wave(&source, &context)
            .unwrap()
            .iter()
            .all(|i| i.operation == TaskOperation::Execute)
    );
    run_wave(&mut team, &source, &context);
    let ready = team.ready_wave(&source, &context).unwrap();
    assert!(
        ready
            .iter()
            .all(|i| i.operation == TaskOperation::Verify && i.task.artifact.is_some())
    );
    team.execute_ready(
        &source,
        &context,
        |invocation, progress| {
            progress(WorkerProgress::SessionStarted {
                session_id: session(invocation),
            })?;
            let mut result = outcome(invocation, &context);
            if let TaskOutcome::Reviewed(review) = &mut result {
                review.criteria.clear();
            }
            Ok(result)
        },
        |_, _| Ok(()),
    )
    .unwrap();
    assert!(
        team.tasks
            .iter()
            .filter(|t| t.stage == TaskStage::Discovery)
            .all(|t| t.state == TaskState::Blocked)
    );
}
#[test]
fn durable_session_receipt_precedes_effect_and_checkpoint_failure_prevents_start() {
    let source = spec(1);
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let context = context(&team);
    let receipts = Arc::new(Mutex::new(BTreeSet::new()));
    let persisted = receipts.clone();
    team.execute_ready(
        &source,
        &context,
        |invocation, progress| {
            progress(WorkerProgress::SessionStarted {
                session_id: session(invocation),
            })?;
            assert!(receipts.lock().unwrap().contains(&invocation.agent.id));
            Ok(outcome(invocation, &context))
        },
        |snapshot, event| {
            if let SchedulerEvent::SessionRecorded { agent_id, .. } = event {
                assert!(
                    snapshot
                        .agents
                        .iter()
                        .find(|a| &a.id == agent_id)
                        .unwrap()
                        .session_id
                        .is_some()
                );
                persisted.lock().unwrap().insert(agent_id.clone());
            }
            Ok(())
        },
    )
    .unwrap();
    let mut fresh = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let calls = AtomicUsize::new(0);
    assert!(
        fresh
            .execute_ready(
                &source,
                &context,
                |i, _| {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(outcome(i, &context))
                },
                |_, _| Err(Error::Invalid("disk failure".into()))
            )
            .is_err()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(
        fresh
            .tasks
            .iter()
            .any(|t| t.state == TaskState::Running && t.attempt == 1)
    );
}
#[test]
fn checkpoint_restore_never_replays_unknown_effects_or_resets_attempts() {
    let source = spec(1);
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let context = context(&team);
    let checkpoint = Mutex::new(None);
    let _ = team.execute_ready(
        &source,
        &context,
        |i, _| Ok(outcome(i, &context)),
        |snapshot, _| {
            *checkpoint.lock().unwrap() = Some(serde_json::to_vec(snapshot).unwrap());
            Err(Error::Invalid("unknown commit outcome".into()))
        },
    );
    let mut restored: TeamPlan =
        serde_json::from_slice(&checkpoint.lock().unwrap().clone().unwrap()).unwrap();
    restored.validate(&source).unwrap();
    let interrupted = restored.recover_interrupted();
    assert_eq!(interrupted.len(), 3);
    assert!(restored.ready_wave(&source, &context).unwrap().is_empty());
    assert!(
        restored
            .tasks
            .iter()
            .filter(|t| interrupted.contains(&t.id))
            .all(|t| t.attempt == 1 && t.state == TaskState::Blocked)
    );
    restored
        .tasks
        .iter_mut()
        .find(|t| interrupted.contains(&t.id))
        .unwrap()
        .state = TaskState::Planned;
    assert!(
        restored
            .validate(&source)
            .unwrap_err()
            .to_string()
            .contains("replay")
    );
}
#[test]
fn rejection_requires_finding_and_repair_plan_and_preserves_old_artifact() {
    let source = spec(1);
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let context = context(&team);
    run_wave(&mut team, &source, &context);
    team.execute_ready(
        &source,
        &context,
        |i, progress| {
            progress(WorkerProgress::SessionStarted {
                session_id: session(i),
            })?;
            let mut result = outcome(i, &context);
            if let TaskOutcome::Reviewed(review) = &mut result {
                review.accepted = false;
                review.finding_ids.insert("finding-1".into());
                for value in review.criteria.values_mut() {
                    *value = false;
                }
            }
            Ok(result)
        },
        |_, _| Ok(()),
    )
    .unwrap();
    let task = team
        .tasks
        .iter()
        .find(|t| t.state == TaskState::ChangesRequested)
        .unwrap()
        .clone();
    assert!(
        team.request_repair(
            &task.id,
            RepairRound {
                finding_ids: BTreeSet::new(),
                plan: "Try again".into()
            }
        )
        .is_err()
    );
    team.request_repair(
        &task.id,
        RepairRound {
            finding_ids: BTreeSet::from(["finding-1".into()]),
            plan: "Read the omitted source and correct the unsupported claim".into(),
        },
    )
    .unwrap();
    let repaired = team.tasks.iter().find(|t| t.id == task.id).unwrap();
    assert_eq!(repaired.history[0].artifact, task.artifact);
    assert_eq!(repaired.attempt, 1);
    run_wave(&mut team, &source, &context);
    assert_eq!(
        team.tasks.iter().find(|t| t.id == task.id).unwrap().attempt,
        2
    );
}
#[test]
fn full_fake_workflow_finishes_with_verified_current_artifacts() {
    let source = spec(4);
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let context = context(&team);
    while !team.ready_wave(&source, &context).unwrap().is_empty() {
        run_wave(&mut team, &source, &context);
    }
    assert!(team.tasks.iter().all(|t| t.state == TaskState::Verified));
    let saved = serde_json::to_vec(&team).unwrap();
    let mut restored: TeamPlan = serde_json::from_slice(&saved).unwrap();
    restored.recover_interrupted();
    assert!(restored.ready_wave(&source, &context).unwrap().is_empty());
    restored.validate(&source).unwrap();
}
#[test]
fn stale_or_absent_gates_never_start_deliverables() {
    let source = spec(1);
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let mut context = context(&team);
    context.gates.clear();
    while !team.ready_wave(&source, &context).unwrap().is_empty() {
        run_wave(&mut team, &source, &context);
    }
    assert!(
        team.tasks
            .iter()
            .filter(|t| t.deliverable)
            .all(|t| t.state == TaskState::Planned && t.attempt == 0)
    );
}

#[test]
fn a_worker_cannot_omit_the_durable_session_receipt() {
    let source = spec(1);
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let context = context(&team);
    team.execute_ready(
        &source,
        &context,
        |i, _| Ok(outcome(i, &context)),
        |_, _| Ok(()),
    )
    .unwrap();
    assert!(
        team.tasks
            .iter()
            .filter(|task| task.stage == TaskStage::Discovery)
            .all(|task| task.state == TaskState::Blocked
                && task
                    .diagnostic
                    .as_ref()
                    .unwrap()
                    .contains("session receipt"))
    );
}

#[test]
fn failed_session_checkpoint_prevents_effects_and_blocks_future_waves() {
    let source = spec(1);
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let context = context(&team);
    let effects = AtomicUsize::new(0);
    assert!(
        team.execute_ready(
            &source,
            &context,
            |i, progress| {
                progress(WorkerProgress::SessionStarted {
                    session_id: session(i),
                })?;
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(outcome(i, &context))
            },
            |_, event| {
                if matches!(event, SchedulerEvent::SessionRecorded { .. }) {
                    Err(Error::Invalid("failed durable session checkpoint".into()))
                } else {
                    Ok(())
                }
            }
        )
        .is_err()
    );
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    assert!(team.checkpoint_uncertain);
    assert!(team.ready_wave(&source, &context).is_err());
}

#[test]
fn interrupted_unknown_writer_keeps_its_scope_lease_after_recovery() {
    let source = spec(1);
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let context = context(&team);
    let ready = team.ready_wave(&source, &context).unwrap();
    let writer = team
        .tasks
        .iter_mut()
        .find(|t| t.id == ready[0].task.id)
        .unwrap();
    writer.state = TaskState::Running;
    writer.attempt = 1;
    writer.write_scopes.insert("src".into());
    writer.integration = IntegrationStrategy::InPlace;
    writer.required_gates.extend([
        "context_ready".into(),
        "solution_selected".into(),
        "plan_ready".into(),
    ]);
    let other = team
        .tasks
        .iter_mut()
        .find(|t| t.id == ready[1].task.id)
        .unwrap();
    other.write_scopes.insert("src/child.rs".into());
    other.integration = IntegrationStrategy::InPlace;
    other.required_gates.extend([
        "context_ready".into(),
        "solution_selected".into(),
        "plan_ready".into(),
    ]);
    team.recover_interrupted();
    let wave = team.ready_wave(&source, &context).unwrap();
    assert!(
        !wave
            .iter()
            .any(|i| i.task.write_scopes.contains("src/child.rs"))
    );
}

#[test]
fn task_source_bindings_cannot_be_omitted_or_fabricated() {
    let source = spec(1);
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let mut context = context(&team);
    context.source_hashes.remove("objective");
    assert!(team.ready_wave(&source, &context).is_err());
    team.tasks[0].source_dependencies.remove("objective");
    assert!(team.validate(&source).is_err());
}

#[test]
fn worker_panic_preserves_an_explicit_blocked_attempt() {
    let source = spec(1);
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let context = context(&team);
    team.execute_ready(
        &source,
        &context,
        |i, progress| {
            progress(WorkerProgress::SessionStarted {
                session_id: session(i),
            })?;
            panic!("seeded worker panic after durable session");
        },
        |_, _| Ok(()),
    )
    .unwrap();
    assert!(
        team.tasks
            .iter()
            .filter(|task| task.stage == TaskStage::Discovery)
            .all(|task| task.state == TaskState::Blocked && task.attempt == 1)
    );
}

#[test]
fn checkpoint_panic_does_not_deadlock_waiting_session_acknowledgements() {
    let source = spec(1);
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let context = context(&team);
    let effects = AtomicUsize::new(0);
    assert!(
        team.execute_ready(
            &source,
            &context,
            |i, progress| {
                progress(WorkerProgress::SessionStarted {
                    session_id: session(i),
                })?;
                effects.fetch_add(1, Ordering::SeqCst);
                Ok(outcome(i, &context))
            },
            |_, event| {
                if matches!(event, SchedulerEvent::SessionRecorded { .. }) {
                    panic!("seeded checkpoint panic");
                }
                Ok(())
            }
        )
        .is_err()
    );
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    assert!(team.checkpoint_uncertain);
}

#[test]
fn actual_provider_failure_is_preserved_without_a_session_receipt() {
    let source = spec(1);
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let context = context(&team);
    team.execute_ready(
        &source,
        &context,
        |_, _| Err(Error::Invalid("provider authentication required".into())),
        |_, _| Ok(()),
    )
    .unwrap();
    assert!(
        team.tasks
            .iter()
            .filter(|task| task.stage == TaskStage::Discovery)
            .all(|task| task.diagnostic.as_deref() == Some("provider authentication required"))
    );
}

#[test]
fn a_model_proposal_cannot_manufacture_runtime_execution_history() {
    let source = spec(1);
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    team.validate_fresh_proposal(&source).unwrap();
    let current = context(&team);
    run_wave(&mut team, &source, &current);
    assert!(team.validate_fresh_proposal(&source).is_err());
}

#[test]
#[cfg(unix)]
fn aliased_write_scopes_fail_even_with_all_required_gates() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir(directory.path().join("real")).unwrap();
    std::fs::write(directory.path().join("real/file.rs"), "original").unwrap();
    std::os::unix::fs::symlink("real", directory.path().join("alias")).unwrap();
    let mut source = spec(1);
    source.workspace = directory.path().to_string_lossy().into_owned();
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let task = &mut team.tasks[0];
    task.write_scopes.insert("alias/file.rs".into());
    task.integration = IntegrationStrategy::InPlace;
    task.required_gates.extend([
        "context_ready".into(),
        "solution_selected".into(),
        "plan_ready".into(),
    ]);
    assert!(
        team.validate(&source)
            .unwrap_err()
            .to_string()
            .contains("symlink alias")
    );
    team.tasks[0].write_scopes = BTreeSet::from(["real/file.rs".into()]);
    std::fs::hard_link(
        directory.path().join("real/file.rs"),
        directory.path().join("second.rs"),
    )
    .unwrap();
    assert!(
        team.validate(&source)
            .unwrap_err()
            .to_string()
            .contains("hardlink alias")
    );
}

#[test]
fn baseline_plan_never_substitutes_for_required_positive_runner_coverage() {
    let mut spec = spec(1);
    spec.task_family = TaskFamily::Software;
    spec.check_command = vec!["true".into()];
    let mut team = TeamPlan::bootstrap(spec.task_family, &spec).unwrap();
    let task = team
        .tasks
        .iter_mut()
        .find(|t| !t.check_plan.is_empty())
        .unwrap();
    let role = team
        .agents
        .iter()
        .find(|a| a.id == task.owner_id)
        .unwrap()
        .role;
    task.change_contract = Some(changes::ChangeContract {
        kind: changes::ChangeKind::Feature,
        rationale: "Explicit requested test fixture addition".into(),
        source_refs: task.source_dependencies.clone(),
        bugfixes: vec![],
    });
    checks::validate_plan(task, true, role, spec.task_family).unwrap();
    task.check_plan[0].expected_exit_code = 1;
    assert!(checks::validate_plan(task, true, role, spec.task_family).is_err());
    task.check_plan[0].phase = checks::CheckPhase::BeforeExecution;
    assert!(checks::validate_plan(task, true, role, spec.task_family).is_err());
    let mut positive = task.check_plan[0].clone();
    positive.id = "post-change".into();
    positive.phase = checks::CheckPhase::BeforeVerification;
    positive.expected_exit_code = 0;
    task.check_plan.push(positive);
    checks::validate_plan(task, true, role, spec.task_family).unwrap();
    task.check_plan[0].expected_exit_code = -1;
    assert!(checks::validate_plan(task, true, role, spec.task_family).is_err());
    let old = serde_json::json!({"id":"old-check", "argv":["true"], "criterion_ids":["criterion"], "gate_ids":[], "rationale":"retained pre-baseline check"});
    let decoded: checks::PlannedCheck = serde_json::from_value(old).unwrap();
    assert_eq!(decoded.phase, checks::CheckPhase::BeforeVerification);
    assert_eq!(decoded.expected_exit_code, 0);
}

#[test]
fn software_change_contract_requires_scoped_complete_before_after_bugfix_obligations() {
    let mut source = spec(2);
    source.task_family = TaskFamily::Software;
    source.check_command = vec!["true".into()];
    let team = TeamPlan::bootstrap(source.task_family, &source).unwrap();
    let mut task = team.tasks.iter().find(|t| t.deliverable).unwrap().clone();
    let role = team
        .agents
        .iter()
        .find(|a| a.id == task.owner_id)
        .unwrap()
        .role;
    assert!(changes::validate_plan(&task, true, role, source.task_family).is_err());
    let mut before = task.check_plan[0].clone();
    before.id = "reproduce".into();
    before.phase = checks::CheckPhase::BeforeExecution;
    before.expected_exit_code = 1;
    before.gate_ids.clear();
    task.check_plan.push(before);
    task.change_contract = Some(changes::ChangeContract {
        kind: changes::ChangeKind::Bugfix,
        rationale: "Repair original violated contract".into(),
        source_refs: task.source_dependencies.clone(),
        bugfixes: vec![changes::Bugfix {
            id: "defect".into(),
            defect: "Original value violates required behavior".into(),
            criterion_ids: task.criterion_ids.clone(),
            oracle: "Compare actual values against original contract".into(),
            before: changes::BeforeProof::Reproduction {
                check_id: "reproduce".into(),
            },
            after_check_id: task.check_plan[0].id.clone(),
        }],
    });
    changes::validate_plan(&task, true, role, source.task_family).unwrap();
    task.check_plan[1].expected_exit_code = 0; // A successful measurement may still demonstrate the defect.
    changes::validate_plan(&task, true, role, source.task_family).unwrap();
    let mut invalid = task.clone();
    invalid.change_contract.as_mut().unwrap().bugfixes[0].after_check_id = "reproduce".into();
    assert!(changes::validate_plan(&invalid, true, role, source.task_family).is_err());
    invalid = task.clone();
    invalid.change_contract.as_mut().unwrap().bugfixes[0]
        .criterion_ids
        .clear();
    assert!(changes::validate_plan(&invalid, true, role, source.task_family).is_err());
    invalid = task.clone();
    invalid.change_contract.as_mut().unwrap().kind = changes::ChangeKind::Feature;
    assert!(changes::validate_plan(&invalid, true, role, source.task_family).is_err());
    invalid = task.clone();
    invalid
        .change_contract
        .as_mut()
        .unwrap()
        .source_refs
        .insert("file:unassigned".into());
    assert!(changes::validate_plan(&invalid, true, role, source.task_family).is_err());
    invalid = task.clone();
    invalid.change_contract.as_mut().unwrap().bugfixes[0].before =
        changes::BeforeProof::Alternative {
            investigation_check_id: "reproduce".into(),
            unavailable_reason: "Original runtime service unavailable".into(),
            method: "Use retained source and independent contract experiment".into(),
            source_refs: task.source_dependencies.clone(),
        };
    changes::validate_plan(&invalid, true, role, source.task_family).unwrap();
    if let changes::BeforeProof::Alternative {
        unavailable_reason, ..
    } = &mut invalid.change_contract.as_mut().unwrap().bugfixes[0].before
    {
        unavailable_reason.clear();
    }
    assert!(changes::validate_plan(&invalid, true, role, source.task_family).is_err());
}

#[test]
fn supervised_scheduler_holds_cross_stage_results_and_keeps_same_stage_work_available() {
    let source = spec(1);
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let mut current = context(&team);
    for _ in 0..2 {
        run_wave(&mut team, &source, &current);
    }
    current.accepted_results = Some(BTreeSet::new());
    assert!(team.ready_wave(&source, &current).unwrap().is_empty());
    let discovery: BTreeSet<_> = team
        .tasks
        .iter()
        .filter(|t| t.stage == TaskStage::Discovery)
        .map(|t| t.id.clone())
        .collect();
    current.accepted_results = Some(discovery);
    let planning = team.ready_wave(&source, &current).unwrap();
    assert!(!planning.is_empty());
    assert!(planning.iter().all(|i| i.task.stage == TaskStage::Planning));
    for _ in 0..2 {
        run_wave(&mut team, &source, &current);
    }
    let ready = team.ready_wave(&source, &current).unwrap();
    assert!(ready.iter().any(|i| i.task.stage == TaskStage::Planning));
    assert!(ready.iter().all(|i| i.task.stage != TaskStage::Execution));
}
