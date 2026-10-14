//! Independent checks of lossless inline context and evidence eligibility.
use architect::{
    domain::*,
    hash, orchestration,
    providers::{ObservedInvocation, Provider, fake::FakeProvider},
    verification,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
// Independent canonical JSON string encoder, rather than the production encoder.
fn json_string(raw: &str) -> String {
    let mut encoded = String::from("\"");
    for character in raw.chars() {
        match character {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '\u{8}' => encoded.push_str("\\b"),
            '\u{c}' => encoded.push_str("\\f"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            '\u{0}'..='\u{1f}' => encoded.push_str(&format!("\\u{:04x}", character as u32)),
            other => encoded.push(other),
        }
    }
    encoded.push('"');
    encoded
}
fn registry(prompt: &str) -> Value {
    let encoded = prompt
        .split_once("\nBEGIN INLINE CONTEXT REGISTRY\n")
        .unwrap()
        .1
        .split_once("\nEND INLINE CONTEXT REGISTRY")
        .unwrap()
        .0;
    let mut registry: Value = serde_json::from_str(encoded).unwrap();
    if registry["format"] == "inline-exact-utf8-v7" {
        // Normalize only the manufactured document table for the existing
        // independent decoder. Never walk original JSON restored from a body.
        for document in registry["documents"].as_object_mut().unwrap().values_mut() {
            let tuple = document.as_array().expect("v7 document must be a tuple");
            assert_eq!(
                tuple.len(),
                3,
                "v7 tuple has exactly digest, byte length, body"
            );
            let digest = tuple[0].as_str().expect("tuple digest must be a string");
            assert!(
                digest.len() == 64
                    && digest
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            );
            let bytes = tuple[1]
                .as_u64()
                .expect("tuple byte length must be a nonnegative integer");
            let representation = tuple[2].clone();
            *document = if representation.is_string() {
                json!({"sha256":digest,"utf8_bytes":bytes,"utf8":representation})
            } else {
                assert!(
                    representation.is_array(),
                    "tuple body must be literal string or parts array"
                );
                json!({"sha256":digest,"utf8_bytes":bytes,"parts":representation})
            };
        }
    } else {
        assert!(
            matches!(
                registry["format"].as_str(),
                Some("inline-exact-utf8-v5" | "inline-exact-utf8-v6")
            ),
            "unknown registry grammar"
        );
    }
    let mut hashes: Vec<_> = registry["documents"]
        .as_object()
        .unwrap()
        .iter()
        .map(|(alias, doc)| (doc["sha256"].as_str().unwrap(), alias.as_str()))
        .collect();
    hashes.sort_unstable();
    for (index, (_, alias)) in hashes.iter().enumerate() {
        assert_eq!(
            *alias,
            format!("d{}", index + 1),
            "aliases are deterministic SHA order"
        );
    }
    // Inspect every manufactured metadata reference, without recursively
    // interpreting literal objects restored across json_value_ref boundaries.
    resolve(&registry["sections"], &registry);
    registry
}
fn document_bytes(id: &str, registry: &Value, visiting: &mut BTreeSet<String>) -> String {
    assert!(
        visiting.insert(id.into()),
        "cyclic inline fragment reference {id}"
    );
    let document = registry["documents"]
        .get(id)
        .expect("every fragment must be inline");
    assert_ne!(
        document.get("utf8").is_some(),
        document.get("parts").is_some(),
        "a document must have exactly one representation"
    );
    let body = if let Some(raw) = document.get("utf8") {
        raw.as_str().unwrap().to_string()
    } else {
        let mut body = String::new();
        for part in document["parts"].as_array().unwrap() {
            if let Some(literal) = part.as_str() {
                body.push_str(literal);
            } else if let Some(literal) = part.get("utf8") {
                assert_eq!(
                    registry["format"], "inline-exact-utf8-v5",
                    "v6 literal parts must be strings"
                );
                assert_eq!(part.as_object().unwrap().len(), 1);
                body.push_str(literal.as_str().unwrap());
            } else {
                let (child, quoted) =
                    part_reference(part).expect("valid raw or quoted fragment reference");
                if part.is_object() {
                    assert_eq!(
                        registry["format"], "inline-exact-utf8-v5",
                        "v6 reference parts must be integers"
                    );
                    assert_eq!(part.as_object().unwrap().len(), 1);
                }
                assert!(
                    registry["documents"][&child]["utf8_bytes"]
                        .as_u64()
                        .expect("missing child")
                        < document["utf8_bytes"].as_u64().unwrap(),
                    "every fragment edge must strictly decrease byte length"
                );
                let bytes = document_bytes(&child, registry, visiting);
                body.push_str(&if quoted { json_string(&bytes) } else { bytes });
            }
        }
        body
    };
    assert_eq!(hash(&body), document["sha256"]);
    assert_eq!(document["utf8_bytes"], body.len());
    visiting.remove(id);
    body
}
fn part_reference(part: &Value) -> Option<(String, bool)> {
    if part.is_number() {
        let index = part
            .as_i64()
            .expect("fragment reference must be a signed integer");
        assert_ne!(index, 0, "zero is not a fragment reference");
        return Some((format!("d{}", index.unsigned_abs()), index < 0));
    }
    part.get("document_ref")
        .and_then(Value::as_str)
        .map(|reference| (reference.into(), false))
        .or_else(|| {
            part.get("json_string_ref")
                .and_then(Value::as_str)
                .map(|reference| (reference.into(), true))
        })
}
fn body(id: &str, registry: &Value) -> String {
    let alias = if registry["documents"].get(id).is_some() {
        id
    } else {
        alias(id, registry)
    };
    document_bytes(alias, registry, &mut BTreeSet::new())
}
fn alias<'a>(digest: &str, registry: &'a Value) -> &'a str {
    registry["documents"]
        .as_object()
        .unwrap()
        .iter()
        .find(|(_, doc)| doc["sha256"] == digest)
        .expect("exact byte hash must have an inline document")
        .0
}
fn resolve(value: &Value, registry: &Value) -> Value {
    if let Some(reference) = value
        .as_object()
        .filter(|o| o.len() == 1)
        .and_then(|o| o.get("json_value_ref"))
        .and_then(Value::as_str)
    {
        // Decode once. Literal wrapper-shaped input is data after this boundary.
        assert!(
            registry["documents"].get(reference).is_some(),
            "manufactured typed reference must use a local alias"
        );
        return serde_json::from_str(&body(reference, registry)).unwrap();
    }

    if let Some(reference) = value
        .as_object()
        .filter(|object| object.len() == 1)
        .and_then(|o| o.get("document_ref"))
        .and_then(Value::as_str)
    {
        assert!(
            registry["documents"].get(reference).is_some(),
            "manufactured document reference must use a local alias"
        );
        return Value::String(body(reference, registry));
    }
    match value {
        Value::Array(items) => Value::Array(items.iter().map(|v| resolve(v, registry)).collect()),
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), resolve(v, registry)))
                .collect(),
        ),
        _ => value.clone(),
    }
}
// Check authenticity independently rather than treating matching source hashes
// alone as proof that an old passing gate is still usable.
fn authentic_gate(run: &Run, gate: &architect::quality::GateAssessment) -> bool {
    let Some(review) = run
        .reviews
        .iter()
        .find(|review| hash(serde_json::to_vec(review).unwrap()) == gate.review_hash)
    else {
        return false;
    };
    let Some(report) = &review.quality else {
        return false;
    };
    let Some(artifact) = run
        .artifacts
        .iter()
        .find(|artifact| artifact.hash == gate.artifact_hash)
    else {
        return false;
    };
    let Some(check) = review
        .checks
        .iter()
        .find(|check| check.check_id == gate.check_id)
    else {
        return false;
    };
    gate.spec_revision == run.spec_revision
        && gate.source_hashes == run.source_hashes()
        && gate.status == CheckStatus::Pass
        && hash(serde_json::to_vec(report).unwrap()) == gate.report_hash
        && review.artifact_hash == artifact.hash
        && hash(&artifact.content) == artifact.hash
        && review.spec_revision == run.spec_revision
        && review.checklist_version == run.checklist_version
        && review.executor_session == artifact.session_id
        && !review.verifier_session.trim().is_empty()
        && review.verifier_session != review.executor_session
        && review.findings.is_empty()
        && check.status == CheckStatus::Pass
        && check.check_id == architect::quality::gate_check_id(&gate.gate_id, gate.phase)
        && check.evidence_ids == gate.evidence_ids
        && !check.evidence_ids.is_empty()
        && check.evidence_ids.iter().all(|id| {
            run.evidence.iter().any(|e| {
                e.id == *id
                    && e.success
                    && hash(&e.output) == e.output_hash
                    && verification::evidence_is_current(run, artifact, e)
            })
        })
}

