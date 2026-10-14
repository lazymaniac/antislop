use architect::{
    domain::*,
    providers::{self, ObservedInvocation},
    verification,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn registry(prompt: &str) -> Value {
    let body = prompt
        .split("BEGIN INLINE CONTEXT REGISTRY\n")
        .nth(1)
        .unwrap()
        .split("\nEND INLINE CONTEXT REGISTRY")
        .next()
        .unwrap();
    serde_json::from_str(body).unwrap()
}
fn raw_document(registry: &Value, id: &str) -> String {
    let document = registry["documents"]
        .get(id)
        .or_else(|| {
            registry["documents"]
                .as_object()
                .unwrap()
                .values()
                .find(|d| d.get("sha256").unwrap_or(&d[0]) == id)
        })
        .unwrap();
    let document = if let Some(tuple) = document.as_array() {
        assert_eq!(tuple.len(), 3);
        if tuple[2].is_string() {
            serde_json::json!({"sha256":tuple[0],"utf8_bytes":tuple[1],"utf8":tuple[2]})
        } else {
            serde_json::json!({"sha256":tuple[0],"utf8_bytes":tuple[1],"parts":tuple[2]})
        }
    } else {
        document.clone()
    };
    let raw = if let Some(raw) = document["utf8"].as_str() {
        raw.to_owned()
    } else {
        document["parts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|part| {
                if let Some(text) = part.as_str() {
                    text.to_owned()
                } else {
                    let index = part.as_i64().unwrap();
                    assert_ne!(index, 0);
                    let child = format!("d{}", index.unsigned_abs());
                    assert!(
                        registry["documents"][&child]
                            .get("utf8_bytes")
                            .unwrap_or(&registry["documents"][&child][1])
                            .as_u64()
                            .unwrap()
                            < document["utf8_bytes"].as_u64().unwrap()
                    );
                    let raw = raw_document(registry, &child);
                    if index < 0 {
                        serde_json::to_string(&raw).unwrap()
                    } else {
                        raw
                    }
                }
            })
            .collect()
    };
    assert_eq!(architect::hash(&raw), document["sha256"]);
    assert_eq!(document["utf8_bytes"], raw.len());
    raw
}
fn resolve(value: &Value, registry: &Value) -> Value {
    match value {
        Value::Object(object) if object.len() == 1 && object.contains_key("json_value_ref") => {
            serde_json::from_str(&raw_document(
                registry,
                object["json_value_ref"].as_str().unwrap(),
            ))
            .unwrap()
        }
        Value::Object(object) if object.len() == 1 && object.contains_key("document_ref") => {
            let id = object["document_ref"].as_str().unwrap();
            Value::String(raw_document(registry, id))
        }
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| (key.clone(), resolve(value, registry)))
                .collect(),
        ),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| resolve(value, registry))
                .collect(),
        ),
        other => other.clone(),
    }
}
fn section(registry: &Value, name: &str) -> Value {
    let value = &registry["sections"][name];
    let resolved = resolve(value, registry);
    if value.get("document_ref").is_some() {
        serde_json::from_str(resolved.as_str().unwrap()).unwrap()
    } else {
        resolved
    }
}

