use architect::{
    domain::{Approval, Run, RunSpec, Status},
    hash, id,
    process::container::ContainerResource,
    runtime::{
        Command, Runtime,
        check_resources::{self as custody, Phase},
        team::checks::{CheckState, Invocation},
    },
    storage::Store,
    team::checks::{CheckPhase, PlannedCheck},
};
use std::{
    collections::BTreeSet,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

fn fixture(root: &Path) -> (Store, Run, String, PathBuf) {
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.to_string_lossy().into();
    let mut run = Run::new(id(), spec).unwrap();
    run.status = Status::Running;
    let check_id = id();
    let approval = hash("owned wave approval");
    run.approvals.push(Approval {
        action: "step_team".into(),
        scope_hash: approval.clone(),
        spec_revision: run.spec_revision,
        actor: "local_user".into(),
    });
    let inputs_hash = hash(serde_json::to_vec(&run.input_snapshot).unwrap());
    run.task_checks.push(Invocation {
        id: check_id.clone(),
        task_id: "task".into(),
        task_attempt: 0,
        artifact_id: None,
        input_snapshot: Some(run.input_snapshot.clone()),
        artifact_hash: inputs_hash.clone(),
        plan: PlannedCheck {
            id: "owned-check".into(),
            argv: vec!["true".into()],
            phase: CheckPhase::BeforeExecution,
            expected_exit_code: 0,
            criterion_ids: BTreeSet::from(["REQ-1:stages".into()]),
            gate_ids: BTreeSet::new(),
            rationale: "Owned lifecycle fixture".into(),
        },
        spec_revision: run.spec_revision,
        configuration_hash: hash(serde_json::to_vec(&run.spec).unwrap()),
        source_hashes: run.source_hashes(),
        authorization_scope: approval,
        before_hash: inputs_hash,
        after_hash: None,
        state: CheckState::Intent,
        success: false,
        output_hash: None,
        evidence_id: None,
        restoration: None,
    });
    let path = root.join("state.db");
    let mut store = Store::open(&path).unwrap();
    store.save(&run, "owned_check_intent", None).unwrap();
    (store, run, check_id, path)
}
fn reserve(store: &mut Store, run: &mut Run, check: &str) -> String {
    custody::reserve(
        run,
        check,
        PathBuf::from("/absolute/owned-engine"),
        format!("sha256:{}", "b".repeat(64)),
        &mut |r, e| store.save(r, e, None),
    )
    .unwrap()
}
fn created(store: &mut Store, run: &mut Run, record: &str) -> ContainerResource {
    let fence = custody::begin_create(run, record, &mut |r, e| store.save(r, e, None)).unwrap();
    let resource = ContainerResource {
        binary: fence.record().binary().into(),
        container_id: "a".repeat(64),
        image_id: fence.record().image_id().into(),
        owner_token: fence.record().owner_token().into(),
    };
    custody::record_created(run, record, &resource, &mut |r, e| store.save(r, e, None)).unwrap();
    resource
}

#[test]
fn uncertain_creation_observation_rejects_foreign_or_missing_bindings_without_effects() {
    let root = tempfile::tempdir().unwrap();
    let (mut store, mut run, check, path) = fixture(root.path());
    let binary = root.path().join("engine");
    fs::write(
        &binary,
        r#"#!/usr/bin/env python3
import sys,json,pathlib
p=pathlib.Path(__file__).parent
s=json.loads((p/'engine-state.json').read_text())
a=sys.argv[1:]
with (p/'engine-calls').open('a') as f:f.write(json.dumps(a)+'\n')
assert a[-1]==s['expected_name']
if a[:2]==['container','exists']:
 sys.exit(1 if s['mode']=='absent' else 125 if s['mode']=='unavailable' else 0)
assert a[:3]==['container','inspect','--format']
assert '{{json .Name}}' in a[3] and '{{json .ID}}' in a[3]
if s['mode']=='oversized':print('x'*8192)
elif s['mode']=='duplicate':print('{"name":"first","name":"second"}')
else:print(json.dumps(s['binding']))
"#,
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let record = custody::reserve(
        &mut run,
        &check,
        binary,
        format!("sha256:{}", "b".repeat(64)),
        &mut |r, e| store.save(r, e, None),
    )
    .unwrap();
    custody::begin_create(&mut run, &record, &mut |r, e| store.save(r, e, None)).unwrap();
    run.status = Status::Blocked;
    run.task_checks[0].state = CheckState::Unknown;
    run.reconciliation_required = true;
    store
        .save(&run, "owned_interrupted_creation", None)
        .unwrap();
    let reserved = run.check_resources.get(&record).unwrap().clone();
    let good = serde_json::json!({"name":reserved.name(),"id":"a".repeat(64),"image":reserved.image_id(),"token":reserved.owner_token()});
    let write = |mode: &str, binding: &serde_json::Value| {
        fs::write(
            root.path().join("engine-state.json"),
            serde_json::to_vec(
                &serde_json::json!({"expected_name":reserved.name(),"mode":mode,"binding":binding}),
            )
            .unwrap(),
        )
        .unwrap()
    };
    let before = serde_json::to_vec(&run).unwrap();
    for fault in [
        "absent",
        "unavailable",
        "name",
        "image",
        "token",
        "id",
        "duplicate",
        "oversized",
    ] {
        let mut binding = good.clone();
        if ["name", "image", "token", "id"].contains(&fault) {
            binding[fault] = serde_json::json!("foreign");
        }
        write(fault, &binding);
        let observed = custody::observe_creation(&reserved);
        if fault == "absent" {
            assert!(observed.unwrap().is_none());
        } else {
            assert!(observed.is_err(), "{fault}");
        }
        assert_eq!(serde_json::to_vec(&run).unwrap(), before);
        assert_eq!(
            serde_json::to_vec(&store.get(&run.id).unwrap()).unwrap(),
            before
        );
        assert!(reserved.unresolved());
    }
    write("valid", &good);
    let fault = rusqlite::Connection::open(&path).unwrap();
    fault.execute_batch("CREATE TRIGGER refuse_observation BEFORE INSERT ON events WHEN NEW.kind='check_container_checkpoint' BEGIN SELECT RAISE(ABORT,'owned observation fault'); END;").unwrap();
    let proof = custody::observe_creation(&reserved).unwrap().unwrap();
    assert!(
        custody::record_observed_creation(&mut run, &record, proof, &mut |r, e| store
            .save(r, e, None))
        .is_err()
    );
    assert_eq!(serde_json::to_vec(&run).unwrap(), before);
    assert_eq!(
        serde_json::to_vec(&store.get(&run.id).unwrap()).unwrap(),
        before
    );
    fault
        .execute_batch("DROP TRIGGER refuse_observation")
        .unwrap();
    let proof = custody::observe_creation(&reserved).unwrap().unwrap();
    custody::record_observed_creation(&mut run, &record, proof, &mut |r, e| store.save(r, e, None))
        .unwrap();
    assert_eq!(
        run.check_resources.get(&record).unwrap().phase(),
        Some(Phase::Created)
    );
    assert!(custody::begin_start(&mut run, &record, &mut |r, e| store.save(r, e, None)).is_err());
    let resource =
        custody::begin_cleanup(&mut run, &record, &mut |r, e| store.save(r, e, None)).unwrap();
    assert_eq!(resource.container_id, "a".repeat(64));
    assert!(run.evidence.is_empty());
    assert!(!run.task_checks[0].success);
    let calls = fs::read_to_string(root.path().join("engine-calls")).unwrap();
    assert!(calls.lines().all(|line| {
        let args: Vec<String> = serde_json::from_str(line).unwrap();
        args[0] == "container" && ["inspect", "exists"].contains(&args[1].as_str())
    }));
}

#[test]
fn checkpoints_survive_reopen_and_unknown_operations_cannot_be_replayed() {
    let root = tempfile::tempdir().unwrap();
    let (mut store, mut run, check, path) = fixture(root.path());
    let record = reserve(&mut store, &mut run, &check);
    let fence =
        custody::begin_create(&mut run, &record, &mut |r, e| store.save(r, e, None)).unwrap();
    assert_eq!(fence.record().phase(), Some(Phase::CreateUnknown));
    drop(store);
    let mut store = Store::open(&path).unwrap();
    run = store.get(&run.id).unwrap();
    assert!(custody::begin_create(&mut run, &record, &mut |r, e| store.save(r, e, None)).is_err());
    assert!(
        custody::abort_before_create(&mut run, &record, &mut |r, e| store.save(r, e, None))
            .is_err()
    );
    assert!(custody::begin_cleanup(&mut run, &record, &mut |r, e| store.save(r, e, None)).is_err());
    let resource = ContainerResource {
        binary: fence.record().binary().into(),
        container_id: "a".repeat(64),
        image_id: fence.record().image_id().into(),
        owner_token: fence.record().owner_token().into(),
    };
    custody::record_created(&mut run, &record, &resource, &mut |r, e| {
        store.save(r, e, None)
    })
    .unwrap();
    let permit =
        custody::begin_start(&mut run, &record, &mut |r, e| store.save(r, e, None)).unwrap();
    assert_eq!(permit.into_resource(), resource);
    drop(store);
    let mut store = Store::open(&path).unwrap();
    let mut saved = store.get(&run.id).unwrap();
    assert_eq!(
        saved.check_resources.get(&record).unwrap().phase(),
        Some(Phase::StartUnknown)
    );
    assert!(custody::begin_start(&mut saved, &record, &mut |r, e| store.save(r, e, None)).is_err());
    assert!(
        custody::reserve(
            &mut saved,
            &check,
            resource.binary,
            resource.image_id,
            &mut |r, e| store.save(r, e, None)
        )
        .is_err()
    );
    assert!(saved.evidence.is_empty());
}

#[test]
fn real_sqlite_checkpoint_failure_cannot_issue_a_create_or_start_fence() {
    for before_start in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let (mut store, mut run, check, path) = fixture(root.path());
        let record = reserve(&mut store, &mut run, &check);
        if before_start {
            created(&mut store, &mut run, &record);
        }
        let before = serde_json::to_vec(&run).unwrap();
        let sequence = store.sequence().unwrap();
        let fault = rusqlite::Connection::open(&path).unwrap();
        fault.execute_batch("CREATE TRIGGER reject_custody BEFORE INSERT ON events WHEN NEW.kind='check_container_checkpoint' BEGIN SELECT RAISE(ABORT,'owned checkpoint fault'); END;").unwrap();
        if before_start {
            assert!(
                custody::begin_start(&mut run, &record, &mut |r, e| store.save(r, e, None))
                    .is_err()
            );
        } else {
            assert!(
                custody::begin_create(&mut run, &record, &mut |r, e| store.save(r, e, None))
                    .is_err()
            );
        }
        assert_eq!(serde_json::to_vec(&run).unwrap(), before);
        assert_eq!(
            serde_json::to_vec(&store.get(&run.id).unwrap()).unwrap(),
            before
        );
        assert_eq!(store.sequence().unwrap(), sequence);
    }
}

#[test]
fn changed_intent_approval_pause_or_uncertainty_refuses_new_effect_fences() {
    for change in [
        "command",
        "approval",
        "unknown",
        "paused",
        "reconciliation",
        "revision",
    ] {
        let root = tempfile::tempdir().unwrap();
        let (mut store, mut run, check, _) = fixture(root.path());
        let record = reserve(&mut store, &mut run, &check);
        created(&mut store, &mut run, &record);
        match change {
            "command" => run.task_checks[0].plan.argv.push("changed".into()),
            "approval" => run.approvals.clear(),
            "unknown" => run.task_checks[0].state = CheckState::Unknown,
            "paused" => run.status = Status::Paused,
            "reconciliation" => run.reconciliation_required = true,
            _ => run.spec_revision += 1,
        }
        let before = serde_json::to_vec(&run).unwrap();
        assert!(
            custody::begin_start(&mut run, &record, &mut |r, e| store.save(r, e, None)).is_err(),
            "{change}"
        );
        assert_eq!(serde_json::to_vec(&run).unwrap(), before);
        assert_eq!(
            custody::begin_cleanup(&mut run, &record, &mut |r, e| store.save(r, e, None))
                .unwrap()
                .container_id,
            "a".repeat(64)
        );
    }
}

#[test]
fn restart_preserves_pause_cancel_and_unknown_custody_without_touching_the_engine() {
    for status in [Status::Running, Status::Paused, Status::Cancelled] {
        let root = tempfile::tempdir().unwrap();
        let (mut store, mut run, check, path) = fixture(root.path());
        let record = reserve(&mut store, &mut run, &check);
        created(&mut store, &mut run, &record);
        custody::begin_start(&mut run, &record, &mut |r, e| store.save(r, e, None)).unwrap();
        run.status = status;
        if status == Status::Paused {
            run.paused_from = Some(Status::Running);
        }
        store.save(&run, "owned_crash_boundary", None).unwrap();
        drop(store);
        let runtime = std::sync::Arc::new(Runtime::new(Store::open(&path).unwrap()).unwrap());
        let saved = Store::open(&path).unwrap().get(&run.id).unwrap();
        assert!(saved.reconciliation_required);
        assert_eq!(
            saved.check_resources.get(&record).unwrap().phase(),
            Some(Phase::StartUnknown)
        );
        assert_eq!(
            saved.status,
            if status == Status::Running {
                Status::Blocked
            } else {
                status
            }
        );
        if status == Status::Paused {
            assert_eq!(saved.paused_from, Some(Status::Blocked));
        }
        let observation = architect::recovery::observe(&saved).unwrap();
        let error = runtime
            .dispatch(
                &id(),
                Command::Reconcile {
                    run_id: run.id.clone(),
                    scope_hash: saved.scope_hash("reconcile"),
                    input_hash: observation.hash,
                },
            )
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("container cleanup is unconfirmed")
        );
        assert!(
            Store::open(&path)
                .unwrap()
                .get(&run.id)
                .unwrap()
                .reconciliation_required
        );
    }
}

