//! Independent adversarial checks for scheduler contracts.
use architect::{config::RoleKind, domain::RunSpec, hash, quality::TaskFamily, team::*};
use std::collections::{BTreeMap, BTreeSet};

fn spec() -> RunSpec {
    serde_json::from_value(serde_json::json!({
        "task_family":"research", "objective":"Preserve both independent obligations", "workspace":".",
        "provider":"fake", "model":null,
        "requirements":[
            {"id":"R0","source_ref":"user:0","original_text":"Investigate claim zero, including exceptions","criteria":{"verified":"Check claim zero and exceptions"}},
            {"id":"R1","source_ref":"user:1","original_text":"Investigate claim one","criteria":{"verified":"Check claim one"}}
        ],
        "plan_items":[
            {"id":"P0","dependencies":[],"requirement_ids":["R0"],"description":"Investigate claim zero"},
            {"id":"P1","dependencies":[],"requirement_ids":["R1"],"description":"Investigate claim one"}
        ]
    })).unwrap()
}
fn context(team: &TeamPlan) -> GateContext {
    GateContext {
        accepted_results: None,
        gate_tasks: BTreeMap::new(),
        spec_revision: team.spec_revision,
        source_hashes: team
            .tasks
            .iter()
            .flat_map(|t| &t.source_dependencies)
            .map(|id| (id.clone(), hash(id)))
            .collect(),
        gates: BTreeMap::new(),
    }
}
fn outcome(i: &TaskInvocation, c: &GateContext) -> TaskOutcome {
    let session = format!("session-{}", i.agent.id);
    match i.operation {
        TaskOperation::Execute => TaskOutcome::Executed(ArtifactRef {
            execution_id: None,
            dependency_hashes: i.dependency_hashes.clone(),
            id: format!("artifact-{}", i.task.id),
            task_id: i.task.id.clone(),
            created_by: i.agent.id.clone(),
            session_id: session,
            attempt: i.task.attempt,
            spec_revision: c.spec_revision,
            content_hash: hash(&i.task.id),
            source_hashes: c.source_hashes.clone(),
            evidence_refs: BTreeSet::from(["actual-evidence".into()]),
        }),
        TaskOperation::Verify => TaskOutcome::Reviewed(ReviewDecision {
            verifier_id: i.agent.id.clone(),
            session_id: session,
            artifact_hash: i.task.artifact.as_ref().unwrap().content_hash.clone(),
            spec_revision: c.spec_revision,
            criteria: i
                .task
                .criterion_ids
                .iter()
                .map(|id| (id.clone(), true))
                .collect(),
            evidence_refs: BTreeSet::from(["independent-evidence".into()]),
            finding_ids: BTreeSet::new(),
            accepted: true,
        }),
    }
}

#[test]
fn original_plan_requirement_assignments_cannot_be_swapped() {
    let source = spec();
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    for task in team.tasks.iter_mut().filter(|t| t.deliverable) {
        let replacement = if task.plan_item_ids.contains("P0") {
            "R1"
        } else {
            "R0"
        };
        task.requirement_ids = BTreeSet::from([replacement.into()]);
        task.criterion_ids = BTreeSet::from([format!("REQ:{replacement}:verified")]);
        task.source_dependencies
            .retain(|id| !id.starts_with("requirement:"));
        task.source_dependencies
            .insert(format!("requirement:{replacement}"));
    }
    assert!(
        team.validate(&source).is_err(),
        "global coverage must not hide a changed original plan contract"
    );
}

#[test]
fn team_requires_an_actual_explicit_orchestrator() {
    let source = spec();
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    team.agents
        .iter_mut()
        .find(|a| a.kind == RoleKind::Orchestrator)
        .unwrap()
        .kind = RoleKind::Executor;
    assert!(
        team.validate(&source).is_err(),
        "a catalog label cannot replace the explicit orchestrator role"
    );
}

#[test]
fn changed_sources_invalidate_verified_prerequisites() {
    let source = spec();
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let mut current = context(&team);
    for _ in 0..2 {
        team.execute_ready(
            &source,
            &current,
            |i, progress| {
                progress(WorkerProgress::SessionStarted {
                    session_id: format!("session-{}", i.agent.id),
                })?;
                Ok(outcome(i, &current))
            },
            |_, _| Ok(()),
        )
        .unwrap();
    }
    assert!(team.tasks.iter().any(|t| t.state == TaskState::Verified));
    current.source_hashes.insert(
        "objective".into(),
        hash("changed underlying source snapshot"),
    );
    let ready = team.ready_wave(&source, &current);
    assert!(
        ready.is_err() || ready.unwrap().is_empty(),
        "downstream tasks cannot consume stale verified artifacts"
    );
}

#[test]
fn expansion_cannot_start_ungated_implementation() {
    let source = spec();
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let (owner, verifier) = team
        .add_pair(
            CatalogRole::Implementer,
            "expanded-code",
            "Implement an additional helper".into(),
        )
        .unwrap();
    let mut task = TaskRecord::new(
        "expanded-code".into(),
        owner,
        verifier,
        "Write a helper".into(),
        TaskStage::Execution,
        BTreeSet::from(["R0".into()]),
        BTreeSet::from(["REQ:R0:verified".into()]),
    );
    task.write_scopes.insert("src/helper.rs".into());
    task.integration = IntegrationStrategy::InPlace;
    team.tasks.push(task);
    let ready = team.ready_wave(&source, &context(&team));
    assert!(
        ready.is_err() || !ready.unwrap().iter().any(|i| i.task.id == "expanded-code"),
        "support tasks that write solution code also require current implementation gates"
    );
}

#[test]
fn interrupted_effect_cannot_replay_by_inventing_a_repair_field() {
    let source = spec();
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let task = &mut team.tasks[0];
    task.state = TaskState::Running;
    task.attempt = 1;
    team.recover_interrupted();
    let task = &mut team.tasks[0];
    task.state = TaskState::Planned;
    task.repair = Some(RepairRound {
        finding_ids: BTreeSet::from(["invented".into()]),
        plan: "Retry the unknown effect".into(),
    });
    assert!(
        team.validate(&source).is_err(),
        "a repair must derive from a durable rejection, not merely be present"
    );
}

#[test]
#[cfg(unix)]
fn symlink_aliases_share_one_write_lease() {
    let temporary = tempfile::tempdir().unwrap();
    std::fs::create_dir(temporary.path().join("real")).unwrap();
    std::fs::write(temporary.path().join("real/file.rs"), "original").unwrap();
    std::os::unix::fs::symlink("real", temporary.path().join("alias")).unwrap();
    let mut source = spec();
    source.workspace = temporary.path().to_string_lossy().into_owned();
    let mut team = TeamPlan::bootstrap(TaskFamily::Research, &source).unwrap();
    let initial = team.ready_wave(&source, &context(&team)).unwrap();
    assert_eq!(initial.len(), 3);
    for (i, path) in initial
        .iter()
        .zip(["real/file.rs", "alias/file.rs", "independent.md"])
    {
        let task = team.tasks.iter_mut().find(|t| t.id == i.task.id).unwrap();
        task.write_scopes.insert(path.into());
        task.integration = IntegrationStrategy::InPlace;
    }
    let ready = team.ready_wave(&source, &context(&team));
    assert!(
        ready.is_err() || ready.unwrap().len() == 2,
        "two names for the same file must not run concurrently"
    );
}
