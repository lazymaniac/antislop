use architect::{
    context::{self, Bundle, Page, Read, Reader, Scope},
    hash,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Cursor, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt, symlink},
    process::{Command, Stdio},
};

fn scope() -> Scope {
    Scope {
        run_id: "run".into(),
        turn_id: "turn".into(),
        spec_revision: 1,
        input_hash: hash("inputs"),
        prompt_hash: hash("prompt"),
        schema_hash: hash("schema"),
    }
}
fn reader(sources: impl IntoIterator<Item = (String, String)>) -> Reader {
    let bundle = Bundle::new(scope(), sources).unwrap();
    let bytes = bundle.bytes().unwrap();
    Reader::from_bytes(&bytes, &hash(&bytes)).unwrap()
}
fn all(reader: &Reader, document: &str, size: usize) -> String {
    let mut result = String::new();
    let mut offset = 0;
    loop {
        let page = reader
            .read(
                &Read {
                    bundle_hash: reader.bundle_hash().to_owned(),
                    document_hash: document.into(),
                    offset,
                    max_bytes: size,
                },
                1,
            )
            .unwrap();
        reader.authenticate(&page).unwrap();
        assert!(page.content.len() <= size);
        result.push_str(&page.content);
        match page.next_offset {
            Some(next) => {
                assert!(next > offset);
                offset = next;
            }
            None => break,
        }
    }
    assert_eq!(hash(&result), document);
    result
}
fn initialize() -> Value {
    json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}})
}
fn notification() -> Value {
    json!({"jsonrpc":"2.0","method":"notifications/initialized"})
}
fn call(id: usize, reader: &Reader, doc: &str, offset: usize, size: usize) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call","params":{"name":"read_context","arguments":{"bundle_hash":reader.bundle_hash(),"document_hash":doc,"offset":offset,"max_bytes":size}}})
}
fn frames(requests: &[Value]) -> Cursor<Vec<u8>> {
    Cursor::new(
        requests
            .iter()
            .map(|r| format!("{r}\n"))
            .collect::<String>()
            .into_bytes(),
    )
}
fn values(output: &[u8]) -> Vec<Value> {
    std::str::from_utf8(output)
        .unwrap()
        .lines()
        .map(|s| architect::transport::decode(s.as_bytes()).unwrap())
        .collect()
}

