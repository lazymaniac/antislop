//! Independent adversarial checks for explicitly delegated local execution.
use architect::{
    domain::{Run, RunSpec, Status},
    drive, id,
    ipc::{self, Operation, Request, Response},
};
use std::{
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
struct Daemon(Child);
impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn daemon(state: &Path) -> Daemon {
    let child = Command::new(env!("CARGO_BIN_EXE_antislop"))
        .arg("daemon")
        .arg(state)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut daemon = Daemon(child);
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if ipc::call(
            state,
            &Request {
                version: ipc::PROTOCOL_VERSION,
                request_id: id(),
                idempotency_key: None,
                operation: Operation::List,
            },
        )
        .is_ok()
        {
            return daemon;
        }
        assert!(
            daemon.0.try_wait().unwrap().is_none(),
            "daemon exited during startup"
        );
        assert!(Instant::now() < deadline, "daemon startup timeout");
        std::thread::sleep(Duration::from_millis(20));
    }
}
fn spec(root: &Path) -> RunSpec {
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.to_string_lossy().into();
    spec
}
fn runs(state: &Path) -> Vec<Run> {
    let response = ipc::call(
        state,
        &Request {
            version: ipc::PROTOCOL_VERSION,
            request_id: id(),
            idempotency_key: None,
            operation: Operation::List,
        },
    )
    .unwrap();
    assert!(response.error.is_none());
    response.data.unwrap()["runs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            serde_json::from_str(
                &ipc::read_document(state, r["id"].as_str().unwrap(), "run").unwrap(),
            )
            .unwrap()
        })
        .collect()
}
#[test]
fn real_daemon_fake_driver_completes_every_stage_with_independent_reviews() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    let _daemon = daemon(&state);
    let spec_path = root.path().join("spec.json");
    std::fs::write(&spec_path, serde_json::to_vec(&spec(root.path())).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_antislop"))
        .arg("run")
        .arg(&state)
        .arg(&spec_path)
        .arg("--approve-scoped")
        .arg("0")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(receipt["status"], "completed");
    let run = runs(&state).pop().unwrap();
    assert_eq!(receipt["id"], run.id);
    assert_eq!(run.status, Status::Completed);
    assert_eq!(run.spec_revision, 1);
    assert_eq!(run.artifacts.len(), 6);
    assert_eq!(run.reviews.len(), 6);
    assert_eq!(run.approvals.len(), 12);
    assert!(!run.reconciliation_required);
    for (artifact, review) in run.artifacts.iter().zip(&run.reviews) {
        assert_eq!(artifact.hash, review.artifact_hash);
        assert_eq!(artifact.session_id, review.executor_session);
        assert_ne!(review.executor_session, review.verifier_session);
        assert!(
            review.quality.as_ref().unwrap().synthetic,
            "fake results must remain labelled synthetic"
        );
    }
    assert_eq!(runs(&state)[0].status, Status::Completed);
}
#[test]
fn verifier_rejection_stops_without_a_repair_grant_or_false_completion() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    let _daemon = daemon(&state);
    let mut spec = spec(root.path());
    spec.objective.push_str(" [reject]");
    let error = drive::run(&state, spec, 0, Duration::from_secs(15))
        .unwrap_err()
        .to_string();
    assert!(error.contains("ChangesRequested"), "{error}");
    let run = runs(&state).pop().unwrap();
    assert_eq!(run.status, Status::ChangesRequested);
    assert_eq!(run.attempt, 1);
    assert_eq!(run.reviews.len(), 1);
    assert_eq!(run.approvals.len(), 1);
    assert_eq!(run.approvals[0].action, "start");
    assert!(!run.reviews[0].findings.is_empty());
}
#[test]
fn repair_grant_is_exact_and_does_not_authorize_unbounded_retries() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    let _daemon = daemon(&state);
    let mut spec = spec(root.path());
    spec.objective.push_str(" [reject]");
    let error = drive::run(&state, spec, 1, Duration::from_secs(15))
        .unwrap_err()
        .to_string();
    assert!(error.contains("ChangesRequested"), "{error}");
    let run = runs(&state).pop().unwrap();
    assert_eq!(run.attempt, 2);
    assert_eq!(
        run.approvals
            .iter()
            .filter(|a| a.action == "repair")
            .count(),
        1
    );
    assert_ne!(run.status, Status::Completed);
}
#[test]
fn cli_without_scoped_authorization_creates_no_run_or_local_effect() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    let _daemon = daemon(&state);
    let path = root.path().join("spec.json");
    let mut spec = spec(root.path());
    spec.check_command = vec!["touch".into(), "unauthorized-effect".into()];
    std::fs::write(&path, serde_json::to_vec(&spec).unwrap()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_antislop"))
        .arg("run")
        .arg(&state)
        .arg(&path)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("--approve-scoped"));
    assert!(runs(&state).is_empty());
    assert!(!root.path().join("unauthorized-effect").exists());
}
#[test]
fn pinned_input_change_stops_for_reconciliation_even_with_repairs_granted() {
    let root = tempfile::tempdir().unwrap();
    let state = root.path().join("state");
    let _daemon = daemon(&state);
    std::fs::write(root.path().join("pinned.txt"), "original").unwrap();
    let mut spec = spec(root.path());
    spec.input_files = vec!["pinned.txt".into()];
    spec.check_command = vec![
        "python3".into(),
        "-c".into(),
        "from pathlib import Path; Path('pinned.txt').write_text('unreviewed change')".into(),
    ];
    let error = drive::run(&state, spec, 5, Duration::from_secs(15))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("uncertain effects require explicit reconciliation"),
        "{error}"
    );
    let run = runs(&state).pop().unwrap();
    assert_eq!(run.status, Status::Blocked);
    assert!(run.reconciliation_required);
    assert_eq!(run.attempt, 1);
    assert_eq!(run.approvals.len(), 1);
    assert!(
        !run.approvals
            .iter()
            .any(|a| a.action == "repair" || a.action == "reconcile")
    );
}
#[test]
fn scope_drift_in_persisted_document_never_receives_an_approval() {
    use std::{
        io::{BufReader, Write},
        os::unix::{fs::PermissionsExt, net::UnixListener},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    for revision_only in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state = root.path().join("state");
        ipc::private_directory(&state).unwrap();
        let original = spec(root.path());
        let mut changed = Run::new("changed-run".into(), original.clone()).unwrap();
        if revision_only {
            changed.spec_revision = 2;
        } else {
            changed.spec.objective = "Unapproved replacement objective".into();
        }
        let document = serde_json::to_string(&changed).unwrap();
        let digest = architect::hash(&document);
        let listener = UnixListener::bind(state.join("architect.sock")).unwrap();
        std::fs::set_permissions(
            state.join("architect.sock"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let mutations = Arc::new(AtomicUsize::new(0));
        let observed = mutations.clone();
        let server = std::thread::spawn(move || {
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let request: Request = architect::transport::read(&mut BufReader::new(&stream))
                    .unwrap()
                    .unwrap();
                let data = match request.operation {
                    Operation::Mutate { command } => {
                        observed.fetch_add(1, Ordering::SeqCst);
                        assert!(
                            matches!(*command, architect::runtime::Command::Create { .. }),
                            "no delegated action may follow scope drift"
                        );
                        serde_json::json!({"id":"changed-run"})
                    }
                    Operation::Snapshot { .. } => {
                        serde_json::json!({"run":{"status":"awaiting_start","stage":"discovery"}})
                    }
                    Operation::Read { offset, .. } => {
                        assert_eq!(offset, 0);
                        serde_json::json!({"hash":digest,"chunk":document,"next_offset":null})
                    }
                    _ => panic!("unexpected driver operation"),
                };
                let response = Response {
                    version: ipc::PROTOCOL_VERSION,
                    request_id: request.request_id,
                    data: Some(data),
                    error: None,
                };
                serde_json::to_writer(&mut stream, &response).unwrap();
                stream.write_all(b"\n").unwrap();
            }
        });
        let error = drive::run(&state, original, 5, Duration::from_secs(10))
            .unwrap_err()
            .to_string();
        server.join().unwrap();
        assert!(error.contains("task scope changed"), "{error}");
        assert_eq!(mutations.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn selfhost_acceptance_rejects_exact_output_regressions_even_when_python_is_optimized() {
    let python = r#"
import ast, subprocess
from pathlib import Path
module=ast.parse(Path('scripts/selfhost.py').read_text())
generated=next(n.args[0].value for n in ast.walk(module) if isinstance(n,ast.Call) and isinstance(n.func,ast.Attribute) and n.func.attr=='write_text' and n.args and isinstance(n.args[0],ast.Constant) and isinstance(n.args[0].value,str) and 'result.stdout' in n.args[0].value)
import tomllib
version=tomllib.loads(Path('Cargo.toml').read_text())['package']['version']
original=subprocess.run
for optimization in [0,2]:
 for case in ['correct','no-newline','extra-space','stderr','version-failed','help-failed']:
  def stub(argv,**kwargs):
   if '--version' in argv:
    code=1 if case=='version-failed' else 0
    stdout=f'antislop {version}'+('' if case=='no-newline' else ' \n' if case=='extra-space' else '\n')
    return subprocess.CompletedProcess(argv,code,stdout,'noise' if case=='stderr' else '')
   if '--help' in argv:
    return subprocess.CompletedProcess(argv,1 if case=='help-failed' else 0,'antislop','')
   return subprocess.CompletedProcess(argv,0,'','')
  subprocess.run=stub
  rejected=False
  try:
   exec(compile(generated,'selfhost_check.py','exec',optimize=optimization),{'__file__':str(Path.cwd()/'selfhost_check.py')})
  except (AssertionError,SystemExit): rejected=True
  finally: subprocess.run=original
  if rejected != (case!='correct'): raise SystemExit(f'acceptance mismatch: {case}, optimization {optimization}, rejected={rejected}')
print('All 12 exact-output and optimized acceptance scenarios passed')
"#;
    let output = Command::new("python3")
        .arg("-c")
        .arg(python)
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