fn proof_review_rows(
    run: &Run,
    graph: &std::collections::BTreeMap<String, ProvenanceNode>,
    reviews: &std::collections::BTreeMap<String, Review>,
    registry: &Value,
) -> std::collections::BTreeMap<String, BTreeSet<String>> {
    use std::collections::BTreeMap;
    let declared: BTreeMap<String, BTreeSet<String>> = serde_json::from_value(resolve(
        &registry["sections"]["Current proof review check IDs"],
        registry,
    ))
    .expect("explicit proof-row declaration required");
    let mut required = BTreeMap::<String, BTreeSet<String>>::new();
    for (id, node) in graph {
        if let Some(binding) = &node.revalidation {
            required
                .entry(binding.review_hash.clone())
                .or_default()
                .insert(format!("REVALIDATE:{id}"));
        }
        if node.kind == ProvenanceKind::RiskAssessment {
            let check: CheckResult = serde_json::from_str(&node.content)
                .expect("actual risk assessment CheckResult body");
            for dependency in node.dependencies.keys() {
                if let Some(review) = dependency.strip_prefix("review:") {
                    required
                        .entry(review.into())
                        .or_default()
                        .insert(check.check_id.clone());
                }
            }
        }
    }
    let sources = run.source_hashes();
    for gate in run
        .quality_gates
        .values()
        .filter(|gate| authentic_gate(run, gate))
    {
        required
            .entry(gate.review_hash.clone())
            .or_default()
            .insert(gate.check_id.clone());
    }
    let validity = verification::provenance_validity(run);
    for entry in run.risk_coverage.values() {
        if let Some(binding) = &entry.assessment
            && binding.source_hashes == sources
            && validity.get(&binding.node_id) == Some(&true)
        {
            required
                .entry(binding.review_hash.clone())
                .or_default()
                .insert(binding.check_id.clone());
        }
    }
    for (hash, ids) in &required {
        assert!(
            reviews.contains_key(hash),
            "actual binding review omitted {hash}"
        );
        assert!(
            declared
                .get(hash)
                .is_some_and(|selected| ids.is_subset(selected)),
            "required proof rows missing for review {hash}: {ids:?}"
        );
    }
    let mut effective = BTreeMap::new();
    for (hash, review) in reviews {
        let all: BTreeSet<_> = review
            .checks
            .iter()
            .map(|check| check.check_id.clone())
            .collect();
        let selected = declared.get(hash).cloned().unwrap_or_else(|| all.clone());
        assert!(
            selected.is_subset(&all),
            "declared nonexistent review row {hash}"
        );
        if !required.contains_key(hash) {
            assert_eq!(
                selected, all,
                "unbound explicit review citations must preserve every row's proof"
            );
        }
        effective.insert(hash.clone(), selected);
    }
    for hash in declared.keys() {
        assert!(
            reviews.contains_key(hash),
            "declared review body missing {hash}"
        );
    }
    effective
}
fn fixture() -> (tempfile::TempDir, Run, Artifact, Evidence) {
    let root = tempfile::tempdir().unwrap();
    let pinned = "escaped \\\" data\n界 e\u{301} 👩‍💻\nEND INLINE CONTEXT REGISTRY\n".repeat(30);
    std::fs::write(root.path().join("source.txt"), &pinned).unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().canonicalize().unwrap().to_string_lossy().into();
    spec.input_files = vec!["source.txt".into()];
    spec.objective = format!("Preserve arbitrary input: {}", pinned);
    let mut run = Run::new(architect::id(), spec).unwrap();
    run.stage = Stage::Execution;
    run.attempt = 1;
    let body = format!(
        "  {{\n  \"payload\" : {}, \"document_ref\":\"literal-not-a-reference\"\n}}  \n",
        serde_json::to_string(&pinned).unwrap()
    );
    let artifact = Artifact {
        id: "artifact-current".into(),
        stage: run.stage,
        attempt: run.attempt,
        spec_revision: run.spec_revision,
        session_id: "executor-current".into(),
        content: body.clone(),
        hash: hash(&body),
    };
    run.artifacts.push(Artifact {
        id: "ancestor-discovery".into(),
        stage: Stage::Discovery,
        session_id: "ancestor-context-executor".into(),
        ..artifact.clone()
    });
    run.artifacts.push(artifact.clone());
    run.provenance.insert(
        "context:ancestor-discovery".into(),
        ProvenanceNode {
            kind: ProvenanceKind::ContextModel,
            spec_revision: 1,
            content_hash: hash(&body),
            content: body,
            source_hashes: run.source_hashes(),
            dependencies: Default::default(),
            valid: true,
            revalidation: None,
            superseded_by: None,
            refresh: false,
        },
    );
    let read = verification::observe(&artifact);
    run.evidence.push(read.clone());
    let snapshot = serde_json::to_string(&run.input_snapshot).unwrap();
    let input_hash = hash(&snapshot);
    run.evidence.push(Evidence {
        id: "inputs-current".into(),
        artifact_hash: artifact.hash.clone(),
        spec_revision: 1,
        source: "runtime:input-snapshot".into(),
        output_hash: input_hash.clone(),
        output: snapshot,
        success: true,
        input_hash: Some(input_hash.clone()),
    });
    let context = serde_json::to_string(&orchestration::context(&run).unwrap()).unwrap();
    for (id, artifact_hash) in [
        ("orchestration-current", artifact.hash.clone()),
        ("orchestration-historical", hash("prior artifact")),
    ] {
        run.evidence.push(Evidence {
            id: id.into(),
            artifact_hash,
            spec_revision: 1,
            source: "runtime:orchestration".into(),
            output_hash: hash(&context),
            output: context.clone(),
            success: true,
            input_hash: Some(input_hash.clone()),
        });
    }
    (root, run, artifact, read)
}
#[test]
fn raw_utf8_bodies_and_long_metadata_roundtrip_without_rehashing_or_mutating_run() {
    let (_root, run, artifact, read) = fixture();
    let before = serde_json::to_value(&run).unwrap();
    let provider = FakeProvider;
    let invocation = ObservedInvocation {
        turn_id: "review",
        run: &run,
        resume: None,
        context_packet: None,
    };
    for prepared in [
        provider.prepare_execute(&invocation).unwrap(),
        provider
            .prepare_verify(&invocation, &artifact, &read)
            .unwrap(),
    ] {
        let registry = registry(&prepared.prompt);
        assert!(matches!(
            registry["format"].as_str(),
            Some("inline-exact-utf8-v6" | "inline-exact-utf8-v7")
        ));
        let sections = &registry["sections"];
        assert_eq!(
            resolve(&sections["Run specification"], &registry),
            serde_json::to_value(&run.spec).unwrap()
        );
        let artifacts = sections
            .get("Prior artifacts")
            .or_else(|| sections.get("Previous artifacts"))
            .unwrap();
        assert_eq!(
            resolve(artifacts, &registry),
            serde_json::to_value(&run.artifacts).unwrap()
        );
        let pinned = sections
            .get("Current pinned files")
            .or_else(|| sections.get("Pinned inputs"))
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(resolve(pinned, &registry).as_str().unwrap()).unwrap(),
            serde_json::to_value(&run.input_snapshot).unwrap()
        );
        let documents = registry["documents"].as_object().unwrap();
        let reconstructed: Vec<_> = documents.keys().map(|id| body(id, &registry)).collect();
        assert_eq!(body(&artifact.hash, &registry), artifact.content);
        assert_eq!(
            reconstructed
                .iter()
                .filter(|raw| **raw == artifact.content)
                .count(),
            1,
            "artifact repeated in evidence/provenance must have one exact body"
        );
        if let Some(graph) = sections.get("Dependency graph") {
            assert_eq!(
                resolve(graph, &registry),
                serde_json::to_value(&run.provenance).unwrap()
            );
        }
    }
    assert_eq!(
        serde_json::to_value(&run).unwrap(),
        before,
        "prompt rendering must never mutate evidence or hashes"
    );
}
#[test]
fn current_and_historical_receipts_remain_distinct_and_only_current_ids_can_be_cited() {
    let (_root, run, artifact, read) = fixture();
    let prepared = FakeProvider
        .prepare_verify(
            &ObservedInvocation {
                turn_id: "review",
                run: &run,
                resume: None,
                context_packet: None,
            },
            &artifact,
            &read,
        )
        .unwrap();
    let registry = registry(&prepared.prompt);
    let sections = &registry["sections"];
    let expected: Vec<_> = run
        .evidence
        .iter()
        .filter(|e| verification::evidence_is_current(&run, &artifact, e))
        .collect();
    assert_eq!(
        resolve(&sections["Current receipts"], &registry),
        serde_json::to_value(&expected).unwrap()
    );
    let current = resolve(&sections["Current receipts"], &registry);
    let historical = resolve(&sections["Historical orchestration snapshots"], &registry);
    assert!(
        current
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["id"] == "orchestration-current")
    );
    assert!(
        !current
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["id"] == "orchestration-historical")
    );
    assert_eq!(historical.as_array().unwrap().len(), 1);
    assert_eq!(historical[0]["id"], "orchestration-historical");
    assert_eq!(
        current
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == "orchestration-current")
            .unwrap()["output"],
        historical[0]["output"],
        "even identical bodies preserve temporal receipt identity"
    );
    let ids = prepared
        .schema
        .pointer("/properties/checks/items/properties/evidence_ids/items/enum")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(ids, expected.iter().map(|e| e.id.as_str()).collect());
    assert!(!ids.contains("orchestration-historical"));
    assert_eq!(
        resolve(&sections["Artifact read receipt"], &registry),
        serde_json::to_value(&read).unwrap()
    );
    assert_eq!(
        resolve(&sections["Pinned artifact"], &registry),
        serde_json::to_value(&artifact).unwrap()
    );
    assert!(
        prepared
            .prompt
            .contains("These presentation references are NOT evidence IDs")
    );
    assert_eq!(
        resolve(&sections["Expected IDs"], &registry),
        json!(run.expected_checks())
    );
}

