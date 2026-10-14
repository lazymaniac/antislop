use architect::{
    domain::*,
    id,
    ipc::{self, Operation, Request},
    runtime::{Command, Runtime},
    storage::Store,
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    process::{Child, Command as Process, Stdio},
    sync::Arc,
    time::{Duration, Instant},
};
struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn launch(state: &std::path::Path) -> Daemon {
    let process = Process::new(env!("CARGO_BIN_EXE_antislop"))
        .args(["daemon", state.to_str().unwrap()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut daemon = Daemon(process);
    let deadline = Instant::now() + Duration::from_secs(10);
    while UnixStream::connect(state.join("architect.sock")).is_err() {
        assert!(daemon.0.try_wait().unwrap().is_none(), "daemon exited");
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    daemon
}
fn request(operation: Operation) -> Request {
    Request {
        version: ipc::PROTOCOL_VERSION,
        request_id: id(),
        idempotency_key: Some(id()),
        operation,
    }
}
fn fixture(root: &std::path::Path) -> RunSpec {
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.to_str().unwrap().into();
    spec
}
fn rpc(state: &std::path::Path, operation: Operation) -> Value {
    let response = ipc::call(state, &request(operation)).unwrap();
    assert!(response.error.is_none(), "{:?}", response.error);
    response.data.unwrap()
}
#[test]
fn raw_socket_rejects_duplicate_criteria_and_escaped_duplicate_keys() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let _daemon = launch(&state);
    for key in [r#""c""#, r#""\u0063""#] {
        let mut spec = fixture(temp.path());
        spec.requirements[0].criteria =
            std::collections::BTreeMap::from([("c".into(), "FIRST".into())]);
        let q = request(Operation::Mutate {
            command: Box::new(Command::Create { spec }),
        });
        let wire = serde_json::to_string(&q)
            .unwrap()
            .replace(r#""c":"FIRST""#, &format!(r#""c":"FIRST",{key}:"SECOND""#));
        let mut socket = UnixStream::connect(state.join("architect.sock")).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        socket.write_all(format!("{wire}\n").as_bytes()).unwrap();
        let mut line = String::new();
        BufReader::new(socket).read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert!(reply["error"].as_str().unwrap().contains("duplicate JSON"));
    }
    assert_eq!(rpc(&state, Operation::List)["runs"], json!([]));
}

#[test]
fn tui_backend_message_roundtrip_uses_real_ipc_and_restores_actual_queue_receipts() {
    use architect::tui::backend::{Action, Backend, IpcBackend};
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let daemon = launch(&state);
    let created = rpc(
        &state,
        Operation::Mutate {
            command: Box::new(Command::Create {
                spec: fixture(temp.path()),
            }),
        },
    );
    let rid = created["id"].as_str().unwrap();
    let mut backend = IpcBackend::new(&state);
    let content = "Actual IPC message 界\nCheck boundaries";
    backend
        .act(
            rid,
            Action::SendMessage {
                target_agent_id: "Discovery:executor".into(),
                text: content.into(),
            },
        )
        .unwrap();
    let snapshot = backend.snapshot(rid).unwrap();
    assert_eq!(snapshot.messages.len(), 1);
    assert_eq!(snapshot.messages[0].text, content);
    assert_eq!(snapshot.messages[0].sender, "user");
    assert_eq!(snapshot.messages[0].state, "queued_next_turn");
    assert!(snapshot.messages[0].session.is_empty());
    let document = format!("message:{}", snapshot.messages[0].id);
    assert_eq!(ipc::read_document(&state, rid, &document).unwrap(), content);
    drop(daemon);
    let _replacement = launch(&state);
    let restored = backend.snapshot(rid).unwrap();
    assert_eq!(restored.messages[0].id, snapshot.messages[0].id);
    assert_eq!(restored.messages[0].state, "queued_next_turn");
    backend
        .act(
            rid,
            Action::ApproveStart {
                scope_hash: restored.scope_hashes["start"].clone(),
            },
        )
        .unwrap();
    backend.act(rid, Action::Advance).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let current = match backend.snapshot(rid) {
            Ok(snapshot) => Some(snapshot),
            Err(error)
                if error.to_string().contains("run changed between")
                    || error.to_string().contains("document changed") =>
            {
                None
            }
            Err(error) => panic!("{error}"),
        };
        if let Some(current) = current
            && current.status != "running"
        {
            assert_eq!(current.status, "awaiting_result");
            assert_eq!(current.messages[0].state, "delivered");
            assert!(!current.messages[0].session.is_empty());
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn tui_backend_rejects_controls_from_an_older_document_and_refreshes_without_mutation() {
    use architect::tui::backend::{Backend, IpcBackend};
    use std::os::unix::{fs::PermissionsExt, net::UnixListener};
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    ipc::private_directory(&state).unwrap();
    let runtime =
        Arc::new(Runtime::new(Store::open(&temp.path().join("state.db")).unwrap()).unwrap());
    let run = runtime
        .dispatch(
            &id(),
            Command::Create {
                spec: fixture(temp.path()),
            },
        )
        .unwrap();
    let listener = UnixListener::bind(state.join("architect.sock")).unwrap();
    listener.set_nonblocking(true).unwrap();
    std::fs::set_permissions(
        state.join("architect.sock"),
        std::fs::Permissions::from_mode(0o600),
    )
    .unwrap();
    let server_runtime = runtime.clone();
    let run_id = run.id.clone();
    let server = std::thread::spawn(move || {
        let mut operations = vec![];
        for index in 0..4 {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline);
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("{error}"),
                }
            };
            // BSD accept can inherit the polling listener's nonblocking flag.
            // The daemon serves blocking streams with bounded I/O timeouts.
            stream.set_nonblocking(false).unwrap();
            stream
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let request: Request = architect::transport::read(&mut BufReader::new(&stream))
                .unwrap()
                .unwrap();
            operations.push(match &request.operation {
                Operation::Snapshot { .. } => "snapshot",
                Operation::Read { .. } => "read",
                _ => panic!("snapshot must remain read-only"),
            });
            if index == 1 {
                server_runtime
                    .dispatch(
                        &id(),
                        Command::Pause {
                            run_id: run_id.clone(),
                        },
                    )
                    .unwrap();
            }
            let response = ipc::handle(&server_runtime, request);
            let mut bytes = serde_json::to_vec(&response).unwrap();
            bytes.push(b'\n');
            stream.write_all(&bytes).unwrap();
        }
        operations
    });
    let mut backend = IpcBackend::new(&state);
    assert!(
        backend
            .snapshot(&run.id)
            .unwrap_err()
            .to_string()
            .contains("run changed between")
    );
    let refreshed = backend.snapshot(&run.id).unwrap();
    assert_eq!(refreshed.status, "paused");
    assert_eq!(
        server.join().unwrap(),
        ["snapshot", "read", "snapshot", "read"]
    );
    assert!(
        runtime
            .store
            .lock()
            .unwrap()
            .get(&run.id)
            .unwrap()
            .approvals
            .is_empty()
    );
}
#[test]
fn large_unicode_run_has_small_control_replies_and_consistent_paginated_export() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    let _daemon = launch(&state);
    let source = "ą🦀\n".repeat(170_000);
    std::fs::write(temp.path().join("large.txt"), &source).unwrap();
    let mut spec = fixture(temp.path());
    spec.input_files = vec!["large.txt".into()];
    let created = rpc(
        &state,
        Operation::Mutate {
            command: Box::new(Command::Create { spec }),
        },
    );
    assert!(serde_json::to_vec(&created).unwrap().len() < 4096);
    let rid = created["id"].as_str().unwrap();
    let snapshot = rpc(&state, Operation::Snapshot { run_id: rid.into() });
    rpc(
        &state,
        Operation::Mutate {
            command: Box::new(Command::ApproveStart {
                run_id: rid.into(),
                scope_hash: snapshot["scope_hashes"]["start"].as_str().unwrap().into(),
            }),
        },
    );
    rpc(
        &state,
        Operation::Mutate {
            command: Box::new(Command::Advance { run_id: rid.into() }),
        },
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let snap = rpc(&state, Operation::Snapshot { run_id: rid.into() });
        assert!(serde_json::to_vec(&snap).unwrap().len() < 8192);
        if snap["run"]["status"] != "running" {
            assert_eq!(snap["run"]["status"], "awaiting_result", "{snap}");
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let exported = ipc::read_document(&state, rid, "run").unwrap();
    assert!(exported.len() > architect::transport::MAX_FRAME);
    let run: Run = serde_json::from_str(&exported).unwrap();
    assert_eq!(run.input_snapshot["large.txt"], source);
    let cli = Process::new(env!("CARGO_BIN_EXE_antislop"))
        .args(["snapshot", state.to_str().unwrap(), rid])
        .output()
        .unwrap();
    assert!(cli.status.success(), "{:?}", cli);
    let bad = ipc::call(
        &state,
        &request(Operation::Read {
            run_id: rid.into(),
            document: "run".into(),
            offset: 1,
            expected_hash: Some("stale".into()),
        }),
    )
    .unwrap();
    assert!(bad.error.unwrap().contains("document changed"));
}
#[test]
fn run_lists_are_paginated_without_bodies() {
    let temp = tempfile::tempdir().unwrap();
    let runtime =
        Arc::new(Runtime::new(Store::open(&temp.path().join("state.db")).unwrap()).unwrap());
    for _ in 0..103 {
        runtime
            .dispatch(
                &id(),
                Command::Create {
                    spec: fixture(temp.path()),
                },
            )
            .unwrap();
    }
    let page = ipc::handle(&runtime, request(Operation::List))
        .data
        .unwrap();
    assert_eq!(page["runs"].as_array().unwrap().len(), 100);
    let cursor = page["next_cursor"].as_str().unwrap();
    let next = ipc::handle(
        &runtime,
        request(Operation::ListPage {
            after: cursor.into(),
        }),
    )
    .data
    .unwrap();
    assert_eq!(next["runs"].as_array().unwrap().len(), 3);
    assert!(next["next_cursor"].is_null());
    assert!(page["runs"][0].get("spec").is_none());
}

#[test]
fn changed_run_rejects_an_old_cursor_before_new_document_offset_validation() {
    let root = tempfile::tempdir().unwrap();
    let runtime =
        Arc::new(Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap());
    let mut run = runtime
        .dispatch(
            &id(),
            Command::Create {
                spec: fixture(root.path()),
            },
        )
        .unwrap();
    run.diagnostic = Some("long preceding version".repeat(ipc::DOCUMENT_CHUNK / 5));
    runtime
        .store
        .lock()
        .unwrap()
        .save(&run, "long", None)
        .unwrap();
    let first = ipc::handle(
        &runtime,
        request(Operation::Read {
            run_id: run.id.clone(),
            document: "run".into(),
            offset: 0,
            expected_hash: None,
        }),
    )
    .data
    .unwrap();
    let offset = first["next_offset"].as_u64().unwrap() as usize;
    let old_hash = first["hash"].as_str().unwrap().to_owned();
    run.diagnostic = None;
    runtime
        .store
        .lock()
        .unwrap()
        .save(&run, "short", None)
        .unwrap();
    let stale = ipc::handle(
        &runtime,
        request(Operation::Read {
            run_id: run.id.clone(),
            document: "run".into(),
            offset,
            expected_hash: Some(old_hash),
        }),
    );
    assert_eq!(
        stale.error.as_deref(),
        Some("document changed; restart from offset zero")
    );
    let fresh = ipc::handle(
        &runtime,
        request(Operation::Read {
            run_id: run.id.clone(),
            document: "run".into(),
            offset: 0,
            expected_hash: None,
        }),
    )
    .data
    .unwrap();
    let invalid = ipc::handle(
        &runtime,
        request(Operation::Read {
            run_id: run.id.clone(),
            document: "run".into(),
            offset,
            expected_hash: Some(fresh["hash"].as_str().unwrap().into()),
        }),
    );
    assert_eq!(invalid.error.as_deref(), Some("invalid document offset"));
}