#[test]
fn exact_pages_reconstruct_large_unicode_sources_and_a_paged_source_index() {
    let body = (0..10000)
        .map(|n| format!("{n}: źródło 🦀 literal \\\"\n"))
        .collect::<String>();
    assert!(body.len() > 192000);
    let wrapped = "{\"json_value_ref\":\"literal user data\",\"numbers\":[1,-2,0]}";
    let r = reader([
        ("directory/AGENTS.md".into(), body.clone()),
        ("historical:receipt".into(), body.clone()),
        ("literal".into(), wrapped.into()),
        ("empty".into(), String::new()),
    ]);
    assert_eq!(all(&r, &hash(&body), context::MAX_PAGE_BYTES), body);
    assert_eq!(all(&r, &hash(wrapped), 4), wrapped);
    assert_eq!(all(&r, &hash(""), 4), "");
    let index: Value = serde_json::from_str(&all(&r, r.index_hash(), 17)).unwrap();
    assert_eq!(index["scope"], serde_json::to_value(scope()).unwrap());
    assert_eq!(index["documents"].as_array().unwrap().len(), 3);
    let duplicate = index["documents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["document_hash"] == hash(&body))
        .unwrap();
    assert_eq!(
        duplicate["source_refs"],
        json!(["directory/AGENTS.md", "historical:receipt"])
    );
}

#[test]
fn changed_scope_bodies_lengths_refs_and_encoding_cannot_forge_a_bundle() {
    let bundle = Bundle::new(scope(), [("original".into(), "PRIVATE_SOURCE".into())]).unwrap();
    let bytes = bundle.bytes().unwrap();
    let mut changed = bytes.clone();
    changed.push(b' ');
    assert!(Reader::from_bytes(&changed, &hash(&bytes)).is_err());
    let value = serde_json::to_value(&bundle).unwrap();
    let doc = hash("PRIVATE_SOURCE");
    for mutation in ["body", "length", "hash", "scope", "empty-ref", "alias"] {
        let mut v = value.clone();
        match mutation {
            "body" => v["documents"][&doc]["utf8"] = json!("changed"),
            "length" => v["documents"][&doc]["utf8_bytes"] = json!(1),
            "hash" => {
                let d = v["documents"]
                    .as_object_mut()
                    .unwrap()
                    .remove(&doc)
                    .unwrap();
                v["documents"]["invalid"] = d;
            }
            "scope" => v["scope"]["input_hash"] = json!("invalid"),
            "empty-ref" => v["documents"][&doc]["source_refs"] = json!([]),
            "alias" => {
                v["documents"][hash("other")] =
                    json!({"source_refs":["original"],"utf8_bytes":5,"utf8":"other"});
            }
            _ => unreachable!(),
        }
        let raw = serde_json::to_vec(&v).unwrap();
        assert!(Reader::from_bytes(&raw, &hash(&raw)).is_err(), "{mutation}");
    }
    let duplicate = br#"{"format":"PRIVATE_SOURCE","format":"pinned-context-v1"}"#;
    let error = Reader::from_bytes(duplicate, &hash(duplicate))
        .err()
        .unwrap()
        .to_string();
    assert!(!error.contains("PRIVATE_SOURCE"));
    assert!(
        Bundle::new(
            scope(),
            [("same".into(), "one".into()), ("same".into(), "two".into())]
        )
        .is_err()
    );
    // A different turn has a different authority even when every source body matches.
    let mut other = bundle.clone();
    other.scope.turn_id = "another-turn".into();
    let other = other.bytes().unwrap();
    let other = Reader::from_bytes(&other, &hash(&other)).unwrap();
    assert_ne!(other.bundle_hash(), hash(bytes));
}

#[test]
fn reads_refuse_foreign_documents_invalid_unicode_offsets_and_tampered_receipts() {
    let text = "a🦀bc";
    let r = reader([("source".into(), text.into())]);
    let base = Read {
        bundle_hash: r.bundle_hash().to_owned(),
        document_hash: hash(text),
        offset: 0,
        max_bytes: 4,
    };
    for change in ["bundle", "path", "offset", "overflow", "small", "large"] {
        let mut read = base.clone();
        match change {
            "bundle" => read.bundle_hash = hash("other"),
            "path" => read.document_hash = "../../secret".into(),
            "offset" => read.offset = 2,
            "overflow" => read.offset = usize::MAX,
            "small" => read.max_bytes = 3,
            "large" => read.max_bytes = context::MAX_PAGE_BYTES + 1,
            _ => unreachable!(),
        }
        assert!(r.read(&read, 1).is_err(), "{change}");
    }
    let page = r.read(&base, 1).unwrap();
    assert_eq!(page.content, "a");
    assert_eq!(page.next_offset, Some(1));
    for change in ["body", "scope", "hash", "end", "next"] {
        let mut page = page.clone();
        match change {
            "body" => page.content = "b".into(),
            "scope" => page.receipt.scope.turn_id = "other".into(),
            "hash" => page.receipt.chunk_hash = hash("b"),
            "end" => page.receipt.end = usize::MAX,
            "next" => page.next_offset = None,
            _ => unreachable!(),
        }
        assert!(r.authenticate(&page).is_err(), "{change}");
    }
}

#[test]
fn mcp_exposes_one_scoped_read_and_commits_before_returning_source_bytes() {
    let r = reader([("source".into(), "PRIVATE_SOURCE".into())]);
    let mut input = frames(&[
        initialize(),
        notification(),
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        call(3, &r, &hash("PRIVATE_SOURCE"), 0, 32),
    ]);
    let committed = std::rc::Rc::new(std::cell::Cell::new(false));
    struct Output {
        bytes: Vec<u8>,
        committed: std::rc::Rc<std::cell::Cell<bool>>,
    }
    impl Write for Output {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.bytes.extend_from_slice(bytes);
            if self
                .bytes
                .windows(b"PRIVATE_SOURCE".len())
                .any(|w| w == b"PRIVATE_SOURCE")
            {
                assert!(self.committed.get());
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let mut output = Output {
        bytes: vec![],
        committed: committed.clone(),
    };
    let mut receipts = vec![];
    context::stdio::serve(&r, &mut input, &mut output, &mut |receipt| {
        receipts.push(receipt.clone());
        committed.set(true);
        Ok(())
    })
    .unwrap();
    let responses = values(&output.bytes);
    assert_eq!(responses.len(), 3);
    let tools = responses[1]["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "read_context");
    let page: Page = serde_json::from_str(
        responses[2]["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    r.authenticate(&page).unwrap();
    assert_eq!(page.receipt, receipts[0]);
    assert_eq!(page.receipt.sequence, 1);
    let mut output = vec![];
    let mut input = frames(&[
        initialize(),
        notification(),
        call(3, &r, &hash("PRIVATE_SOURCE"), 0, 32),
    ]);
    let error = context::stdio::serve(&r, &mut input, &mut output, &mut |_| {
        Err(architect::Error::Invalid("durable commit failed".into()))
    })
    .unwrap_err();
    assert!(error.to_string().contains("durable commit failed"));
    assert!(
        !String::from_utf8(output)
            .unwrap()
            .contains("PRIVATE_SOURCE")
    );
}

#[test]
fn mcp_refuses_uninitialized_writes_foreign_reads_and_malformed_arguments() {
    let r = reader([("source".into(), "PRIVATE_SOURCE".into())]);
    let mut bad = call(6, &r, &hash("PRIVATE_SOURCE"), 0, 32);
    bad["params"]["arguments"]["path"] = json!("PRIVATE_PATH");
    let mut wrong = call(5, &r, "PRIVATE_PATH", 0, 32);
    wrong["params"]["arguments"]["bundle_hash"] = json!(hash("other"));
    let mut input = frames(&[
        call(0, &r, &hash("PRIVATE_SOURCE"), 0, 32),
        initialize(),
        notification(),
        json!({"jsonrpc":"2.0","id":4,"method":"write_file","params":{"path":"PRIVATE_PATH"}}),
        wrong,
        bad,
    ]);
    let mut output = vec![];
    let mut commits = 0;
    context::stdio::serve(&r, &mut input, &mut output, &mut |_| {
        commits += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(commits, 0);
    let responses = values(&output);
    assert_eq!(responses[0]["error"]["code"], -32600);
    assert_eq!(responses[2]["error"]["code"], -32601);
    assert!(responses[3]["result"]["isError"].as_bool().unwrap());
    assert!(responses[4]["result"]["isError"].as_bool().unwrap());
    let output = String::from_utf8(output).unwrap();
    assert!(!output.contains("PRIVATE_PATH") && !output.contains("PRIVATE_SOURCE"));
    let mut input = Cursor::new(b"{\"jsonrpc\":\"2.0\",\"jsonrpc\":\"2.0\"}\n");
    assert!(context::stdio::serve(&r, &mut input, &mut vec![], &mut |_| Ok(())).is_err());
}

#[test]
fn mcp_integral_json_numbers_are_exact_byte_counts_but_fractional_and_rounded_values_are_refused() {
    let r = reader([("source".into(), "pinned source".into())]);
    let mut request = call(2, &r, &hash("pinned source"), 0, 32);
    request["params"]["arguments"]["offset"] = json!(0.0);
    request["params"]["arguments"]["max_bytes"] = json!(32.0);
    let mut input = frames(&[initialize(), notification(), request]);
    let mut output = vec![];
    let mut committed = 0;
    context::stdio::serve(&r, &mut input, &mut output, &mut |_| {
        committed += 1;
        Ok(())
    })
    .unwrap();
    assert_eq!(committed, 1);
    assert_eq!(values(&output)[1]["result"]["isError"], false);
    for bad in [
        json!(-1),
        json!(0.5),
        json!("32"),
        json!(true),
        json!(9_007_199_254_740_992.0),
        json!(18446744073709551616.0),
    ] {
        let read = json!({"bundle_hash":r.bundle_hash(),"document_hash":hash("pinned source"),"offset":bad,"max_bytes":32});
        assert!(
            architect::transport::decode::<Read>(&serde_json::to_vec(&read).unwrap()).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn native_stdio_process_reads_only_the_saved_snapshot_with_private_durable_receipts() {
    let root = tempfile::tempdir().unwrap();
    fs_private(root.path());
    let original = root.path().join("source");
    std::fs::write(&original, "pinned 🦀 source").unwrap();
    let bundle = Bundle::new(
        scope(),
        [("source".into(), std::fs::read_to_string(&original).unwrap())],
    )
    .unwrap();
    let bytes = bundle.bytes().unwrap();
    let r = Reader::from_bytes(&bytes, &hash(&bytes)).unwrap();
    let file = root.path().join("bundle.json");
    private_write(&file, &bytes);
    std::fs::write(&original, "new user contents").unwrap();
    let audit = root.path().join("reads.jsonl");
    let mut child = Command::new(env!("CARGO_BIN_EXE_antislop"))
        .args([
            "__context",
            file.to_str().unwrap(),
            r.bundle_hash(),
            audit.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = frames(&[
        initialize(),
        notification(),
        call(2, &r, &hash("pinned 🦀 source"), 0, 32),
    ]);
    std::io::copy(&mut input, &mut child.stdin.take().unwrap()).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let responses = values(&output.stdout);
    let page: Page = serde_json::from_str(
        responses[1]["result"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(page.content, "pinned 🦀 source");
    r.authenticate(&page).unwrap();
    let receipts: Vec<context::Receipt> = std::fs::read_to_string(&audit)
        .unwrap()
        .lines()
        .map(|line| architect::transport::decode(line.as_bytes()).unwrap())
        .collect();
    assert_eq!(receipts, vec![page.receipt]);
    assert_eq!(
        std::fs::metadata(&audit).unwrap().permissions().mode() & 0o077,
        0
    );
    assert_eq!(
        std::fs::read_to_string(original).unwrap(),
        "new user contents"
    );
    // A second process cannot overwrite an earlier turn's durable receipts.
    let before = std::fs::read(&audit).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_antislop"))
        .args([
            "__context",
            file.to_str().unwrap(),
            r.bundle_hash(),
            audit.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(std::fs::read(&audit).unwrap(), before);
}
fn fs_private(root: &std::path::Path) {
    std::fs::set_permissions(root, std::fs::Permissions::from_mode(0o700)).unwrap();
}
fn private_write(file: &std::path::Path, body: &[u8]) {
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(file)
        .unwrap();
    f.write_all(body).unwrap();
}

#[test]
fn bundle_file_tampering_links_and_public_permissions_fail_before_any_source_delivery() {
    let root = tempfile::tempdir().unwrap();
    fs_private(root.path());
    let bundle = Bundle::new(scope(), [("source".into(), "PRIVATE_SOURCE".into())]).unwrap();
    let bytes = bundle.bytes().unwrap();
    let file = root.path().join("bundle.json");
    private_write(&file, &bytes);
    let link = root.path().join("link");
    symlink(&file, &link).unwrap();
    let mut paths = BTreeMap::new();
    paths.insert("symlink", link);
    paths.insert("public", file.clone());
    paths.insert("changed", file.clone());
    for (kind, input) in paths {
        if kind == "public" {
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        } else {
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
        let expected = if kind == "changed" {
            hash("old bundle")
        } else {
            hash(&bytes)
        };
        let audit = root.path().join(format!("{kind}.jsonl"));
        let output = Command::new(env!("CARGO_BIN_EXE_antislop"))
            .args([
                "__context",
                input.to_str().unwrap(),
                &expected,
                audit.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{kind}");
        assert!(output.stdout.is_empty());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("PRIVATE_SOURCE"));
        assert!(!audit.exists());
    }
}

#[test]
fn context_for_a_real_turn_binds_all_prepared_input_bytes_and_current_scope() {
    let root = tempfile::tempdir().unwrap();
    let captured = "Original captured source 🦀";
    std::fs::write(root.path().join("source"), captured).unwrap();
    let mut spec: architect::domain::RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    spec.input_files = vec!["source".into()];
    let run = architect::domain::Run::new(architect::id(), spec).unwrap();
    let prompt = "Complete prepared input: źródła 🦀 and immutable communication/footer";
    let schema =
        json!({"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"]});
    let request = architect::providers::TurnRequest {
        turn_id: "actual-turn",
        run: &run,
        prompt,
        schema: &schema,
        resume: None,
    };
    let bundle = Bundle::for_turn(&request).unwrap();
    std::fs::write(root.path().join("source"), "later user change").unwrap();
    assert!(
        bundle.documents[&hash(captured)]
            .source_refs
            .contains("file:source")
    );
    for (source, body) in run.sources() {
        assert!(bundle.documents[&hash(&body)].source_refs.contains(&source));
        assert_eq!(bundle.documents[&hash(&body)].utf8, body);
    }
    assert_eq!(bundle.scope.run_id, run.id);
    assert_eq!(bundle.scope.spec_revision, run.spec_revision);
    assert_eq!(bundle.scope.turn_id, "actual-turn");
    assert_eq!(bundle.scope.prompt_hash, hash(prompt));
    assert_eq!(
        bundle.scope.schema_hash,
        hash(serde_json::to_vec(&schema).unwrap())
    );
    assert_eq!(
        bundle.scope.input_hash,
        hash(serde_json::to_vec(&run.input_snapshot).unwrap())
    );
    let bytes = bundle.bytes().unwrap();
    let reader = Reader::from_bytes(&bytes, &hash(&bytes)).unwrap();
    assert_eq!(all(&reader, &hash(prompt), 9), prompt);
    assert_eq!(all(&reader, &hash(captured), 7), captured);
}

#[test]
fn directory_context_browses_original_members_without_expanding_selectors_or_following_links() {
    use architect::{
        domain::{Run, RunSpec},
        providers::TurnRequest,
    };
    let root = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), "MUST_NOT_BE_READ").unwrap();
    std::fs::create_dir(root.path().join("nested")).unwrap();
    let original = "Original member źródło 🦀";
    std::fs::write(root.path().join("nested/source.rs"), original).unwrap();
    std::fs::write(root.path().join("binary"), [0, 255, 1]).unwrap();
    symlink(outside.path().join("secret"), root.path().join("link")).unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    spec.input_files = vec![".".into()];
    let mut run = Run::new(architect::id(), spec).unwrap();
    // A normal JSON file that looks exactly like a capture is still one file.
    let imitation = run.input_snapshot["."].clone();
    std::fs::write(root.path().join("ordinary.json"), &imitation).unwrap();
    run.input_snapshot.insert("ordinary.json".into(), imitation);
    let pinned = run.input_snapshot.clone();
    std::fs::write(root.path().join("nested/source.rs"), "CHANGED LIVE BODY").unwrap();
    let schema = json!({});
    let bundle = Bundle::for_turn(&TurnRequest {
        turn_id: "directory",
        run: &run,
        prompt: "task",
        schema: &schema,
        resume: None,
    })
    .unwrap();
    assert_eq!(run.input_snapshot, pinned);
    assert_eq!(run.spec.input_files, vec!["."]);
    let catalog = bundle
        .documents
        .values()
        .find(|d| d.source_refs.contains("directory-index:\"file:.\""))
        .unwrap();
    let catalog: Value = serde_json::from_str(&catalog.utf8).unwrap();
    assert_eq!(catalog["parent_source_ref"], "file:.");
    assert_eq!(catalog["parent_document_hash"], hash(&pinned["."]));
    let members = catalog["members"].as_array().unwrap();
    let source = members
        .iter()
        .find(|m| m["path"] == "nested/source.rs")
        .unwrap();
    assert_eq!(source["document_hash"], hash(original));
    let body = &bundle.documents[&hash(original)];
    assert_eq!(body.utf8, original);
    assert!(
        body.source_refs
            .contains(source["source_ref"].as_str().unwrap())
    );
    for (path, kind) in [("binary", "binary"), ("link", "symlink")] {
        let item = members.iter().find(|m| m["path"] == path).unwrap();
        assert_eq!(item["metadata"]["kind"], kind);
        assert!(item["source_ref"].is_null());
    }
    assert!(
        bundle
            .documents
            .values()
            .all(|d| !d.utf8.contains("MUST_NOT_BE_READ") && !d.utf8.contains("CHANGED LIVE BODY"))
    );
    assert!(!bundle.documents.values().any(|d| {
        d.source_refs
            .contains("directory-index:\"file:ordinary.json\"")
    }));
    // A changed or traversing captured member must not become a trusted view.
    for mutation in ["hash", "traversal"] {
        let mut changed = run.clone();
        let mut body: Value = serde_json::from_str(&changed.input_snapshot["."]).unwrap();
        if mutation == "hash" {
            body["files"]["nested/source.rs"]["sha256"] = json!(hash("wrong"));
        } else {
            let entry = body["files"]["nested/source.rs"].clone();
            body["files"]["../escape"] = entry;
        }
        changed.input_snapshot.insert(".".into(), body.to_string());
        assert!(
            Bundle::for_turn(&TurnRequest {
                turn_id: "bad",
                run: &changed,
                prompt: "task",
                schema: &schema,
                resume: None
            })
            .is_err(),
            "{mutation}"
        );
    }
}

#[test]
fn interview_originals_are_available_before_input_selection_and_bound_to_the_round() {
    use architect::{
        domain::{Run, RunSpec},
        providers::TurnRequest,
    };
    let root = tempfile::tempdir().unwrap();
    let original = "Full pre-contract source\n".repeat(10000);
    std::fs::write(root.path().join("large.rs"), &original).unwrap();
    let sources = architect::workspace::read_inputs(root.path(), &[".".into()]).unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    let mut run = Run::new(architect::id(), spec).unwrap();
    run.intake = true;
    assert!(run.input_snapshot.is_empty());
    let context = json!({"interview_source_hash":hash(serde_json::to_vec(&sources).unwrap()),"interview_source_visibility":"pinned_context"});
    run.orchestration_rounds.push(
        serde_json::from_value(json!({
            "id":"interview","spec_revision":1,"source_hashes":{},"input_hash":hash("inputs"),
            "approval_scope":"scope","previous_status":"awaiting_start","message_ids":[],
            "request_sequence":0,"request_context":context.to_string(),"status":"running",
            "observed_interview_sources":sources
        }))
        .unwrap(),
    );
    std::fs::remove_file(root.path().join("large.rs")).unwrap();
    let schema = json!({});
    let bundle = Bundle::for_turn(&TurnRequest {
        turn_id: "interview",
        run: &run,
        prompt: "manifest",
        schema: &schema,
        resume: None,
    })
    .unwrap();
    assert_eq!(bundle.documents[&hash(&original)].utf8, original);
    assert!(
        bundle
            .documents
            .values()
            .any(|d| d.source_refs.contains("interview-file:."))
    );
    assert!(run.spec.input_files.is_empty() && run.input_snapshot.is_empty());
    run.orchestration_rounds[0]
        .observed_interview_sources
        .insert(".".into(), "changed".into());
    assert!(
        Bundle::for_turn(&TurnRequest {
            turn_id: "bad",
            run: &run,
            prompt: "manifest",
            schema: &schema,
            resume: None
        })
        .is_err()
    );
}

#[test]
fn special_files_and_hardlinks_are_refused_without_waiting_for_input_or_creating_receipts() {
    use std::os::unix::ffi::OsStrExt;
    let root = tempfile::tempdir().unwrap();
    fs_private(root.path());
    let fifo = root.path().join("fifo");
    let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let audit = root.path().join("audit.jsonl");
    assert!(context::stdio::serve_file(&fifo, &hash("any"), &audit).is_err());
    assert!(!audit.exists());
    let bytes = Bundle::new(scope(), [("source".into(), "private".into())])
        .unwrap()
        .bytes()
        .unwrap();
    let file = root.path().join("bundle.json");
    private_write(&file, &bytes);
    std::fs::hard_link(&file, root.path().join("alias")).unwrap();
    assert!(context::stdio::serve_file(&file, &hash(&bytes), &audit).is_err());
    assert!(!audit.exists());
}
