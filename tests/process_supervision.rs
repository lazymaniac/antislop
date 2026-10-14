use architect::{
    domain::{Run, RunSpec},
    id,
    ipc::{Operation, Request, call},
    process::{LeaseScope, SupervisedChild},
    runtime::Command,
};
use std::{
    os::{fd::AsRawFd, unix::fs::PermissionsExt},
    path::Path,
    process::{Child, Command as OsCommand, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for subprocess"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn spec(root: &Path) -> RunSpec {
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.to_string_lossy().into();
    spec
}
fn delayed_descendant() -> &'static str {
    "import subprocess,pathlib,time\nsubprocess.Popen(['python3','-c',\"import time,pathlib; time.sleep(1); pathlib.Path('late-effect').write_text('orphan')\"])\npathlib.Path('started').write_text('ready')\n"
}
fn no_late_effect(root: &Path) {
    std::thread::sleep(Duration::from_millis(1200));
    assert!(
        !root.join("late-effect").exists(),
        "descendant survived supervision"
    );
}

#[test]
fn leader_exit_terminates_descendants_and_closes_output_pipes() {
    let root = tempfile::tempdir().unwrap();
    let (success, _) = architect::workspace::check(
        root.path(),
        &["python3".into(), "-c".into(), delayed_descendant().into()],
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert!(success);
    no_late_effect(root.path());
}

#[test]
fn check_cancellation_terminates_descendants() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().to_path_buf();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let worker = std::thread::spawn(move || {
        architect::workspace::check(
            &workspace,
            &[
                "python3".into(),
                "-c".into(),
                format!("{}time.sleep(10)\n", delayed_descendant()),
            ],
            worker_cancelled,
        )
    });
    until(|| root.path().join("started").exists());
    cancelled.store(true, Ordering::SeqCst);
    assert!(worker.join().unwrap().is_err());
    no_late_effect(root.path());
}

struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn mutate(state: &Path, command: Command) -> Run {
    let response = call(
        state,
        &Request {
            version: architect::ipc::PROTOCOL_VERSION,
            request_id: id(),
            idempotency_key: Some(id()),
            operation: Operation::Mutate {
                command: Box::new(command),
            },
        },
    )
    .unwrap();
    assert!(response.error.is_none(), "{:?}", response.error);
    let summary = response.data.unwrap();
    serde_json::from_str(
        &architect::ipc::read_document(state, summary["id"].as_str().unwrap(), "run").unwrap(),
    )
    .unwrap()
}

#[test]
fn sigkill_daemon_terminates_uncooperative_check_and_descendants() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    std::fs::write(
        root.path().join("check.py"),
        format!("{}time.sleep(10)\n", delayed_descendant()),
    )
    .unwrap();
    let mut run_spec = spec(root.path());
    run_spec.input_files = vec!["check.py".into()];
    run_spec.check_command = vec!["python3".into(), "check.py".into()];
    let mut daemon = Daemon(
        OsCommand::new(env!("CARGO_BIN_EXE_antislop"))
            .arg("daemon")
            .arg(&state)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    until(|| {
        call(
            &state,
            &Request {
                version: architect::ipc::PROTOCOL_VERSION,
                request_id: id(),
                idempotency_key: None,
                operation: Operation::List,
            },
        )
        .is_ok()
    });
    let run = mutate(&state, Command::Create { spec: run_spec });
    mutate(
        &state,
        Command::ApproveStart {
            run_id: run.id.clone(),
            scope_hash: run.scope_hash("start"),
        },
    );
    mutate(&state, Command::Advance { run_id: run.id });
    until(|| root.path().join("started").exists());
    daemon.0.kill().unwrap();
    daemon.0.wait().unwrap();
    no_late_effect(root.path());
}

#[test]
fn codex_exit_terminates_descendants() {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("fake-codex");
    std::fs::write(&binary, format!("#!/usr/bin/env python3\nimport sys,json,os,pathlib\nif '--version' in sys.argv:\n print('codex-cli 0.162.1')\n sys.exit(0)\nos.chdir(pathlib.Path(__file__).parent)\nsys.stdin.read()\n{}print(json.dumps({{'type':'thread.started','thread_id':'fake-session'}}),flush=True)\nprint(json.dumps({{'type':'item.completed','item':{{'type':'agent_message','text':'{{\"ok\":true}}'}}}}),flush=True)\nprint(json.dumps({{'type':'turn.completed'}}),flush=True)\n", delayed_descendant())).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let provider = architect::providers::codex::CodexProvider {
        binary: Some(binary.to_string_lossy().into()),
        ..Default::default()
    };
    let run = Run::new(id(), spec(root.path())).unwrap();
    let result = provider
        .turn(
            &run,
            "test",
            &serde_json::json!({}),
            None,
            Arc::new(AtomicBool::new(false)),
            &mut |_| Ok(()),
        )
        .unwrap();
    assert_eq!(result, ("fake-session".into(), "{\"ok\":true}".into()));
    no_late_effect(root.path());
}

