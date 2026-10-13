use architect::{domain::*, id, ipc::{self, Operation, Request}, runtime::{Command, Runtime}, storage::Store};
use serde_json::{Value, json};
use std::{io::{BufRead, BufReader, Write}, os::unix::net::UnixStream, process::{Child, Command as Process, Stdio}, sync::Arc, time::{Duration, Instant}};
struct Daemon(Child);
impl Drop for Daemon { fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); } }
fn launch(state: &std::path::Path) -> Daemon {
    let process = Process::new(env!("CARGO_BIN_EXE_architect")).args(["daemon", state.to_str().unwrap()]).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    let mut daemon = Daemon(process);
    let deadline = Instant::now() + Duration::from_secs(10);
    while UnixStream::connect(state.join("architect.sock")).is_err() {
        assert!(daemon.0.try_wait().unwrap().is_none(), "daemon exited");
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    daemon
}
fn request(operation: Operation) -> Request { Request { version:ipc::PROTOCOL_VERSION, request_id:id(), idempotency_key:Some(id()), operation } }
fn fixture(root: &std::path::Path) -> RunSpec {
    let mut spec: RunSpec = serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace=root.to_str().unwrap().into(); spec
}
fn rpc(state: &std::path::Path, operation: Operation) -> Value {
    let response=ipc::call(state,&request(operation)).unwrap(); assert!(response.error.is_none(), "{:?}",response.error);response.data.unwrap()
}
#[test]
fn raw_socket_rejects_duplicate_criteria_and_escaped_duplicate_keys() {
    let temp=tempfile::tempdir().unwrap();let state=temp.path().join("state");let _daemon=launch(&state);
    for key in [r#""c""#,r#""\u0063""#] {
        let mut spec=fixture(temp.path());spec.requirements[0].criteria=std::collections::BTreeMap::from([("c".into(),"FIRST".into())]);
        let q=request(Operation::Mutate{command:Box::new(Command::Create{spec})});
        let wire=serde_json::to_string(&q).unwrap().replace(r#""c":"FIRST""#,&format!(r#""c":"FIRST",{key}:"SECOND""#));
        let mut socket=UnixStream::connect(state.join("architect.sock")).unwrap();socket.set_read_timeout(Some(Duration::from_secs(5))).unwrap();socket.write_all(format!("{wire}\n").as_bytes()).unwrap();
        let mut line=String::new();BufReader::new(socket).read_line(&mut line).unwrap();let reply:Value=serde_json::from_str(&line).unwrap();
        assert!(reply["error"].as_str().unwrap().contains("duplicate JSON"));
    }
    assert_eq!(rpc(&state,Operation::List)["runs"],json!([]));
}
#[test]
fn large_unicode_run_has_small_control_replies_and_consistent_paginated_export() {
    let temp=tempfile::tempdir().unwrap();let state=temp.path().join("state");let _daemon=launch(&state);
    let source="ą🦀\n".repeat(170_000);std::fs::write(temp.path().join("large.txt"),&source).unwrap();
    let mut spec=fixture(temp.path());spec.input_files=vec!["large.txt".into()];
    let created=rpc(&state,Operation::Mutate{command:Box::new(Command::Create{spec})});
    assert!(serde_json::to_vec(&created).unwrap().len()<4096);
    let rid=created["id"].as_str().unwrap();
    let snapshot=rpc(&state,Operation::Snapshot{run_id:rid.into()});
    rpc(&state,Operation::Mutate{command:Box::new(Command::ApproveStart{run_id:rid.into(),scope_hash:snapshot["scope_hashes"]["start"].as_str().unwrap().into()})});
    rpc(&state,Operation::Mutate{command:Box::new(Command::Advance{run_id:rid.into()})});
    let deadline=Instant::now()+Duration::from_secs(30);
    loop {
        let snap=rpc(&state,Operation::Snapshot{run_id:rid.into()});
        assert!(serde_json::to_vec(&snap).unwrap().len()<8192);
        if snap["run"]["status"]!="running" { assert_eq!(snap["run"]["status"],"awaiting_result", "{snap}"); break; }
        assert!(Instant::now()<deadline); std::thread::sleep(Duration::from_millis(10));
    }
    let exported=ipc::read_document(&state,rid,"run").unwrap();assert!(exported.len()>architect::transport::MAX_FRAME);
    let run:Run=serde_json::from_str(&exported).unwrap();assert_eq!(run.input_snapshot["large.txt"],source);
    let cli=Process::new(env!("CARGO_BIN_EXE_architect")).args(["snapshot",state.to_str().unwrap(),rid]).output().unwrap();assert!(cli.status.success(),"{:?}",cli);
    let bad=ipc::call(&state,&request(Operation::Read{run_id:rid.into(),document:"run".into(),offset:1,expected_hash:Some("stale".into())})).unwrap();assert!(bad.error.unwrap().contains("document changed"));
}
#[test]
fn run_lists_are_paginated_without_bodies() {
    let temp=tempfile::tempdir().unwrap();let runtime=Arc::new(Runtime::new(Store::open(&temp.path().join("state.db")).unwrap()).unwrap());
    for _ in 0..103 { runtime.dispatch(&id(),Command::Create{spec:fixture(temp.path())}).unwrap(); }
    let page=ipc::handle(&runtime,request(Operation::List)).data.unwrap();assert_eq!(page["runs"].as_array().unwrap().len(),100);
    let cursor=page["next_cursor"].as_str().unwrap();let next=ipc::handle(&runtime,request(Operation::ListPage{after:cursor.into()})).data.unwrap();assert_eq!(next["runs"].as_array().unwrap().len(),3);assert!(next["next_cursor"].is_null());
    assert!(page["runs"][0].get("spec").is_none());
}
