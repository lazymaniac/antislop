use architect::{
    hash,
    workspace::{self, instructions},
};
use std::{collections::BTreeMap, path::Path};

#[test]
fn captured_instructions_keep_directory_scopes_and_every_source_binding() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("sub")).unwrap();
    for (path, body) in [
        ("AGENTS.md", "Root constraints"),
        ("sub/CLAUDE.md", "Nested instructions"),
        ("sub/GEMINI.md", "Potentially conflicting instructions"),
        ("README.md", "AGENTS.md is mentioned here"),
    ] {
        std::fs::write(root.path().join(path), body).unwrap();
    }
    let snapshot = workspace::read_inputs(
        root.path(),
        &[".".into(), "sub".into(), "sub/CLAUDE.md".into()],
    )
    .unwrap();
    let index = instructions::from_snapshot(root.path(), &snapshot).unwrap();
    assert!(!index.exhaustive);
    assert_eq!(index.documents.len(), 3);
    let nested = index
        .documents
        .iter()
        .find(|d| d.path == "sub/CLAUDE.md")
        .unwrap();
    assert_eq!(nested.directory_scope, "sub");
    assert_eq!(nested.content_hash, Some(hash("Nested instructions")));
    assert_eq!(nested.source_bindings.len(), 3);
    for binding in &nested.source_bindings {
        let source = binding.source_ref.strip_prefix("file:").unwrap();
        assert_eq!(binding.source_hash, hash(&snapshot[source]));
    }
    assert_eq!(
        index.applicable(root.path(), "sub/file.rs").unwrap().len(),
        3
    );
    assert_eq!(
        index
            .applicable(root.path(), "submarine/file.rs")
            .unwrap()
            .len(),
        1
    );
    assert!(index.contract.contains("explicit decision"));
    std::fs::write(root.path().join("AGENTS.md"), "New unobserved instructions").unwrap();
    assert_eq!(
        index,
        instructions::from_snapshot(root.path(), &snapshot).unwrap(),
        "historical packages must not read newer files"
    );
    let mut conflicting = snapshot;
    conflicting.insert(
        "sub/CLAUDE.md".into(),
        "Disagrees with the captured directory".into(),
    );
    assert!(instructions::from_snapshot(root.path(), &conflicting).is_err());
}

#[test]
fn instruction_link_and_binary_metadata_never_become_text_or_permission() {
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(
        outside.path().join("private"),
        "PRIVATE-OUTSIDE-INSTRUCTION",
    )
    .unwrap();
    std::os::unix::fs::symlink(
        outside.path().join("private"),
        root.path().join("GEMINI.md"),
    )
    .unwrap();
    std::fs::write(root.path().join("AGENTS.md"), [0, 255, 1]).unwrap();
    let snapshot = workspace::read_inputs(root.path(), &[".".into()]).unwrap();
    let index = instructions::from_snapshot(root.path(), &snapshot).unwrap();
    assert_eq!(index.documents.len(), 2);
    assert!(
        index
            .documents
            .iter()
            .all(|d| d.kind != "text" && d.content_hash.is_none())
    );
    assert!(
        !serde_json::to_string(&index)
            .unwrap()
            .contains("PRIVATE-OUTSIDE-INSTRUCTION")
    );
    assert!(
        index
            .contract
            .contains("cannot increase runtime permissions")
    );
    let mut conflicting = snapshot.clone();
    let mut other: serde_json::Value = serde_json::from_str(&snapshot["."]).unwrap();
    other["files"]["GEMINI.md"]["target_hex"] = serde_json::json!("00");
    conflicting.insert(
        root.path().to_string_lossy().into_owned(),
        other.to_string(),
    );
    assert!(
        instructions::from_snapshot(root.path(), &conflicting).is_err(),
        "different link targets cannot be deduplicated as the same instruction observation"
    );
    let nul = instructions::from_snapshot(
        root.path(),
        &BTreeMap::from([("AGENTS.md".into(), "rules\0data".into())]),
    )
    .unwrap();
    assert_eq!(nul.documents[0].kind, "binary");
    assert!(nul.documents[0].content_hash.is_none());
}

#[test]
fn instruction_index_reconstructs_missing_old_workspaces_and_rejects_scope_forgery() {
    let root = Path::new("/private/var/old-project-that-does-not-exist");
    let snapshot = BTreeMap::from([(
        "/var/old-project-that-does-not-exist/AGENTS.md".into(),
        "Historical rules".into(),
    )]);
    let index = instructions::from_snapshot(root, &snapshot).unwrap();
    assert_eq!(index.documents[0].path, "AGENTS.md");
    assert_eq!(
        index.observation_hash,
        hash(serde_json::to_vec(&snapshot).unwrap())
    );
    for selector in [
        "../AGENTS.md",
        "/var/old-project-that-does-not-exist-other/AGENTS.md",
    ] {
        assert!(
            instructions::from_snapshot(
                root,
                &BTreeMap::from([(selector.into(), "bad scope".into())])
            )
            .is_err()
        );
    }
    assert!(index.applicable(root, "../secret").is_err());
}

#[test]
fn embedded_envelopes_cannot_invent_new_instruction_documents() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("CLAUDE.md"), "A real captured instruction").unwrap();
    let envelope = workspace::read_inputs(root.path(), &[".".into()]).unwrap()["."].clone();
    let ambiguous = BTreeMap::from([("AGENTS.md".into(), envelope.clone())]);
    let index = instructions::from_snapshot(root.path(), &ambiguous).unwrap();
    assert_eq!(index.documents.len(), 1);
    assert_eq!(index.documents[0].path, "AGENTS.md");
    assert_eq!(index.documents[0].kind, "ambiguous_directory_or_text");
    assert_eq!(index.unresolved_directory_scopes, ["AGENTS.md"]);
    std::fs::write(root.path().join("data.json"), &envelope).unwrap();
    let literal = instructions::from_snapshot(
        root.path(),
        &BTreeMap::from([("data.json".into(), envelope.clone())]),
    )
    .unwrap();
    assert!(
        literal.documents.is_empty(),
        "a JSON file cannot manufacture instruction files below itself"
    );
    std::fs::write(root.path().join("AGENTS.md"), &envelope).unwrap();
    let literal = instructions::from_snapshot(root.path(), &ambiguous).unwrap();
    assert_eq!(literal.documents[0].kind, "text");
    assert_eq!(literal.documents[0].content_hash, Some(hash(&envelope)));
    let mut forged: serde_json::Value = serde_json::from_str(&envelope).unwrap();
    let entry = forged["files"]
        .as_object_mut()
        .unwrap()
        .remove("CLAUDE.md")
        .unwrap();
    forged["files"]["../CLAUDE.md"] = entry;
    assert!(
        instructions::from_snapshot(
            root.path(),
            &BTreeMap::from([(".".into(), forged.to_string())])
        )
        .is_err()
    );
}