#[test]
fn repeated_nested_json_fragments_preserve_noncanonical_bytes_and_all_children_resolve() {
    let (_root, mut run, mut artifact, _read) = fixture();
    let token = format!(
        "{{\"raw\":\"{}\", \"number\":1e+03, \"negative\":-0, \"escaped\":\"\\u0061\\/\\t\" }}",
        "界 e\\\"".repeat(200)
    );
    let raw =
        format!(" \r\n{{\"left\":{token},\"right\":{token},\"ordered\" : [ {token}, {token} ]}}\n");
    assert!(serde_json::from_str::<Value>(&raw).is_ok());
    artifact.content = raw.clone();
    artifact.hash = hash(&raw);
    run.artifacts = vec![artifact.clone()];
    run.provenance.clear();
    run.evidence.clear();
    let read = verification::observe(&artifact);
    run.evidence.push(read.clone());
    let prepared = FakeProvider
        .prepare_verify(
            &ObservedInvocation {
                turn_id: "fragments",
                run: &run,
                resume: None,
                context_packet: None,
            },
            &artifact,
            &read,
        )
        .unwrap();
    let registry = registry(&prepared.prompt);
    let documents = registry["documents"].as_object().unwrap();
    for id in documents.keys() {
        body(id, &registry);
    }
    assert!(
        documents.values().any(|d| d.get("parts").is_some()),
        "repeated JSON token bytes should produce fragment parts"
    );
    assert_eq!(
        body(&artifact.hash, &registry),
        raw,
        "raw whitespace, numeric spelling, escapes and Unicode must not normalize"
    );
    assert_eq!(
        resolve(&registry["sections"]["Pinned artifact"], &registry),
        serde_json::to_value(&artifact).unwrap()
    );
    let encoded_body = &documents[alias(&artifact.hash, &registry)];
    assert!(encoded_body.get("parts").is_some());
    assert!(
        documents.values().any(|doc| doc["sha256"] == hash(&token)),
        "repeated object token must have an available standalone fragment"
    );
    assert_eq!(body(&hash(&token), &registry), token);
    let token_id = alias(&hash(&token), &registry);
    let references = documents
        .values()
        .flat_map(|document| document["parts"].as_array().into_iter().flatten())
        .filter(|part| {
            part_reference(part).is_some_and(|(reference, quoted)| !quoted && reference == token_id)
        })
        .count();
    assert!(
        references >= 2,
        "exact duplicate token must be reused by reference"
    );
    assert!(
        serde_json::to_string(encoded_body).unwrap().len() < raw.len(),
        "fragment representation must reduce the repeated parent body"
    );
}
#[test]
fn malformed_json_and_plain_utf8_bodies_remain_exact_inline_documents() {
    for raw in [
        format!("prefix {{\"unfinished\": \"{}", "界\\\"".repeat(200)),
        format!("plaintext {}", "a\u{301}👩‍💻\\t\n".repeat(100)),
    ] {
        let (_root, mut run, mut artifact, _read) = fixture();
        artifact.content = raw.clone();
        artifact.hash = hash(&raw);
        run.artifacts = vec![artifact.clone()];
        run.evidence.clear();
        run.provenance.clear();
        let read = verification::observe(&artifact);
        run.evidence.push(read.clone());
        let prepared = FakeProvider
            .prepare_verify(
                &ObservedInvocation {
                    turn_id: "plain",
                    run: &run,
                    resume: None,
                    context_packet: None,
                },
                &artifact,
                &read,
            )
            .unwrap();
        let registry = registry(&prepared.prompt);
        assert_eq!(body(&artifact.hash, &registry), raw);
        for id in registry["documents"].as_object().unwrap().keys() {
            body(id, &registry);
        }
    }
}