#[test]
fn exact_registry_preserves_artifact_evidence_provenance_inputs_and_full_risk_catalog() {
    let root = tempfile::tempdir().unwrap();
    let source = "Źródło Unicode 🦀\\n literal escape\nactual newline\t\"quoted\"".repeat(20);
    std::fs::write(root.path().join("source"), &source).unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    spec.input_files = vec!["source".into()];
    let mut run = Run::new(architect::id(), spec).unwrap();
    run.stage = Stage::Planning;
    run.attempt = 2;
    let content = format!(
        "EXACT_ARTIFACT_SENTINEL {}",
        "escaped\\\" Unicodeąć 🦀\n".repeat(200)
    );
    let artifact = Artifact {
        id: architect::id(),
        stage: run.stage,
        attempt: run.attempt,
        spec_revision: run.spec_revision,
        session_id: "executor".into(),
        hash: architect::hash(&content),
        content: content.clone(),
    };
    run.artifacts.push(artifact.clone());
    let evidence = verification::observe(&artifact);
    run.evidence.push(evidence.clone());
    let snapshot = serde_json::to_string(&run.input_snapshot).unwrap();
    for n in 0..3 {
        run.evidence.push(Evidence {
            id: format!("snapshot-{n}"),
            artifact_hash: artifact.hash.clone(),
            spec_revision: run.spec_revision,
            input_hash: Some(architect::hash(&snapshot)),
            source: "runtime:input-snapshot".into(),
            output_hash: architect::hash(&snapshot),
            output: snapshot.clone(),
            success: true,
        });
    }
    run.provenance.insert(
        "artifact-node".into(),
        ProvenanceNode {
            kind: ProvenanceKind::Artifact,
            spec_revision: run.spec_revision,
            content_hash: artifact.hash.clone(),
            content: content.clone(),
            source_hashes: run.source_hashes(),
            dependencies: BTreeMap::new(),
            valid: true,
            revalidation: None,
            superseded_by: None,
            refresh: false,
        },
    );
    let historical = architect::orchestration::observe(&run, &architect::hash("preturn")).unwrap();
    run.evidence.push(historical.clone());
    let current = architect::orchestration::observe(&run, &artifact.hash).unwrap();
    run.evidence.push(current.clone());
    let provider = providers::build("fake").unwrap();
    let request = provider
        .prepare_verify(
            &ObservedInvocation {
                turn_id: "verify",
                run: &run,
                resume: None,
                context_packet: None,
            },
            &artifact,
            &evidence,
        )
        .unwrap();
    let registry = registry(&request.prompt);
    assert_eq!(
        section(&registry, "Pinned artifact"),
        serde_json::to_value(&artifact).unwrap()
    );
    assert_eq!(
        section(&registry, "Artifact read receipt"),
        serde_json::to_value(&evidence).unwrap()
    );
    for artifact in section(&registry, "Prior artifacts").as_array().unwrap() {
        assert!(
            serde_json::to_value(&run.artifacts)
                .unwrap()
                .as_array()
                .unwrap()
                .contains(artifact)
        );
    }
    let graph = section(&registry, "Dependency graph");
    for (id, node) in graph.as_object().unwrap() {
        assert_eq!(node, &serde_json::to_value(&run.provenance[id]).unwrap());
    }
    for check in run.expected_checks() {
        if let Some(id) = check.strip_prefix("REVALIDATE:") {
            assert!(graph.get(id).is_some());
        }
    }
    assert_eq!(
        section(&registry, "Current pinned files"),
        serde_json::to_value(&run.input_snapshot).unwrap()
    );
    assert_eq!(
        section(&registry, "Run specification"),
        serde_json::to_value(&run.spec).unwrap()
    );
    assert_eq!(
        section(&registry, "Full risk catalog"),
        serde_json::from_str::<Value>(include_str!("../fixtures/risks.json")).unwrap()
    );
    assert_eq!(
        section(&registry, "Machine-owned orchestration context"),
        architect::orchestration::context(&run).unwrap()
    );
    assert_eq!(
        section(&registry, "Historical orchestration snapshots"),
        json!([historical])
    );
    let expected: Vec<_> = run
        .evidence
        .iter()
        .filter(|e| verification::evidence_is_current(&run, &artifact, e))
        .collect();
    let current_receipts = section(&registry, "Current receipts");
    for receipt in current_receipts.as_array().unwrap() {
        assert!(
            serde_json::to_value(&expected)
                .unwrap()
                .as_array()
                .unwrap()
                .contains(receipt)
        );
    }
    assert!(
        current_receipts
            .as_array()
            .unwrap()
            .contains(&serde_json::to_value(&evidence).unwrap())
    );
    let ids=request.schema["properties"]["checks"]["items"]["properties"]["evidence_ids"]["items"]["enum"].as_array().unwrap();
    assert!(ids.contains(&json!(current.id)));
    assert!(!ids.contains(&json!(historical.id)));
    let docs = registry["documents"].as_object().unwrap();
    assert_eq!(
        docs.keys()
            .filter(|id| raw_document(&registry, id) == content)
            .count(),
        1
    );
    assert_eq!(
        docs.keys()
            .filter(|id| raw_document(&registry, id) == snapshot)
            .count(),
        1
    );
    assert_eq!(
        docs.keys()
            .filter(|id| raw_document(&registry, id) == current.output)
            .count(),
        1
    );
    assert_eq!(request.prompt.matches("EXACT_ARTIFACT_SENTINEL").count(), 1);
    for id in docs.keys() {
        raw_document(&registry, id);
    }
    // Every presentation reference resolves; no raw body is omitted in favor
    // of a document that would require a tool call unavailable to the model.
    resolve(&registry["sections"], &registry);
}