#[test]
fn only_actual_engine_absence_can_record_removed_and_failed_commit_consumes_no_authority() {
    let root = tempfile::tempdir().unwrap();
    let (mut store, mut run, check, _) = fixture(root.path());
    let binary = root.path().join("owned-engine");
    fs::write(&binary,"#!/usr/bin/env python3\nimport sys\nassert sys.argv[1:3]==['container','exists']\nsys.exit(1)\n").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let record = custody::reserve(
        &mut run,
        &check,
        binary,
        format!("sha256:{}", "b".repeat(64)),
        &mut |r, e| store.save(r, e, None),
    )
    .unwrap();
    let resource = created(&mut store, &mut run, &record);
    custody::begin_cleanup(&mut run, &record, &mut |r, e| store.save(r, e, None)).unwrap();
    let proof = resource.try_remove().unwrap().unwrap();
    let before = serde_json::to_vec(&run).unwrap();
    assert!(
        custody::record_removed(&mut run, &record, proof, &mut |_, _| Err(
            architect::Error::Invalid("owned persistence fault".into())
        ))
        .is_err()
    );
    assert_eq!(serde_json::to_vec(&run).unwrap(), before);
    let proof = resource.try_remove().unwrap().unwrap();
    custody::record_removed(&mut run, &record, proof, &mut |r, e| store.save(r, e, None)).unwrap();
    assert_eq!(
        run.check_resources.get(&record).unwrap().phase(),
        Some(Phase::Removed)
    );
    assert!(!run.check_resources.get(&record).unwrap().unresolved());
    assert!(
        run.evidence.is_empty(),
        "cleanup is not check-result evidence"
    );
}