#[test]
fn typed_source_hash_maps_and_literal_reserved_objects_roundtrip_without_type_loss() {
    for (reserved, literal) in ["document_ref", "json_value_ref", "json_string_ref"]
        .into_iter()
        .flat_map(|key| [(key, "alias"), (key, "hash")])
    {
        let (_root, mut run, artifact, read) = fixture();
        run.spec.requirements[0].criteria = std::collections::BTreeMap::from([(
            reserved.to_string(),
            if literal == "alias" {
                "d1".into()
            } else {
                artifact.hash.clone()
            },
        )]);
        let prepared = FakeProvider
            .prepare_verify(
                &ObservedInvocation {
                    turn_id: "typed",
                    run: &run,
                    resume: None,
                    context_packet: None,
                },
                &artifact,
                &read,
            )
            .unwrap();
        let registry = registry(&prepared.prompt);
        let sections = &registry["sections"];
        assert_eq!(
            resolve(&sections["Run specification"], &registry),
            serde_json::to_value(&run.spec).unwrap()
        );
        assert!(
            sections["Run specification"]
                .get("json_value_ref")
                .is_some(),
            "complete typed metadata protects literal wrapper-shaped criterion maps"
        );
        assert!(sections["Dependency graph"].get("json_value_ref").is_some());
        assert_eq!(
            resolve(&sections["Dependency graph"], &registry),
            serde_json::to_value(&run.provenance).unwrap()
        );
        let expected:Vec<_>=run.source_hashes().into_iter().map(|(source_ref,source_hash)|json!({"source_ref":source_ref,"source_hash":source_hash})).collect();
        assert_eq!(
            resolve(&sections["Quality source bindings"], &registry),
            json!(expected)
        );
        for id in registry["documents"].as_object().unwrap().keys() {
            body(id, &registry);
        }
    }
}
#[test]
fn slim_runner_metadata_keeps_the_entire_exact_argv_program_inline() {
    let (_root, mut run, mut artifact, _read) = fixture();
    let mut raw: Value = serde_json::from_str(&artifact.content).unwrap();
    raw["runner_evidence_id"] = json!("long-supplemental-runner");
    artifact.content = raw.to_string();
    artifact.hash = hash(&artifact.content);
    run.artifacts.retain(|a| a.stage != run.stage);
    run.artifacts.push(artifact.clone());
    let read = verification::observe(&artifact);
    run.evidence.push(read.clone());
    let program = format!(
        "# PROBE_PROGRAM_BODY_EXACT_SENTINEL\n{}\nprint('observations')",
        "# Unicode 界 \\\" literal\\t".repeat(100)
    );
    let arguments = vec![
        "python3".to_string(),
        "-I".into(),
        "-c".into(),
        program.clone(),
        run.spec.workspace.clone(),
    ];
    let source = format!("runner:{}", serde_json::to_string(&arguments).unwrap());
    let output=json!({"argv":arguments,"exit_code":0,"stdout":"actual observation","stderr":"","truncated":false}).to_string();
    run.evidence.push(Evidence {
        id: "long-supplemental-runner".into(),
        artifact_hash: artifact.hash.clone(),
        spec_revision: 1,
        source: source.clone(),
        output_hash: hash(&output),
        output,
        success: true,
        input_hash: Some(hash(serde_json::to_vec(&run.input_snapshot).unwrap())),
    });
    for prepared in [
        FakeProvider
            .prepare_execute(&ObservedInvocation {
                turn_id: "slim",
                run: &run,
                resume: None,
                context_packet: None,
            })
            .unwrap(),
        FakeProvider
            .prepare_verify(
                &ObservedInvocation {
                    turn_id: "slim",
                    run: &run,
                    resume: None,
                    context_packet: None,
                },
                &artifact,
                &read,
            )
            .unwrap(),
    ] {
        let prefix = prepared
            .prompt
            .split_once("\nBEGIN INLINE CONTEXT REGISTRY\n")
            .unwrap()
            .0;
        assert!(
            !prefix.contains("PROBE_PROGRAM_BODY_EXACT_SENTINEL"),
            "quality runner metadata must not duplicate large argv programs"
        );
        assert!(
            prefix.contains(&hash(&source)),
            "slim metadata must identify the exact full source"
        );
        let registry = registry(&prepared.prompt);
        let receipts = registry["sections"]
            .get("Current receipts")
            .or_else(|| registry["sections"].get("Runner evidence"))
            .unwrap();
        let receipts = resolve(receipts, &registry);
        let runner = receipts
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["id"] == "long-supplemental-runner")
            .unwrap();
        assert_eq!(runner["source"], source);
        let raw: Value = serde_json::from_str(runner["output"].as_str().unwrap()).unwrap();
        assert_eq!(raw["argv"], json!(arguments));
        assert_eq!(raw["argv"][3], program);
        assert!(
            registry
                .to_string()
                .contains("PROBE_PROGRAM_BODY_EXACT_SENTINEL"),
            "full actual program must remain inline"
        );
        for id in registry["documents"].as_object().unwrap().keys() {
            body(id, &registry);
        }
    }
}

#[test]
fn canonical_nested_strings_factor_but_noncanonical_escapes_and_runner_spacing_remain_exact() {
    let (_root, mut run, mut artifact, _read) = fixture();
    let program = format!(
        "# EXACT_DECODED_PROGRAM\n{}\n",
        "界 e\u{301} 👩‍💻 / \" \\ \0\u{8}\u{c}\t\r\n\u{1f}\u{2028}".repeat(30)
    );
    let canonical = json_string(&program);
    assert_eq!(canonical, serde_json::to_string(&program).unwrap());
    let nested = format!(" {{\"stdout\":{canonical}, \"number\":1e+03, \"negative\":-0}} \r\n");
    let noncanonical = format!("\"{}\\u0061\\/\\uD83D\\uDE00\"", "x".repeat(320));
    let raw = format!(
        " \n{{\"nested\":{}, \"legacy\":{noncanonical}, \"ordinary\":{canonical}, \"runner_receipt\":\"spaced-runner\"}}\n",
        json_string(&nested)
    );
    assert!(serde_json::from_str::<Value>(&raw).is_ok());
    artifact.content = raw.clone();
    artifact.hash = hash(&raw);
    run.artifacts = vec![artifact.clone()];
    run.evidence.clear();
    run.provenance.clear();
    let read = verification::observe(&artifact);
    run.evidence.push(read.clone());
    let spaced_argv = format!(" [ \"python3\", \"-c\", {canonical} ] \n");
    let source = format!("runner:{spaced_argv}");
    let snapshot = serde_json::to_string(&run.input_snapshot).unwrap();
    run.evidence.push(Evidence {
        id: "canonical-input-snapshot".into(),
        artifact_hash: artifact.hash.clone(),
        spec_revision: run.spec_revision,
        source: "runtime:input-snapshot".into(),
        output_hash: hash(&snapshot),
        input_hash: Some(hash(&snapshot)),
        output: snapshot,
        success: true,
    });
    run.evidence.push(Evidence {
        id: "spaced-runner".into(),
        artifact_hash: artifact.hash.clone(),
        spec_revision: run.spec_revision,
        source: source.clone(),
        output_hash: hash("observed"),
        output: "observed".into(),
        success: true,
        input_hash: Some(hash(serde_json::to_vec(&run.input_snapshot).unwrap())),
    });
    let prepared = FakeProvider
        .prepare_verify(
            &ObservedInvocation {
                turn_id: "canonical-v4",
                run: &run,
                resume: None,
                context_packet: None,
            },
            &artifact,
            &read,
        )
        .unwrap();
    let registry = registry(&prepared.prompt);
    let documents = registry["documents"].as_object().unwrap();
    for id in documents.keys() {
        body(id, &registry);
    }
    assert_eq!(body(&artifact.hash, &registry), raw);
    assert_eq!(body(&hash(&source), &registry), source);
    assert_eq!(body(&hash(&spaced_argv), &registry), spaced_argv);
    assert_eq!(
        body(&hash(&program), &registry),
        program,
        "complete decoded code must be inline"
    );
    assert_eq!(body(&hash(&nested), &registry), nested);
    let quoted_children: Vec<_> = documents
        .values()
        .flat_map(|document| document["parts"].as_array().into_iter().flatten())
        .filter_map(|part| {
            part_reference(part)
                .filter(|(_, quoted)| *quoted)
                .map(|(reference, _)| reference)
        })
        .collect();
    assert!(quoted_children.contains(&alias(&hash(&program), &registry).to_string()));
    assert!(quoted_children.contains(&alias(&hash(&nested), &registry).to_string()));
    let legacy_decoded: String = serde_json::from_str(&noncanonical).unwrap();
    assert!(
        !quoted_children
            .iter()
            .any(|child| registry["documents"][child]["sha256"] == hash(&legacy_decoded)),
        "noncanonical escape token cannot become a canonical encoding reference"
    );
    assert!(body(&artifact.hash, &registry).contains(&noncanonical));
    assert_eq!(
        resolve(&registry["sections"]["Pinned artifact"], &registry),
        serde_json::to_value(&artifact).unwrap()
    );
}