#[test]
fn canonical_string_parts_restore_exact_tokens_without_normalizing_other_json() {
    let root = tempfile::tempdir().unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    let mut run = Run::new(architect::id(), spec).unwrap();
    let program = "print(\"Źródło 🦀\")\n\t".repeat(100);
    let nested = serde_json::to_string(&json!({"program": program})).unwrap();
    let content = format!(
        r#"{{ "nested": {}, "noncanonical": "{}", "number": 1.200e+03 }}"#,
        serde_json::to_string(&nested).unwrap(),
        r"\u0041".repeat(300)
    );
    let artifact = Artifact {
        id: architect::id(),
        stage: run.stage,
        attempt: run.attempt,
        spec_revision: run.spec_revision,
        session_id: "executor".into(),
        hash: architect::hash(&content),
        content: content.clone(),
    };
    run.artifacts.push(artifact.clone());
    let prepared = providers::build("fake")
        .unwrap()
        .prepare_execute(&ObservedInvocation {
            turn_id: "test",
            run: &run,
            resume: None,
            context_packet: None,
        })
        .unwrap();
    let registry = registry(&prepared.prompt);
    assert_eq!(raw_document(&registry, &artifact.hash), content);
    assert_eq!(raw_document(&registry, &architect::hash(&program)), program);
    assert_eq!(raw_document(&registry, &architect::hash(&nested)), nested);
    assert!(
        !registry["documents"]
            .as_object()
            .unwrap()
            .values()
            .any(|doc| doc.get("sha256").unwrap_or(&doc[0]) == &architect::hash("A".repeat(300)))
    );
    assert_eq!(section(&registry, "Previous artifacts"), json!([artifact]));
}

#[test]
fn role_na_instructions_match_existing_validator_exceptions_without_waiving_gates() {
    let root = tempfile::tempdir().unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    let mut run = Run::new(architect::id(), spec).unwrap();
    run.stage = Stage::Execution;
    let provider = providers::build("fake").unwrap();
    let request = provider
        .prepare_execute(&ObservedInvocation {
            turn_id: "test",
            run: &run,
            resume: None,
            context_packet: None,
        })
        .unwrap();
    assert!(architect::quality::allows_not_applicable(
        "ROLE:performance:baseline"
    ));
    assert!(architect::quality::allows_not_applicable(
        "ROLE:security:authentication"
    ));
    assert!(!architect::quality::allows_not_applicable(
        "ROLE:code:input_contracts"
    ));
    assert!(!architect::quality::allows_not_applicable(
        "GATE:refactor_complete:delivery"
    ));
    assert!(request.prompt.contains("All GATE checks and all ROLE checks other than ROLE:performance:* and ROLE:security:* require pass"));
    assert!(
        request
            .prompt
            .contains("Do not waive a material performance/security risk")
    );
    assert!(
        !request
            .prompt
            .contains("Mandatory ROLE/GATE checks cannot be N/A")
    );
}

