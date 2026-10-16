use architect::{
    context::Bundle,
    domain::{Run, RunSpec},
    hash,
    providers::TurnRequest,
    storage::Store,
    workspace::{
        self,
        observation::{self, Kind},
    },
};
use serde_json::{Value, json};

#[test]
fn a_new_mediated_version_of_an_unambiguous_legacy_file_retains_its_known_type() {
    use architect::domain::{FileEdit, FileOperation};
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("source"), "plain legacy file").unwrap();
    std::fs::create_dir(root.path().join("directory")).unwrap();
    std::fs::write(root.path().join("directory/child"), "leaf").unwrap();
    let body =
        workspace::read_inputs(root.path(), &["directory".into()]).unwrap()["directory"].clone();
    let mut snapshot = workspace::read_inputs(root.path(), &["source".into()]).unwrap();
    let old = snapshot.clone();
    let mut types = None;
    let edit = FileEdit {
        path: "source".into(),
        content: body,
        operation: FileOperation::Write,
    };
    workspace::apply_edit(root.path(), &edit, &hash("plain legacy file")).unwrap();
    observation::record_change(
        root.path(),
        &mut snapshot,
        &mut types,
        &edit,
        &hash("plain legacy file"),
    )
    .unwrap();
    assert_eq!(types.as_ref().unwrap().entries["source"].kind, Kind::File);
    types.unwrap().validate(&snapshot).unwrap();
    assert_eq!(old["source"], "plain legacy file");
}

#[test]
fn persisted_subdirectory_types_keep_original_members_and_instructions_after_paths_change() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("docs")).unwrap();
    std::fs::write(root.path().join("docs/AGENTS.md"), "Captured nested rules").unwrap();
    let body = workspace::read_inputs(root.path(), &["docs".into()]).unwrap()["docs"].clone();
    // Exactly identical bytes can describe either a real directory capture or
    // an ordinary file. Only the runtime's separate type observation differs.
    std::fs::write(root.path().join("GEMINI.md"), &body).unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    spec.input_files = vec!["docs".into(), "GEMINI.md".into()];
    let run = Run::new(architect::id(), spec).unwrap();
    let observation = run.input_observation.as_ref().unwrap();
    assert_eq!(observation.entries["docs"].kind, Kind::Directory);
    assert_eq!(observation.entries["GEMINI.md"].kind, Kind::File);
    assert_eq!(run.input_snapshot["docs"], run.input_snapshot["GEMINI.md"]);
    assert_eq!(
        observation.manifest("docs", &body).unwrap()["kind"],
        "directory"
    );
    let literal = observation.manifest("GEMINI.md", &body).unwrap();
    assert_eq!(literal["kind"], "text");
    assert_eq!(literal["sha256"], hash(&body));
    assert!(literal.get("files").is_none());
    let expected = run.project_instructions().unwrap();
    assert_eq!(expected.format, "project-instructions-v2");
    assert_eq!(
        expected
            .documents
            .iter()
            .map(|d| d.path.as_str())
            .collect::<Vec<_>>(),
        ["GEMINI.md", "docs/AGENTS.md"]
    );
    assert_eq!(
        expected.type_observation_hash,
        Some(hash(serde_json::to_vec(observation).unwrap()))
    );
    let mut store = Store::open(&root.path().join("state.db")).unwrap();
    store.save(&run, "observed_inputs_fixture", None).unwrap();
    drop(store);
    std::fs::remove_dir_all(root.path().join("docs")).unwrap();
    std::fs::write(root.path().join("docs"), &body).unwrap();
    std::fs::remove_file(root.path().join("GEMINI.md")).unwrap();
    std::fs::create_dir(root.path().join("GEMINI.md")).unwrap();
    std::fs::write(
        root.path().join("GEMINI.md/AGENTS.md"),
        "NEW UNOBSERVED RULES",
    )
    .unwrap();
    let store = Store::open(&root.path().join("state.db")).unwrap();
    let loaded = store.get(&run.id).unwrap();
    assert_eq!(loaded.project_instructions().unwrap(), expected);
    assert!(!workspace::inputs_current(&loaded).unwrap());
    let schema = json!({});
    let bundle = Bundle::for_turn(&TurnRequest {
        turn_id: "historical-view",
        run: &loaded,
        prompt: "Read the pinned original sources, not live files",
        schema: &schema,
        resume: None,
    })
    .unwrap();
    let index = bundle
        .documents
        .values()
        .find(|d| d.source_refs.contains("directory-index:\"file:docs\""))
        .unwrap();
    let index: Value = serde_json::from_str(&index.utf8).unwrap();
    assert_eq!(index["parent_source_ref"], "file:docs");
    assert_eq!(index["parent_document_hash"], hash(&body));
    assert_eq!(
        bundle.documents[&hash("Captured nested rules")].utf8,
        "Captured nested rules"
    );
    assert!(
        !bundle
            .documents
            .values()
            .any(|d| d.source_refs.contains("directory-index:\"file:GEMINI.md\""))
    );
    assert!(
        bundle
            .documents
            .values()
            .all(|d| !d.utf8.contains("NEW UNOBSERVED RULES"))
    );
    assert_eq!(loaded.spec.input_files, ["docs", "GEMINI.md"]);
    assert_eq!(loaded.input_snapshot, run.input_snapshot);
}