#[test]
#[ignore = "read-only audit of an explicitly supplied saved selfhost snapshot"]
fn frozen_attempt_six_proof_closure_preserves_required_history_and_gate_bodies() {
    use std::collections::{BTreeMap, BTreeSet};
    let path = std::env::var("ANTISLOP_VERIFIER_PROMPT_RUN_JSON")
        .expect("saved Run JSON fixture path required");
    let bytes = std::fs::read(&path).unwrap();
    let run: Run = architect::transport::decode(&bytes).unwrap();
    let before = serde_json::to_value(&run).unwrap();
    assert_eq!(run.attempt, 6);
    assert_eq!(run.stage, Stage::Execution);
    assert_eq!(run.artifacts.len(), 8);
    let artifact = run
        .artifacts
        .iter()
        .rev()
        .find(|a| a.stage == run.stage && a.attempt == run.attempt)
        .unwrap();
    let read = verification::observe(artifact);
    let prepared = FakeProvider
        .prepare_verify(
            &ObservedInvocation {
                turn_id: "independent-frozen-proof-audit",
                run: &run,
                resume: None,
                context_packet: None,
            },
            artifact,
            &read,
        )
        .unwrap();
    let registry = registry(&prepared.prompt);
    for id in registry["documents"].as_object().unwrap().keys() {
        body(id, &registry);
    }
    let sections = &registry["sections"];
    let artifacts: Vec<Artifact> =
        serde_json::from_value(resolve(&sections["Prior artifacts"], &registry)).unwrap();
    assert_eq!(artifacts.len(), 6);
    let omitted: Vec<_> = run
        .artifacts
        .iter()
        .filter(|a| !artifacts.iter().any(|selected| selected.id == a.id))
        .map(|a| (a.stage, a.attempt))
        .collect();
    assert_eq!(omitted, vec![(Stage::Execution, 4), (Stage::Execution, 5)]);
    for selected in &artifacts {
        assert_eq!(
            serde_json::to_value(selected).unwrap(),
            serde_json::to_value(run.artifacts.iter().find(|a| a.id == selected.id).unwrap())
                .unwrap()
        );
        assert_eq!(hash(&selected.content), selected.hash);
    }
    let required = run.expected_checks();
    assert_eq!(required.len(), 124);
    assert_eq!(
        resolve(&sections["Expected IDs"], &registry),
        json!(required)
    );
    assert_eq!(
        prepared
            .schema
            .pointer("/properties/checks/items/properties/check_id/enum")
            .unwrap(),
        &json!(required)
    );
    // Role-specific acceptance constraints may tighten nullable fields while
    // preserving the complete original typed DTO field inventory.
    assert_eq!(
        prepared.schema["properties"]["quality"]["properties"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<BTreeSet<_>>(),
        architect::quality::schema(&run)["properties"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<BTreeSet<_>>()
    );
    assert_eq!(
        resolve(&sections["Current quality gate assessments"], &registry),
        serde_json::to_value(&run.quality_gates).unwrap()
    );
    assert_eq!(run.quality_gates.len(), 6);
    let current: Vec<Evidence> =
        serde_json::from_value(resolve(&sections["Current receipts"], &registry)).unwrap();
    let historical: Vec<Evidence> =
        serde_json::from_value(resolve(&sections["Historical proof receipts"], &registry)).unwrap();
    assert_eq!(current.len(), 17);
    assert_eq!(historical.len(), 11);
    let current_ids: BTreeSet<_> = current.iter().map(|e| e.id.as_str()).collect();
    let archived: Value = serde_json::from_slice(
        &std::fs::read("/tmp/antislop-history-selection.json")
            .expect("original saved proof-selection manifest"),
    )
    .unwrap();
    let originally_selected: BTreeSet<_> = archived["selected_current_receipt_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|value| value.as_str().unwrap())
        .collect();
    let removed = BTreeSet::from([
        "4e6eef40-9ca3-4769-8dbc-336ac52ed839",
        "91f24c35-4d26-45b6-8a2a-6f92bf032d64",
    ]);
    // Only the uncited obsolete supplemental probe and its input binding leave
    // the original selection; required proof obligations are audited below.
    let expected: BTreeSet<_> = originally_selected.difference(&removed).copied().collect();
    assert_eq!(current_ids, expected);
    let allowed: BTreeSet<_> = prepared
        .schema
        .pointer("/properties/checks/items/properties/evidence_ids/items/enum")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(allowed, current_ids);
    for e in &current {
        assert!(verification::evidence_is_current(&run, artifact, e));
    }
    for e in &historical {
        assert!(!verification::evidence_is_current(&run, artifact, e));
        assert!(!allowed.contains(e.id.as_str()));
    }
    let retained: BTreeMap<_, _> = current
        .iter()
        .chain(&historical)
        .map(|e| (e.id.as_str(), e))
        .collect();
    for e in retained.values() {
        assert_eq!(
            serde_json::to_value(*e).unwrap(),
            serde_json::to_value(run.evidence.iter().find(|old| old.id == e.id).unwrap()).unwrap()
        );
        assert_eq!(hash(&e.output), e.output_hash);
    }
    let graph: BTreeMap<String, ProvenanceNode> =
        serde_json::from_value(resolve(&sections["Dependency graph"], &registry)).unwrap();
    let reviews: BTreeMap<String, Review> = serde_json::from_value(resolve(
        &sections["Selected authenticating reviews"],
        &registry,
    ))
    .unwrap();
    // The production closure conservatively retains two additional ancestors.
    assert_eq!(graph.len(), 133);
    assert_eq!(reviews.len(), 2);
    let proof_rows = proof_review_rows(&run, &graph, &reviews, &registry);
    for (id, review) in &reviews {
        assert_eq!(hash(serde_json::to_vec(review).unwrap()), *id);
        let old = run
            .provenance_reviews
            .get(id)
            .or_else(|| {
                run.reviews
                    .iter()
                    .find(|r| hash(serde_json::to_vec(r).unwrap()) == *id)
            })
            .unwrap();
        assert_eq!(
            serde_json::to_value(review).unwrap(),
            serde_json::to_value(old).unwrap()
        );
        for check in review
            .checks
            .iter()
            .filter(|check| proof_rows[id].contains(&check.check_id))
        {
            for evidence in &check.evidence_ids {
                assert!(
                    retained.contains_key(evidence.as_str()),
                    "authenticating review omitted receipt {evidence}"
                );
            }
        }
    }
    for (id, node) in &graph {
        assert_eq!(
            serde_json::to_value(node).unwrap(),
            serde_json::to_value(&run.provenance[id]).unwrap()
        );
        for dependency in node.dependencies.keys() {
            assert!(
                graph.contains_key(dependency),
                "missing ancestor {dependency}"
            );
        }
        if let Some(binding) = &node.revalidation {
            assert!(
                reviews.contains_key(&binding.review_hash),
                "missing actual review body {}",
                binding.review_hash
            );
            for dependency in binding.dependencies.keys() {
                assert!(graph.contains_key(dependency));
            }
            for evidence in &binding.evidence_ids {
                assert!(
                    retained.contains_key(evidence.as_str()),
                    "missing binding receipt {evidence}"
                );
            }
        }
    }
    for check in &required {
        if let Some(id) = check.strip_prefix("REVALIDATE:") {
            assert!(graph.contains_key(id), "required claim omitted {id}");
        }
    }
    let old_execution = run
        .artifacts
        .iter()
        .find(|a| a.stage == Stage::Execution && a.attempt == 3)
        .unwrap();
    assert!(required.contains(&format!("REVALIDATE:artifact:{}", old_execution.id)));
    assert!(graph.contains_key(&format!("artifact:{}", old_execution.id)));
    for gate in run.quality_gates.values() {
        assert!(
            reviews.contains_key(&gate.review_hash),
            "gate authenticating review missing"
        );
        for evidence in &gate.evidence_ids {
            assert!(
                retained.contains_key(evidence.as_str()),
                "gate receipt missing"
            );
        }
    }
    let original_validity = verification::provenance_validity(&run);
    let mut projected = run.clone();
    projected.artifacts = artifacts;
    projected.evidence = current.into_iter().chain(historical).collect();
    projected.provenance = graph;
    projected.reviews = reviews.values().cloned().collect();
    projected.provenance_reviews = reviews;
    let projected_validity = verification::provenance_validity(&projected);
    for id in projected.provenance.keys() {
        assert_eq!(
            projected_validity[id], original_validity[id],
            "projection changed validity of {id}"
        );
    }
    assert_eq!(projected.expected_checks(), required);
    assert_eq!(serde_json::to_value(&run).unwrap(), before);
    assert_eq!(
        std::fs::read(path).unwrap(),
        bytes,
        "audit must not mutate saved fixture"
    );
    eprintln!(
        "independent saved-fixture proof: 6/8 artifacts, 133 nodes, 17 current +11 historical receipts, 2 full reviews, 6 gates, 124 checks; all raw documents bytehash-exact"
    );
}

#[test]
fn proof_closure_keeps_required_ancestor_receipts_reviews_gates_and_current_failures() {
    use architect::quality::{GateAssessment, GatePhase};
    use std::collections::BTreeMap;
    let (_root, mut run, mut current, _read) = fixture();
    run.attempt = 6;
    current.attempt = 6;
    let ancestors: Vec<_> = [
        ("discovery-ancestor", Stage::Discovery, 1),
        ("planning-ancestor", Stage::Planning, 2),
        ("execution-required", Stage::Execution, 3),
        ("execution-rejected-four", Stage::Execution, 4),
        ("execution-rejected-five", Stage::Execution, 5),
    ]
    .into_iter()
    .map(|(id, stage, attempt)| {
        let content = if attempt == 3 {
            r#"{"evidence_ids":["historical-proof-first","historical-proof-\u0074hird"]}"#
                .to_string()
        } else {
            format!("Actual original {id} body")
        };
        Artifact {
            id: id.into(),
            stage,
            attempt,
            spec_revision: 1,
            session_id: format!("executor-{id}"),
            hash: hash(&content),
            content,
        }
    })
    .collect();
    run.artifacts = ancestors.iter().cloned().chain([current.clone()]).collect();
    for (index, a) in ancestors.iter().take(3).enumerate() {
        let dependencies = if index == 0 {
            BTreeMap::new()
        } else {
            BTreeMap::from([(
                format!("artifact:{}", ancestors[index - 1].id),
                "older-assessed-binding".into(),
            )])
        };
        run.provenance.insert(
            format!("artifact:{}", a.id),
            ProvenanceNode {
                kind: ProvenanceKind::Artifact,
                spec_revision: 1,
                content_hash: a.hash.clone(),
                content: format!("Pinned ancestor {}", a.id),
                source_hashes: BTreeMap::from([("objective".into(), hash("old source"))]),
                dependencies,
                valid: false,
                revalidation: None,
                superseded_by: None,
                refresh: false,
            },
        );
    }
    for (id, output) in [
        (
            "historical-proof-first",
            r#"{"references":["historical-proof-second","historical-proof-\u0066ourth"]}"#,
        ),
        (
            "historical-proof-second",
            "Exact original ancestral observation",
        ),
        (
            "historical-proof-third",
            "Exact observation referenced by escaped artifact citation",
        ),
        (
            "historical-proof-fourth",
            "Exact observation referenced by escaped receipt citation",
        ),
        ("unrelated-old-proof", "not needed by any selected claim"),
    ] {
        run.evidence.push(Evidence {
            id: id.into(),
            artifact_hash: ancestors[0].hash.clone(),
            spec_revision: 1,
            source: "fixture:historical".into(),
            output: output.into(),
            output_hash: hash(output),
            input_hash: Some(hash("old inputs")),
            success: true,
        });
    }
    let gate_receipt_output =
        "Original current-input observation for the historical planning artifact";
    run.evidence.push(Evidence {
        id: "gate-current-proof".into(),
        artifact_hash: ancestors[1].hash.clone(),
        spec_revision: run.spec_revision,
        source: "fixture:gate-current".into(),
        output: gate_receipt_output.into(),
        output_hash: hash(gate_receipt_output),
        input_hash: Some(hash(serde_json::to_vec(&run.input_snapshot).unwrap())),
        success: true,
    });
    let report = architect::quality::synthetic_artifacts(&run);
    let report_hash = hash(serde_json::to_vec(&report).unwrap());
    let review = Review {
        quality: Some(report),
        artifact_hash: ancestors[1].hash.clone(),
        spec_revision: 1,
        checklist_version: run.checklist_version.clone(),
        executor_session: ancestors[1].session_id.clone(),
        verifier_session: "independent-history-verifier".into(),
        checks: vec![
            CheckResult {
                check_id: "PLAN:PLAN-1".into(),
                status: CheckStatus::Pass,
                method: "Actual historical method".into(),
                evidence_ids: vec!["historical-proof-first".into()],
                rationale: "Original historical rationale".into(),
            },
            CheckResult {
                check_id: architect::quality::gate_check_id("plan_ready", GatePhase::Design),
                status: CheckStatus::Pass,
                method: "Actual gate inspection".into(),
                evidence_ids: vec!["gate-current-proof".into()],
                rationale: "Bound to the original artifact and current inputs".into(),
            },
        ],
        findings: vec![],
    };
    let review_hash = hash(serde_json::to_vec(&review).unwrap());
    run.provenance_reviews
        .insert(review_hash.clone(), review.clone());
    run.reviews.push(review.clone());
    run.quality_gates.insert(
        "historical-design-gate".into(),
        GateAssessment {
            gate_id: "plan_ready".into(),
            phase: GatePhase::Design,
            status: CheckStatus::Pass,
            spec_revision: 1,
            source_hashes: run.source_hashes(),
            artifact_hash: ancestors[1].hash.clone(),
            review_hash: review_hash.clone(),
            check_id: architect::quality::gate_check_id("plan_ready", GatePhase::Design),
            evidence_ids: vec!["gate-current-proof".into()],
            report_hash,
        },
    );
    assert!(authentic_gate(
        &run,
        &run.quality_gates["historical-design-gate"]
    ));
    let failed_output =
        "{\"exit_code\":1,\"stdout\":\"actual failing counterexample\",\"stderr\":\"\"}";
    run.evidence.push(Evidence {
        id: "failed-current-runner".into(),
        artifact_hash: current.hash.clone(),
        spec_revision: 1,
        source: "runner:[\"actual-check\"]".into(),
        output: failed_output.into(),
        output_hash: hash(failed_output),
        input_hash: Some(hash(serde_json::to_vec(&run.input_snapshot).unwrap())),
        success: false,
    });
    let before = serde_json::to_value(&run).unwrap();
    let read = verification::observe(&current);
    let prepared = FakeProvider
        .prepare_verify(
            &ObservedInvocation {
                turn_id: "synthetic-proof-closure",
                run: &run,
                resume: None,
                context_packet: None,
            },
            &current,
            &read,
        )
        .unwrap();
    let registry = registry(&prepared.prompt);
    for id in registry["documents"].as_object().unwrap().keys() {
        body(id, &registry);
    }
    let sections = &registry["sections"];
    let selected = resolve(&sections["Prior artifacts"], &registry);
    let selected_ids: BTreeSet<_> = selected
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap())
        .collect();
    assert_eq!(
        selected_ids,
        BTreeSet::from([
            "discovery-ancestor",
            "planning-ancestor",
            "execution-required",
            current.id.as_str()
        ])
    );
    let graph = resolve(&sections["Dependency graph"], &registry);
    for a in ancestors.iter().take(3) {
        assert_eq!(
            graph[format!("artifact:{}", a.id)],
            serde_json::to_value(&run.provenance[&format!("artifact:{}", a.id)]).unwrap()
        );
    }
    let historical = resolve(&sections["Historical proof receipts"], &registry);
    let historical_ids: BTreeSet<_> = historical
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert!(historical_ids.contains("historical-proof-first"));
    assert!(historical_ids.contains("historical-proof-second"));
    assert!(
        historical_ids.contains("historical-proof-third"),
        "JSON-escaped artifact citations are semantic references too"
    );
    assert!(
        historical_ids.contains("historical-proof-fourth"),
        "JSON-escaped transitive receipt citations are semantic references too"
    );
    assert!(!historical_ids.contains("unrelated-old-proof"));
    assert_eq!(
        resolve(&sections["Selected authenticating reviews"], &registry)[&review_hash],
        serde_json::to_value(&review).unwrap()
    );
    assert_eq!(
        resolve(&sections["Current quality gate assessments"], &registry),
        serde_json::to_value(&run.quality_gates).unwrap()
    );
    assert_eq!(
        resolve(&sections["Expected IDs"], &registry),
        json!(run.expected_checks())
    );
    let current_receipts = resolve(&sections["Current receipts"], &registry);
    assert!(
        current_receipts
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["id"] == "failed-current-runner"
                && e["output"] == failed_output
                && e["success"] == false)
    );
    let allowed: BTreeSet<_> = prepared
        .schema
        .pointer("/properties/checks/items/properties/evidence_ids/items/enum")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    for id in historical_ids {
        assert!(!allowed.contains(id));
    }
    assert_eq!(serde_json::to_value(&run).unwrap(), before);
}

#[test]
#[ignore = "read-only audit of an explicitly supplied saved Validation8 snapshot"]
fn frozen_validation_eight_prompt_fits_scalar_limit_without_losing_gate_proofs() {
    saved_stage_prompt_audit(
        "ANTISLOP_VERIFIER_VALIDATION_RUN_JSON",
        Stage::Validation,
        8,
    );
}

#[test]
#[ignore = "read-only audit of an explicitly supplied saved Integration9 snapshot"]
fn frozen_integration_nine_prompt_fits_scalar_limit_without_losing_gate_proofs() {
    saved_stage_prompt_audit(
        "ANTISLOP_VERIFIER_INTEGRATION_RUN_JSON",
        Stage::Integration,
        9,
    );
}

fn saved_stage_prompt_audit(path_variable: &str, stage: Stage, attempt: u64) {
    use std::collections::{BTreeMap, BTreeSet};
    let path = std::env::var(path_variable).expect("saved stage Run JSON required");
    let raw = std::fs::read(&path).unwrap();
    let run: Run = architect::transport::decode(&raw).unwrap();
    assert_eq!((run.stage, run.attempt), (stage, attempt));
    let before = serde_json::to_value(&run).unwrap();
    let artifact = run
        .artifacts
        .iter()
        .rev()
        .find(|a| {
            a.stage == run.stage && a.attempt == run.attempt && a.spec_revision == run.spec_revision
        })
        .unwrap();
    let read = run
        .evidence
        .iter()
        .rev()
        .find(|e| e.source == "runtime:artifact-read" && e.artifact_hash == artifact.hash)
        .unwrap();
    let prepared = FakeProvider
        .prepare_verify(
            &ObservedInvocation {
                turn_id: "independent-validation8-scalar-audit",
                run: &run,
                resume: None,
                context_packet: None,
            },
            artifact,
            read,
        )
        .unwrap();
    let count = prepared.prompt.chars().count();
    let registry = registry(&prepared.prompt);
    for id in registry["documents"].as_object().unwrap().keys() {
        body(id, &registry);
    }
    let sections = &registry["sections"];
    assert_eq!(
        resolve(&sections["Run specification"], &registry),
        serde_json::to_value(&run.spec).unwrap()
    );
    assert_eq!(
        resolve(&sections["Pinned artifact"], &registry),
        serde_json::to_value(artifact).unwrap()
    );
    assert_eq!(
        resolve(&sections["Current quality gate assessments"], &registry),
        serde_json::to_value(&run.quality_gates).unwrap()
    );
    assert_eq!(
        resolve(&sections["Expected IDs"], &registry),
        json!(run.expected_checks())
    );
    assert_eq!(
        prepared
            .schema
            .pointer("/properties/checks/items/properties/check_id/enum")
            .unwrap(),
        &json!(run.expected_checks())
    );
    let artifacts: Vec<Artifact> =
        serde_json::from_value(resolve(&sections["Prior artifacts"], &registry)).unwrap();
    for a in &artifacts {
        assert_eq!(
            serde_json::to_value(a).unwrap(),
            serde_json::to_value(run.artifacts.iter().find(|old| old.id == a.id).unwrap()).unwrap()
        );
        assert_eq!(hash(&a.content), a.hash);
    }
    let current: Vec<Evidence> =
        serde_json::from_value(resolve(&sections["Current receipts"], &registry)).unwrap();
    let historical: Vec<Evidence> =
        serde_json::from_value(resolve(&sections["Historical proof receipts"], &registry)).unwrap();
    let graph: BTreeMap<String, ProvenanceNode> =
        serde_json::from_value(resolve(&sections["Dependency graph"], &registry)).unwrap();
    let reviews: BTreeMap<String, Review> = serde_json::from_value(resolve(
        &sections["Selected authenticating reviews"],
        &registry,
    ))
    .unwrap();
    let allowed: BTreeSet<_> = prepared
        .schema
        .pointer("/properties/checks/items/properties/evidence_ids/items/enum")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|id| id.as_str().unwrap())
        .collect();
    assert_eq!(allowed, current.iter().map(|e| e.id.as_str()).collect());
    let retained: BTreeSet<_> = current
        .iter()
        .chain(&historical)
        .map(|e| e.id.as_str())
        .collect();
    for e in current.iter().chain(&historical) {
        assert_eq!(
            serde_json::to_value(e).unwrap(),
            serde_json::to_value(run.evidence.iter().find(|old| old.id == e.id).unwrap()).unwrap()
        );
        assert_eq!(hash(&e.output), e.output_hash);
    }
    for e in &current {
        assert!(verification::evidence_is_current(&run, artifact, e));
    }
    for e in &historical {
        assert!(!verification::evidence_is_current(&run, artifact, e));
        assert!(!allowed.contains(e.id.as_str()));
    }
    for (id, node) in &graph {
        assert_eq!(
            serde_json::to_value(node).unwrap(),
            serde_json::to_value(&run.provenance[id]).unwrap()
        );
        for ancestor in node.dependencies.keys() {
            assert!(
                graph.contains_key(ancestor),
                "missing dependency {ancestor}"
            );
        }
        if let Some(binding) = &node.revalidation {
            assert!(reviews.contains_key(&binding.review_hash));
            for ancestor in binding.dependencies.keys() {
                assert!(graph.contains_key(ancestor));
            }
            for evidence in &binding.evidence_ids {
                assert!(retained.contains(evidence.as_str()));
            }
        }
    }
    let proof_rows = proof_review_rows(&run, &graph, &reviews, &registry);
    for (id, review) in &reviews {
        assert_eq!(hash(serde_json::to_vec(review).unwrap()), *id);
        for check in review
            .checks
            .iter()
            .filter(|check| proof_rows[id].contains(&check.check_id))
        {
            for evidence in &check.evidence_ids {
                assert!(retained.contains(evidence.as_str()));
            }
        }
    }
    for check in run.expected_checks() {
        if let Some(node) = check.strip_prefix("REVALIDATE:") {
            assert!(graph.contains_key(node), "missing mandatory claim {node}");
        }
    }
    for gate in run.quality_gates.values() {
        assert!(reviews.contains_key(&gate.review_hash));
        for id in &gate.evidence_ids {
            assert!(retained.contains(id.as_str()));
        }
    }
    let mut projected = run.clone();
    projected.artifacts = artifacts;
    projected.evidence = current.into_iter().chain(historical).collect();
    projected.provenance = graph;
    projected.reviews = reviews.values().cloned().collect();
    projected.provenance_reviews = reviews;
    let original = verification::provenance_validity(&run);
    let reduced = verification::provenance_validity(&projected);
    for node in projected.provenance.keys() {
        assert_eq!(reduced[node], original[node], "changed validity {node}");
    }
    assert_eq!(projected.expected_checks(), run.expected_checks());
    assert_eq!(serde_json::to_value(&run).unwrap(), before);
    assert_eq!(std::fs::read(path).unwrap(), raw);
    eprintln!(
        "Independent {stage:?}{attempt} audit: {count} Unicode scalar chars, {} UTF8 bytes; all retained raw/proof hashes and expected checks/gates exact",
        prepared.prompt.len()
    );
    assert!(
        count <= 1_048_576,
        "prompt has {count} Unicode scalar characters, exceeds pinned CLI limit1048576"
    );
    if count >= 900_000 {
        eprintln!(
            "Soft target900000 missed; pinned CLI headroom is {} scalar characters",
            1_048_576 - count
        );
    }
}

