use architect::{domain::*, hash, id, recovery::{self, EditState, WorkspaceLease}, runtime::{Command, Runtime}, storage::Store, workspace};
use std::{path::Path, sync::Arc};
fn fixture(root:&Path)->RunSpec {
    let mut spec:RunSpec=serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace=root.to_str().unwrap().into();spec.input_files=vec!["a".into(),"b".into()];spec.editable_files=spec.input_files.clone();spec.check_command=vec!["true".into()];spec
}
fn interrupted(root:&Path)->(Run,Vec<FileEdit>) {
    std::fs::write(root.join("a"),"original a").unwrap();std::fs::write(root.join("b"),"original b").unwrap();
    let mut run=Run::new(id(),fixture(root)).unwrap();run.stage=Stage::Execution;run.status=Status::Running;run.attempt=1;run.reconciliation_required=true;
    let edits=vec![FileEdit{path:"a".into(),content:"new a".into()},FileEdit{path:"b".into(),content:"new b".into()}];
    run.pending_edits=recovery::prepare(&run,&edits).unwrap();(run,edits)
}
#[test]
fn crash_after_first_rename_recovers_receipts_without_replaying_or_losing_source() {
    let temp=tempfile::tempdir().unwrap();let db=temp.path().join("state.db");let (run,edits)=interrupted(temp.path());
    let mut store=Store::open(&db).unwrap();store.save(&run,"edit_intent",None).unwrap();drop(store);
    workspace::replace(temp.path(),"a",&run.pending_edits[0].before_hash,&edits[0].content).unwrap();
    // Simulate death before the post-write SQLite checkpoint.
    let runtime=Arc::new(Runtime::new(Store::open(&db).unwrap()).unwrap());
    let observed=runtime.store.lock().unwrap().get(&run.id).unwrap();assert_eq!(observed.status,Status::Blocked);assert!(observed.reconciliation_required);
    assert_eq!(observed.pending_edits[0].state,EditState::Applied);assert_eq!(observed.pending_edits[1].state,EditState::Unapplied);
    assert!(runtime.dispatch(&id(),Command::ApproveRepair{run_id:run.id.clone(),scope_hash:observed.scope_hash("repair")}).is_err());
    assert!(runtime.dispatch(&id(),Command::Reconcile{run_id:run.id.clone(),scope_hash:observed.scope_hash("reconcile"),input_hash:"stale".into()}).is_err());
    let current=workspace::read_inputs(temp.path(),&run.spec.input_files).unwrap();
    let reconciled=runtime.dispatch(&id(),Command::Reconcile{run_id:run.id.clone(),scope_hash:observed.scope_hash("reconcile"),input_hash:recovery::observe(&observed).unwrap().hash}).unwrap();
    assert_eq!(reconciled.status,Status::AwaitingStart);assert_eq!(reconciled.stage,Stage::Discovery);assert_eq!(reconciled.spec_revision,2);assert!(!reconciled.reconciliation_required);
    assert_eq!(reconciled.input_snapshot,current);assert!(reconciled.pending_edits.is_empty());
    assert_eq!(std::fs::read_to_string(temp.path().join("a")).unwrap(),"new a");assert_eq!(std::fs::read_to_string(temp.path().join("b")).unwrap(),"original b");
    let receipt=reconciled.evidence.iter().find(|e|e.source=="runtime:edit-reconciliation").unwrap();assert_eq!(receipt.output_hash,hash(&receipt.output));assert!(receipt.output.contains("unapplied"));
}
#[test]
fn external_edits_are_classified_as_conflicts_and_only_accepted_with_fresh_observed_hash() {
    let temp=tempfile::tempdir().unwrap();let (mut run,_)=interrupted(temp.path());std::fs::write(temp.path().join("a"),"user's edit").unwrap();
    recovery::inspect(&mut run).unwrap();assert_eq!(run.pending_edits[0].state,EditState::Conflict);
    assert_eq!(std::fs::read_to_string(temp.path().join("a")).unwrap(),"user's edit");
    let mut store=Store::open(&temp.path().join("state.db")).unwrap();store.save(&run,"interrupted",None).unwrap();let runtime=Arc::new(Runtime::new(store).unwrap());let run=runtime.store.lock().unwrap().get(&run.id).unwrap();
    let response=runtime.dispatch(&id(),Command::Reconcile{run_id:run.id.clone(),scope_hash:run.scope_hash("reconcile"),input_hash:recovery::observe(&run).unwrap().hash}).unwrap();
    assert_eq!(response.input_snapshot["a"],"user's edit");assert_eq!(response.status,Status::AwaitingStart);
}
#[test]
fn workspace_leases_serialize_overlaps_including_ancestors_and_allow_siblings() {
    let temp=tempfile::tempdir().unwrap();let child=temp.path().join("child");let sibling=temp.path().join("sibling");std::fs::create_dir(&child).unwrap();std::fs::create_dir(&sibling).unwrap();
    let root=temp.path().to_str().unwrap();let lease=WorkspaceLease::acquire(root).unwrap();assert!(WorkspaceLease::acquire(root).is_err());assert!(WorkspaceLease::acquire(child.to_str().unwrap()).is_err());drop(lease);
    let child_lease=WorkspaceLease::acquire(child.to_str().unwrap()).unwrap();assert!(WorkspaceLease::acquire(root).is_err());let sibling_lease=WorkspaceLease::acquire(sibling.to_str().unwrap()).unwrap();drop(child_lease);drop(sibling_lease);assert!(WorkspaceLease::acquire(root).is_ok());
}
#[test]
fn edit_batch_is_validated_in_full_before_any_write_intent() {
    let temp=tempfile::tempdir().unwrap();let (run,mut edits)=interrupted(temp.path());edits[1].path="unauthorized".into();assert!(recovery::prepare(&run,&edits).is_err());assert_eq!(std::fs::read_to_string(temp.path().join("a")).unwrap(),"original a");
    edits[1].path="a".into();assert!(recovery::prepare(&run,&edits).is_err());
}
#[test]
fn v1_database_migration_preserves_state_and_materializes_document_hash() {
    let temp=tempfile::tempdir().unwrap();let (run,_)=interrupted(temp.path());let db=temp.path().join("old.db");let connection=rusqlite::Connection::open(&db).unwrap();connection.execute_batch("CREATE TABLE runs(id TEXT PRIMARY KEY,state TEXT NOT NULL); CREATE TABLE events(seq INTEGER PRIMARY KEY AUTOINCREMENT,run_id TEXT NOT NULL,kind TEXT NOT NULL,payload TEXT NOT NULL); CREATE TABLE requests(key TEXT PRIMARY KEY,fingerprint TEXT NOT NULL,response TEXT NOT NULL); PRAGMA user_version=1;").unwrap();let text=serde_json::to_string(&run).unwrap();connection.execute("INSERT INTO runs(id,state) VALUES(?1,?2)",rusqlite::params![run.id,text]).unwrap();drop(connection);
    let store=Store::open(&db).unwrap();let (digest,total,chunk)=store.run_chunk(&run.id,0,text.len()).unwrap();assert_eq!(digest,hash(&text));assert_eq!(total,text.len());assert!(chunk==text,"migrated document differs");assert_eq!(store.get(&run.id).unwrap().id,run.id);
}