#[test]
fn codex_cancellation_terminates_descendants() {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("fake-codex");
    std::fs::write(&binary, format!("#!/usr/bin/env python3\nimport sys,os,pathlib\nif '--version' in sys.argv:\n print('codex-cli 0.162.1')\n sys.exit(0)\nos.chdir(pathlib.Path(__file__).parent)\nsys.stdin.read()\n{}time.sleep(10)\n", delayed_descendant())).unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let provider = architect::providers::codex::CodexProvider {
        binary: Some(binary.to_string_lossy().into()),
        ..Default::default()
    };
    let run = Run::new(id(), spec(root.path())).unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let worker = std::thread::spawn(move || {
        provider.turn(
            &run,
            "test",
            &serde_json::json!({}),
            None,
            worker_cancelled,
            &mut |_| Ok(()),
        )
    });
    until(|| root.path().join("started").exists());
    cancelled.store(true, Ordering::SeqCst);
    assert!(worker.join().unwrap().is_err());
    no_late_effect(root.path());
}

#[test]
fn codex_cancellation_also_covers_version_probe() {
    let root = tempfile::tempdir().unwrap();
    let binary = root.path().join("fake-codex");
    std::fs::write(
        &binary,
        format!(
            "#!/usr/bin/env python3\nimport os\nos.chdir({})\n{}time.sleep(10)\n",
            serde_json::to_string(root.path().to_str().unwrap()).unwrap(),
            delayed_descendant()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700)).unwrap();
    let provider = architect::providers::codex::CodexProvider {
        binary: Some(binary.to_string_lossy().into()),
        ..Default::default()
    };
    let run = Run::new(id(), spec(root.path())).unwrap();
    let cancelled = Arc::new(AtomicBool::new(false));
    let worker_cancelled = cancelled.clone();
    let worker = std::thread::spawn(move || {
        provider.turn(
            &run,
            "test",
            &serde_json::json!({}),
            None,
            worker_cancelled,
            &mut |_| Ok(()),
        )
    });
    until(|| root.path().join("started").exists());
    cancelled.store(true, Ordering::SeqCst);
    assert!(worker.join().unwrap().is_err());
    no_late_effect(root.path());
}

#[test]
fn guardian_holds_workspace_lease_until_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let lease_paths: Vec<_> = (0..24)
        .map(|index| root.path().join(format!("lease-{index}")))
        .collect();
    let leases: Vec<_> = lease_paths
        .iter()
        .map(|path| std::fs::File::create(path).unwrap())
        .collect();
    for lease in &leases {
        assert_eq!(
            unsafe { libc::flock(lease.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
    }
    let scope = LeaseScope::new(&leases.iter().map(AsRawFd::as_raw_fd).collect::<Vec<_>>());
    let mut command = architect::process::command("python3").unwrap();
    command
        .args(["-c", "import time; time.sleep(10)"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = SupervisedChild::spawn(&mut command).unwrap();
    drop(scope);
    drop(leases);
    let competitors: Vec<_> = lease_paths
        .iter()
        .map(|path| std::fs::File::open(path).unwrap())
        .collect();
    for competitor in &competitors {
        assert_ne!(
            unsafe { libc::flock(competitor.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0,
            "guardian released the lease early"
        );
    }
    child.cancel().unwrap();
    for competitor in &competitors {
        assert_eq!(
            unsafe { libc::flock(competitor.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
    }
}

#[test]
fn missing_companion_fails_closed() {
    if std::env::var_os("ARCHITECT_MISSING_COMPANION_TEST").is_some() {
        assert!(architect::process::command("python3").is_err());
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let copied = root.path().join("isolated-tests");
    std::fs::copy(std::env::current_exe().unwrap(), &copied).unwrap();
    let mut command = OsCommand::new(copied);
    command
        .args(["--exact", "missing_companion_fails_closed"])
        .env("ARCHITECT_MISSING_COMPANION_TEST", "1");
    // A concurrent test's fork can briefly inherit fs::copy's writable FD
    // until exec closes it. Linux rejects this fixture's exec with ETXTBSY;
    // no fixture code has started, so only that launch error is retryable.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut child = loop {
        assert!(
            Instant::now() < deadline,
            "fixture exec remained ETXTBSY for five seconds"
        );
        match command.spawn() {
            Ok(child) => break child,
            Err(error)
                if error.raw_os_error() == Some(libc::ETXTBSY) && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(error) => panic!("isolated companion-gate fixture failed to launch: {error}"),
        }
    };
    let status = child.wait().unwrap();
    assert!(status.success());
}

#[test]
fn codex_frames_reject_duplicate_json_keys() {
    let mut frame = std::io::Cursor::new(
        br#"{"type":"thread.started","thread_id":"first","thread_id":"second"}"#,
    );
    assert!(architect::providers::codex::read_frame(&mut frame).is_err());
    let mut frame = std::io::Cursor::new(br#"{"type":"item.completed","item":{"type":"agent_message","text":"first","text":"second"}}"#);
    assert!(architect::providers::codex::read_frame(&mut frame).is_err());
}

#[test]
fn supervised_check_preserves_signal_termination_instead_of_reporting_exit_one() {
    let root = tempfile::tempdir().unwrap();
    let (_, output) = architect::workspace::check(
        root.path(),
        &[
            "python3".into(),
            "-c".into(),
            "import os,signal; os.kill(os.getpid(),signal.SIGTERM)".into(),
        ],
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let receipt: serde_json::Value = serde_json::from_str(&output).unwrap();
    assert!(receipt["exit_code"].is_null());
    assert_eq!(receipt["signal"], libc::SIGTERM);
    assert_eq!(receipt["truncated"], false);
}