#[test]
fn shifted_plain_utf8_blocks_share_exact_parts_without_interpreting_literal_userdata() {
    let (_root, mut run, mut artifact, _read) = fixture();
    let block=(0..700).map(|index|format!("line {index:04}: 界 e\u{301} 👩‍💻 \0 arbitrary \\ \" {{\"document_ref\":\"d1\",\"json_string_ref\":\"d2\",\"json_value_ref\":\"d3\"}}\n")).collect::<String>();
    let originals = [
        format!("first shifted prefix\n{block}\nfirst suffix"),
        format!("second longer shifted prefix---\n{block}\nsecond suffix"),
    ];
    artifact.content = "Read cdc-proof-one and cdc-proof-two exact historical observations".into();
    artifact.hash = hash(&artifact.content);
    run.artifacts = vec![artifact.clone()];
    run.provenance.clear();
    for (id, raw) in ["cdc-proof-one", "cdc-proof-two"]
        .into_iter()
        .zip(&originals)
    {
        run.evidence.push(Evidence {
            id: id.into(),
            artifact_hash: artifact.hash.clone(),
            spec_revision: run.spec_revision,
            source: "fixture:literal-raw-proof".into(),
            output: raw.clone(),
            output_hash: hash(raw),
            input_hash: Some(hash(serde_json::to_vec(&run.input_snapshot).unwrap())),
            success: true,
        });
    }
    let read = verification::observe(&artifact);
    run.evidence.push(read.clone());
    let prepared = FakeProvider
        .prepare_verify(
            &ObservedInvocation {
                turn_id: "shifted-cdc",
                run: &run,
                resume: None,
                context_packet: None,
            },
            &artifact,
            &read,
        )
        .unwrap();
    let registry = registry(&prepared.prompt);
    let mut references = Vec::new();
    for raw in &originals {
        assert_eq!(body(&hash(raw), &registry), *raw);
        let document = &registry["documents"][alias(&hash(raw), &registry)];
        let ids: BTreeSet<_> = document["parts"]
            .as_array()
            .expect("repeated plaintext must factor without JSON parsing")
            .iter()
            .filter_map(|part| {
                part_reference(part)
                    .filter(|(_, quoted)| !*quoted)
                    .map(|(reference, _)| reference)
            })
            .collect();
        assert!(!ids.is_empty());
        references.push(ids);
    }
    assert!(
        !references[0].is_disjoint(&references[1]),
        "shifted UTF8 documents must reuse exact shared chunks"
    );
    for id in registry["documents"].as_object().unwrap().keys() {
        body(id, &registry);
    }
    let receipts = resolve(&registry["sections"]["Current receipts"], &registry);
    for (id, raw) in ["cdc-proof-one", "cdc-proof-two"]
        .into_iter()
        .zip(&originals)
    {
        assert_eq!(
            receipts
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["id"] == id)
                .unwrap()["output"],
            *raw
        );
    }
}