#[test]
fn empty_ledger_preserves_old_bytes_and_corrupt_history_cannot_enter_sqlite() {
    let root = tempfile::tempdir().unwrap();
    let (mut store, mut run, check, _) = fixture(root.path());
    let old = serde_json::to_string(&run).unwrap();
    assert!(!old.contains("check_resources"));
    assert_eq!(
        serde_json::to_string(&serde_json::from_str::<Run>(&old).unwrap()).unwrap(),
        old
    );
    let record = reserve(&mut store, &mut run, &check);
    let original = serde_json::to_value(&run).unwrap();
    for fault in ["empty", "foreign", "start_without_create", "changed_id"] {
        let mut value = original.clone();
        let entry = &mut value["check_resources"]["records"][&record];
        match fault {
            "empty" => entry["steps"] = serde_json::json!([]),
            "foreign" => entry["run_id"] = serde_json::json!("foreign"),
            "start_without_create" => entry["steps"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({"phase":"start_unknown","container_id":"a".repeat(64)})),
            _ => entry["steps"][0]["container_id"] = serde_json::json!("a".repeat(64)),
        }
        let forged: Run = serde_json::from_value(value).unwrap();
        assert!(
            forged.check_resources.validate_for_run(&forged.id).is_err(),
            "{fault}"
        );
        assert!(store.save(&forged, "forged", None).is_err(), "{fault}");
    }
}