#[test]
fn exported_real_run_prompt_metrics_when_explicitly_requested() {
    let Some(path) = std::env::var_os("ANTISLOP_PROMPT_RUN_JSON") else {
        return;
    };
    let run: Run = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let artifact = run.artifacts.last().unwrap();
    let evidence = run
        .evidence
        .iter()
        .find(|e| e.source == "runtime:artifact-read" && e.artifact_hash == artifact.hash)
        .unwrap();
    let provider = providers::build("fake").unwrap();
    let request = provider
        .prepare_verify(
            &ObservedInvocation {
                turn_id: "measure",
                run: &run,
                resume: None,
                context_packet: None,
            },
            artifact,
            evidence,
        )
        .unwrap();
    if let Some(path) = std::env::var_os("ANTISLOP_PROMPT_OUTPUT") {
        std::fs::write(&path, &request.prompt).unwrap();
        std::fs::write(
            std::path::PathBuf::from(path).with_extension("schema.json"),
            serde_json::to_vec(&request.schema).unwrap(),
        )
        .unwrap();
    }
    println!("prompt_unicode_scalars={}", request.prompt.chars().count());
    println!(
        "prompt_sha256={} schema_sha256={}",
        request.prompt_hash(),
        request.schema_hash().unwrap()
    );
    println!(
        "actual_pinned_artifact_prompt_bytes={} schema_bytes={} total={}",
        request.prompt.len(),
        serde_json::to_vec(&request.schema).unwrap().len(),
        request.prompt.len() + serde_json::to_vec(&request.schema).unwrap().len()
    );
    let registry = registry(&request.prompt);
    for (name, value) in registry["sections"].as_object().unwrap() {
        println!(
            "section bytes={} name={name}",
            serde_json::to_vec(value).unwrap().len()
        );
    }
    let mut encoded_sizes: Vec<_> = registry["documents"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(id, doc)| (serde_json::to_vec(doc).unwrap().len(), id))
        .collect();
    encoded_sizes.sort_by_key(|row| std::cmp::Reverse(row.0));
    for (bytes, id) in encoded_sizes.iter().take(12) {
        println!("encoded_doc bytes={bytes} hash={id}");
    }
    let mut sizes: Vec<_> = registry["documents"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(id, _document)| {
            let raw = raw_document(&registry, id);
            let kind = serde_json::from_str::<Value>(&raw)
                .ok()
                .map(|value| match value {
                    Value::Object(object) => object.keys().cloned().collect::<Vec<_>>().join(","),
                    Value::Array(array) => format!("array:{}", array.len()),
                    _ => "json scalar".into(),
                })
                .unwrap_or_else(|| "plain UTF8".into());
            (raw.len(), id, kind)
        })
        .collect();
    sizes.sort_by_key(|row| std::cmp::Reverse(row.0));
    for (bytes, id, kind) in sizes.iter().take(15) {
        println!("doc bytes={bytes} hash={id} kind={kind}");
    }
    assert_eq!(
        section(&registry, "Pinned artifact"),
        serde_json::to_value(artifact).unwrap()
    );
    let graph = section(&registry, "Dependency graph");
    for (id, node) in graph.as_object().unwrap() {
        assert_eq!(node, &serde_json::to_value(&run.provenance[id]).unwrap());
    }
    for check in run.expected_checks() {
        if let Some(id) = check.strip_prefix("REVALIDATE:") {
            assert!(graph.get(id).is_some());
        }
    }
    let wanted: Vec<_> = run
        .evidence
        .iter()
        .filter(|e| verification::evidence_is_current(&run, artifact, e))
        .collect();
    let current = section(&registry, "Current receipts");
    for receipt in current.as_array().unwrap() {
        assert!(
            serde_json::to_value(&wanted)
                .unwrap()
                .as_array()
                .unwrap()
                .contains(receipt)
        );
    }
    let actual: std::collections::BTreeSet<_> =
        request.schema["properties"]["checks"]["items"]["properties"]["check_id"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
    assert_eq!(actual, run.expected_checks());
    for artifact in section(&registry, "Prior artifacts").as_array().unwrap() {
        assert!(
            serde_json::to_value(&run.artifacts)
                .unwrap()
                .as_array()
                .unwrap()
                .contains(artifact)
        );
    }
}

#[test]
fn proof_closure_preserves_required_old_artifact_and_cited_historical_body() {
    let root = tempfile::tempdir().unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    let mut run = Run::new(architect::id(), spec).unwrap();
    run.stage = Stage::Execution;
    run.attempt = 3;
    let historic_id = architect::id();
    for (attempt, content) in [
        (1, "REQUIRED_OLD_ARTIFACT".into()),
        (2, "UNREFERENCED_REJECTED_ARTIFACT".into()),
        (
            3,
            format!("Inspect historical receipt {historic_id} as history"),
        ),
    ] {
        run.artifacts.push(Artifact {
            id: architect::id(),
            stage: Stage::Execution,
            attempt,
            spec_revision: run.spec_revision,
            session_id: format!("executor-{attempt}"),
            hash: architect::hash(&content),
            content,
        });
    }
    let old = run.artifacts[0].clone();
    let artifact = run.artifacts.last().unwrap().clone();
    let node_id = format!("artifact:{}", old.id);
    run.provenance.insert(
        node_id.clone(),
        ProvenanceNode {
            kind: ProvenanceKind::Artifact,
            spec_revision: run.spec_revision,
            content_hash: old.hash.clone(),
            content: old.content.clone(),
            source_hashes: BTreeMap::from([("objective".into(), "stale".into())]),
            dependencies: BTreeMap::new(),
            valid: false,
            revalidation: None,
            superseded_by: None,
            refresh: false,
        },
    );
    let historical = Evidence {
        id: historic_id.clone(),
        artifact_hash: old.hash.clone(),
        spec_revision: run.spec_revision,
        input_hash: Some("old-input".into()),
        source: "runtime:historical-observation".into(),
        output_hash: architect::hash("ACTUAL_HISTORICAL_BODY"),
        output: "ACTUAL_HISTORICAL_BODY".into(),
        success: true,
    };
    run.evidence.push(historical.clone());
    let receipt = verification::observe(&artifact);
    run.evidence.push(receipt.clone());
    let snapshot = serde_json::to_string(&run.input_snapshot).unwrap();
    let snapshot_hash = architect::hash(&snapshot);
    run.evidence.push(Evidence {
        id: architect::id(),
        artifact_hash: artifact.hash.clone(),
        spec_revision: run.spec_revision,
        input_hash: Some(snapshot_hash.clone()),
        source: "runtime:input-snapshot".into(),
        output_hash: snapshot_hash.clone(),
        output: snapshot,
        success: true,
    });
    let failed = Evidence {
        id: architect::id(),
        artifact_hash: artifact.hash.clone(),
        spec_revision: run.spec_revision,
        input_hash: Some(snapshot_hash),
        source: "runner:[\"uncited-failed-check\"]".into(),
        output_hash: architect::hash("ACTUAL_UNCITED_FAILURE"),
        output: "ACTUAL_UNCITED_FAILURE".into(),
        success: false,
    };
    run.evidence.push(failed.clone());
    let expected = run.expected_checks();
    assert!(expected.contains(&format!("REVALIDATE:{node_id}")));
    let request = providers::build("fake")
        .unwrap()
        .prepare_verify(
            &ObservedInvocation {
                turn_id: "test",
                run: &run,
                resume: None,
                context_packet: None,
            },
            &artifact,
            &receipt,
        )
        .unwrap();
    let registry = registry(&request.prompt);
    assert_eq!(
        section(&registry, "Prior artifacts"),
        json!([old, artifact])
    );
    assert_eq!(
        section(&registry, "Historical proof receipts"),
        json!([historical])
    );
    assert_eq!(
        section(&registry, "Dependency graph")[&node_id],
        serde_json::to_value(&run.provenance[&node_id]).unwrap()
    );
    assert!(!request.prompt.contains("UNREFERENCED_REJECTED_ARTIFACT"));
    let checks: std::collections::BTreeSet<_> =
        request.schema["properties"]["checks"]["items"]["properties"]["check_id"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect();
    assert_eq!(checks, expected);
    let ids=request.schema["properties"]["checks"]["items"]["properties"]["evidence_ids"]["items"]["enum"].as_array().unwrap();
    assert!(!ids.contains(&json!(historic_id)));
    assert!(ids.contains(&json!(receipt.id)));
    assert!(ids.contains(&json!(failed.id)));
    assert!(
        section(&registry, "Current receipts")
            .as_array()
            .unwrap()
            .contains(&serde_json::to_value(failed).unwrap())
    );
    assert!(request.prompt.contains("ACTUAL_UNCITED_FAILURE"));
}

#[test]
fn historical_diagnostic_remains_exact_with_separate_current_binding_observation() {
    let root = tempfile::tempdir().unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    let mut run = Run::new(architect::id(), spec).unwrap();
    run.stage = Stage::Delivery;
    run.attempt = 12;
    let text = "Prior rejection: \"missing risk\"\nŹródło 🧠 \\ exact";
    run.diagnostic = Some(text.into());
    let before = serde_json::to_vec(&run).unwrap();
    let prepared = providers::build("fake")
        .unwrap()
        .prepare_execute(&ObservedInvocation {
            turn_id: "diagnostic-scope",
            run: &run,
            resume: None,
            context_packet: None,
        })
        .unwrap();
    let registry = registry(&prepared.prompt);
    assert_eq!(
        section(&registry, "Historical runtime diagnostic"),
        json!(text)
    );
    assert!(
        registry["sections"]
            .get("Last runtime diagnostic")
            .is_none()
    );
    let context = section(&registry, "Machine-owned orchestration context");
    let summary = verification::prior_risk_binding_summary(&run);
    assert_eq!(
        context["prior_risk_binding_status"],
        serde_json::to_value(&summary).unwrap()
    );
    assert!(
        !summary.all_checked_prior_bindings_current,
        "a retained diagnostic is not treated as repaired without actual binding proof"
    );
    assert!(!summary.stale_or_missing_prior_risk_ids.is_empty());
    assert!(
        prepared
            .prompt
            .contains("retained text alone does not establish a current defect")
    );
    assert!(
        prepared
            .prompt
            .contains("does not establish quality acceptance")
    );
    assert_eq!(serde_json::to_vec(&run).unwrap(), before);
}