#[test]
fn stale_gate_without_authenticating_review_remains_visible_without_current_proof_credit() {
    use architect::quality::{GateAssessment, GatePhase, gate_check_id};
    let (_root, mut run, artifact, read) = fixture();
    for (key, gate_id) in [
        ("stale-context", "context_ready"),
        ("stale-plan", "plan_ready"),
    ] {
        run.quality_gates.insert(
            key.into(),
            GateAssessment {
                gate_id: gate_id.into(),
                phase: GatePhase::Design,
                status: CheckStatus::Pass,
                spec_revision: run.spec_revision,
                source_hashes: run.source_hashes(),
                artifact_hash: hash("unavailable old artifact"),
                review_hash: hash("missing authenticating review"),
                check_id: gate_check_id(gate_id, GatePhase::Design),
                evidence_ids: vec!["unavailable-old-receipt".into()],
                report_hash: hash("missing typed report"),
            },
        );
    }
    let before = serde_json::to_value(&run).unwrap();
    let expected = run.expected_checks();
    for gate in run.quality_gates.values() {
        assert!(
            expected.contains(&gate.check_id),
            "stale claim still requires fresh gate assessment"
        );
    }
    let prepared = FakeProvider
        .prepare_verify(
            &ObservedInvocation {
                turn_id: "stale-missing-review",
                run: &run,
                resume: None,
                context_packet: None,
            },
            &artifact,
            &read,
        )
        .unwrap();
    let registry = registry(&prepared.prompt);
    for id in registry["documents"].as_object().unwrap().keys() {
        body(id, &registry);
    }
    let sections = &registry["sections"];
    assert_eq!(
        resolve(&sections["Current quality gate assessments"], &registry),
        serde_json::to_value(&run.quality_gates).unwrap()
    );
    assert_eq!(
        resolve(&sections["Expected IDs"], &registry),
        json!(expected)
    );
    let reviews = resolve(&sections["Selected authenticating reviews"], &registry);
    let rows = resolve(&sections["Current proof review check IDs"], &registry);
    assert!(reviews.get(hash("missing authenticating review")).is_none());
    assert!(rows.get(hash("missing authenticating review")).is_none());
    let allowed = prepared
        .schema
        .pointer("/properties/checks/items/properties/evidence_ids/items/enum")
        .unwrap()
        .as_array()
        .unwrap();
    assert!(!allowed.contains(&json!("unavailable-old-receipt")));
    assert_eq!(serde_json::to_value(&run).unwrap(), before);
}