#[test]
fn a_file_replacing_a_directory_with_identical_bytes_invalidates_currentness() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("scope")).unwrap();
    std::fs::write(root.path().join("scope/source"), "original").unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    spec.input_files = vec!["scope".into()];
    let run = Run::new(architect::id(), spec).unwrap();
    assert!(workspace::inputs_current(&run).unwrap());
    let mut legacy = run.clone();
    legacy.input_observation = None;
    legacy.diff_baseline_observation = None;
    assert!(
        workspace::inputs_current(&legacy)
            .unwrap_err()
            .to_string()
            .contains("explicitly revise")
    );
    assert!(architect::recovery::prepare(&legacy, &[]).is_err());
    std::fs::remove_dir_all(root.path().join("scope")).unwrap();
    std::fs::write(root.path().join("scope"), &run.input_snapshot["scope"]).unwrap();
    assert_eq!(
        workspace::read_inputs(root.path(), &run.spec.input_files).unwrap(),
        run.input_snapshot
    );
    assert!(!workspace::inputs_current(&run).unwrap());
    assert!(architect::recovery::prepare(&run, &[]).is_err());
    let mut observed_run = run.clone();
    observed_run.reconciliation_required = true;
    let before = architect::recovery::observe(&observed_run).unwrap();
    assert!(matches!(
        before.inputs["scope"],
        architect::recovery::InputObservation::Readable {
            kind: Some(Kind::File),
            ..
        }
    ));
    let mut store = Store::open(&root.path().join("recovery.db")).unwrap();
    store
        .save(&observed_run, "type_change_checkpoint_fixture", None)
        .unwrap();
    let runtime = std::sync::Arc::new(architect::runtime::Runtime::new(store).unwrap());
    let reconciled = runtime
        .dispatch(
            &architect::id(),
            architect::runtime::Command::Reconcile {
                run_id: run.id.clone(),
                scope_hash: observed_run.scope_hash("reconcile"),
                input_hash: before.hash,
            },
        )
        .unwrap();
    assert_eq!(reconciled.status, architect::domain::Status::Blocked);
    assert!(reconciled.spec_revision > run.spec_revision);
    assert_eq!(reconciled.input_snapshot, run.input_snapshot);
    assert_eq!(reconciled.input_observation, run.input_observation);
    assert!(reconciled.diagnostic.unwrap().contains("explicitly revise"));
}

#[test]
fn mediated_content_and_types_advance_atomically_without_importing_unrelated_files() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("scope")).unwrap();
    std::fs::write(root.path().join("scope/source"), "original").unwrap();
    let capture = observation::capture(root.path(), &["scope".into()]).unwrap();
    let mut snapshot = capture.snapshot;
    let mut types = Some(capture.observation);
    let edit = architect::domain::FileEdit {
        path: "scope/source".into(),
        content: "edited".into(),
        operation: architect::domain::FileOperation::Write,
    };
    let before = (snapshot.clone(), types.clone());
    assert!(
        observation::record_change(
            root.path(),
            &mut snapshot,
            &mut types,
            &edit,
            &hash("wrong expected value")
        )
        .is_err()
    );
    assert_eq!((&snapshot, &types), (&before.0, &before.1));
    std::fs::write(root.path().join("scope/unrelated"), "UNRELATED USER CHANGE").unwrap();
    observation::record_change(
        root.path(),
        &mut snapshot,
        &mut types,
        &edit,
        &hash("original"),
    )
    .unwrap();
    types.as_ref().unwrap().validate(&snapshot).unwrap();
    assert_eq!(
        types.as_ref().unwrap().entries["scope"].kind,
        Kind::Directory
    );
    assert!(!snapshot["scope"].contains("UNRELATED USER CHANGE"));
    assert!(snapshot["scope"].contains("edited"));
}

