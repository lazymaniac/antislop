use architect::{domain::*, hash, verification};

#[test]
fn captured_member_citations_preserve_parent_scope_type_and_provenance() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("docs")).unwrap();
    std::fs::write(root.path().join("docs/source.txt"), "original member").unwrap();
    let body =
        architect::workspace::read_inputs(root.path(), &["docs".into()]).unwrap()["docs"].clone();
    std::fs::write(root.path().join("literal.json"), &body).unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    spec.input_files = vec!["docs".into(), "literal.json".into()];
    let mut run = Run::new(architect::id(), spec).unwrap();
    let original_sources = run.source_hashes();
    let reference = "directory-member:[\"file:docs\",\"source.txt\"]";
    let output = ExecutionOutput {
        agent_messages: vec![],
        quality: None,
        artifact: "Structural source-binding fixture".into(),
        edits: vec![],
        solution: None,
        context: Some(ContextModel {
            facts: vec![ContextFact {
                claim: "Captured member observation".into(),
                source_ref: reference.into(),
                source_hash: hash("original member"),
            }],
            unknowns: vec![],
        }),
    };
    verification::validate_output(&run, &output).unwrap();
    let mut conflicting = output.clone();
    let mut second = conflicting.context.as_ref().unwrap().facts[0].clone();
    second.source_hash = hash("another version of the same member");
    conflicting.context.as_mut().unwrap().facts.push(second);
    assert!(
        verification::validate_output(&run, &conflicting).is_err(),
        "an earlier valid citation must not authenticate a different hash for the same alias"
    );
    std::fs::write(
        root.path().join("docs/source.txt"),
        "unobserved live change",
    )
    .unwrap();
    verification::validate_output(&run, &output).unwrap();
    assert_eq!(
        run.source_hashes(),
        original_sources,
        "citation support must not expand canonical selectors or approval hashes"
    );
    for (reference, source_hash) in [
        (
            "directory-member:[\"file:literal.json\",\"source.txt\"]",
            hash("original member"),
        ),
        (
            "directory-member:[\"file:docs\",\"../source.txt\"]",
            hash("original member"),
        ),
        (
            "directory-member:[\"file:docs\",\"missing.txt\"]",
            hash("original member"),
        ),
        (reference, hash("unobserved live change")),
        ("runner:baseline:[\"invented\"]", hash("fabricated output")),
    ] {
        let mut bad = output.clone();
        let fact = &mut bad.context.as_mut().unwrap().facts[0];
        fact.source_ref = reference.into();
        fact.source_hash = source_hash;
        assert!(verification::validate_output(&run, &bad).is_err());
    }
    let content = serde_json::to_string(&output).unwrap();
    let artifact = Artifact {
        id: architect::id(),
        stage: Stage::Discovery,
        attempt: 1,
        spec_revision: run.spec_revision,
        session_id: architect::id(),
        hash: hash(&content),
        content,
    };
    run.artifacts.push(artifact.clone());
    verification::register_artifact(&mut run, &artifact, &output).unwrap();
    let fact_id = format!("fact:{}:0", artifact.id);
    assert!(verification::provenance_node_current(&run, &fact_id));
    run.input_snapshot
        .insert("docs".into(), "changed capture".into());
    assert!(!verification::provenance_node_current(&run, &fact_id));
}