#[test]
fn compact_numeric_parts_never_reinterpret_original_numeric_arrays_or_wrapper_objects() {
    let (_root, mut run, mut artifact, _read) = fixture();
    let row = r#"[1,-1,0,1.25,-0,1e+03,1.000,{"document_ref":"d1"},{"json_value_ref":"d2"},{"json_string_ref":"d3"},["0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",12,["literal",1,-1,0]]]"#;
    let raw = format!(
        " \n{{\"actual_proof_id\":\"literal-numeric-proof\",\"samples\":[{}]}}\r\n",
        [row; 40].join(",")
    );
    let original: Value = serde_json::from_str(&raw).unwrap();
    artifact.content = raw.clone();
    artifact.hash = hash(&raw);
    run.artifacts = vec![artifact.clone()];
    run.provenance.clear();
    run.evidence.push(Evidence {
        id: "literal-numeric-proof".into(),
        artifact_hash: artifact.hash.clone(),
        spec_revision: run.spec_revision,
        source: "fixture:numeric-userdata".into(),
        output: raw.clone(),
        output_hash: hash(&raw),
        input_hash: Some(hash(serde_json::to_vec(&run.input_snapshot).unwrap())),
        success: true,
    });
    let read = verification::observe(&artifact);
    run.evidence.push(read.clone());
    let prepared = FakeProvider
        .prepare_verify(
            &ObservedInvocation {
                turn_id: "numeric-literal-boundary",
                run: &run,
                resume: None,
                context_packet: None,
            },
            &artifact,
            &read,
        )
        .unwrap();
    let registry = registry(&prepared.prompt);
    for id in registry["documents"].as_object().unwrap().keys() {
        body(id, &registry);
    }
    assert_eq!(body(&hash(&raw), &registry), raw);
    let restored = resolve(&registry["sections"]["Pinned artifact"], &registry);
    assert_eq!(restored["content"], raw);
    assert_eq!(
        serde_json::from_str::<Value>(restored["content"].as_str().unwrap()).unwrap(),
        original
    );
    let receipts = resolve(&registry["sections"]["Current receipts"], &registry);
    let receipt = receipts
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "literal-numeric-proof")
        .unwrap();
    assert_eq!(receipt["output"], raw);
    assert_eq!(
        serde_json::from_str::<Value>(receipt["output"].as_str().unwrap()).unwrap(),
        original
    );
}