#[test]
fn changed_or_missing_type_bindings_are_not_reinterpreted_and_special_files_are_nonblocking() {
    use std::os::unix::{ffi::OsStrExt, fs::symlink};
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("file"), "original").unwrap();
    let captured = observation::capture(root.path(), &["file".into()]).unwrap();
    for mutation in ["hash", "length", "missing", "snapshot", "directory"] {
        let mut changed = captured.observation.clone();
        match mutation {
            "hash" => changed.entries.get_mut("file").unwrap().source_hash = hash("changed"),
            "length" => changed.entries.get_mut("file").unwrap().utf8_bytes = 1,
            "missing" => changed.entries.clear(),
            "snapshot" => changed.snapshot_hash = hash("different capture"),
            "directory" => changed.entries.get_mut("file").unwrap().kind = Kind::Directory,
            _ => unreachable!(),
        }
        assert!(changed.validate(&captured.snapshot).is_err(), "{mutation}");
    }
    symlink(root.path().join("file"), root.path().join("link")).unwrap();
    assert!(observation::capture(root.path(), &["link".into()]).is_err());
    let fifo = root.path().join("fifo");
    let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    assert!(observation::capture(root.path(), &["fifo".into()]).is_err());
    assert!(workspace::read_input_bytes(root.path(), "fifo").is_err());
}

#[test]
fn a_literal_directory_envelope_remains_a_writable_file_and_never_grants_child_scope() {
    use architect::domain::{FileEdit, FileOperation};
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("directory")).unwrap();
    std::fs::write(root.path().join("directory/child"), "captured leaf").unwrap();
    let envelope =
        workspace::read_inputs(root.path(), &["directory".into()]).unwrap()["directory"].clone();
    std::fs::write(root.path().join("literal.json"), &envelope).unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    spec.input_files = vec!["literal.json".into()];
    spec.editable_files = vec!["literal.json".into()];
    spec.check_command = vec!["true".into()];
    let mut run = Run::new(architect::id(), spec).unwrap();
    assert!(
        !run.current_diff()
            .unwrap()
            .files
            .iter()
            .any(|f| f.path.starts_with("literal.json/"))
    );
    let edit = FileEdit {
        path: "literal.json".into(),
        content: "updated literal".into(),
        operation: FileOperation::Write,
    };
    let receipt = architect::recovery::prepare(&run, std::slice::from_ref(&edit))
        .unwrap()
        .remove(0);
    assert_eq!(receipt.before_hash, hash(&envelope));
    workspace::apply_edit(root.path(), &edit, &receipt.before_hash).unwrap();
    observation::record_change(
        root.path(),
        &mut run.input_snapshot,
        &mut run.input_observation,
        &edit,
        &receipt.before_hash,
    )
    .unwrap();
    assert!(
        architect::recovery::matches_version_observed(
            root.path(),
            &run.input_snapshot,
            run.input_observation.as_ref(),
            &receipt,
            false
        )
        .unwrap()
    );
    assert!(
        workspace::inputs::captured_text_observed(
            root.path(),
            &run.input_snapshot,
            run.input_observation.as_ref(),
            "literal.json/child"
        )
        .is_err()
    );
    assert_eq!(
        run.input_observation.as_ref().unwrap().entries["literal.json"].kind,
        Kind::File
    );
    let diff = run.current_diff().unwrap();
    assert!(
        diff.files
            .iter()
            .any(|f| f.path == "literal.json" && f.status == workspace::DiffFileStatus::Modified)
    );
    assert!(
        !diff
            .files
            .iter()
            .any(|f| f.path.starts_with("literal.json/"))
    );
    run.diff_baseline_observation
        .as_ref()
        .unwrap()
        .validate(run.diff_baseline())
        .unwrap();
}

#[test]
fn typed_file_diff_preserves_absolute_scope_and_observes_links_without_following_them() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(
        outside.path().join("private"),
        "PRIVATE TARGET MUST NOT ENTER DIFF",
    )
    .unwrap();
    std::fs::write(root.path().join("source"), "original").unwrap();
    std::fs::create_dir(root.path().join(".architect")).unwrap();
    std::fs::write(
        root.path().join(".architect/transcript.json"),
        "PRIVATE RUNTIME DATA MUST NOT ENTER DIFF",
    )
    .unwrap();
    std::os::unix::fs::symlink(outside.path(), root.path().join("outside")).unwrap();
    std::os::unix::fs::symlink(root.path(), root.path().join("cycle")).unwrap();
    let selector = root.path().join("source").to_string_lossy().into_owned();
    let capture = observation::capture(root.path(), std::slice::from_ref(&selector)).unwrap();
    let diff = workspace::compute_diff_observed(
        root.path(),
        &capture.snapshot,
        Some(&capture.observation),
    )
    .unwrap();
    assert!(diff.files.iter().all(|f| f.path != "source"
        && !f.path.contains("/private")
        && !f.path.starts_with("cycle/")));
    assert!(
        !serde_json::to_string(&diff)
            .unwrap()
            .contains("PRIVATE TARGET MUST NOT ENTER DIFF")
    );
    assert!(
        !serde_json::to_string(&diff)
            .unwrap()
            .contains("PRIVATE RUNTIME DATA MUST NOT ENTER DIFF")
    );
    assert!(
        diff.files
            .iter()
            .any(|f| f.path == "outside" && f.metadata_change.is_some())
    );
}
