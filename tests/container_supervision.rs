use architect::process::{
    LeaseScope, SupervisedChild,
    container::{ContainerResource, OWNERSHIP_LABEL},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs,
    os::{fd::AsRawFd, unix::fs::PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(25);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "owned container fixture timed out"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn state(root: &Path, resource: &ContainerResource, unavailable: bool) {
    fs::write(
        root.join("next.json"),
        serde_json::to_vec(
            &json!({"id":resource.container_id,"image":resource.image_id,
            "token":resource.owner_token,"unavailable":unavailable}),
        )
        .unwrap(),
    )
    .unwrap();
    fs::rename(root.join("next.json"), root.join("state.json")).unwrap();
}

fn mock(root: &Path) -> ContainerResource {
    let binary = root.join("engine");
    fs::write(
        &binary,
        r#"#!/usr/bin/env python3
import json,pathlib,sys,time
p=pathlib.Path(__file__).parent
with (p/'called').open('a') as f:f.write('x')
s=json.loads((p/'state.json').read_text())
a=sys.argv[1:]
if s['unavailable']:
 with (p/'unavailable-seen').open('a') as f:f.write('x')
 sys.exit(125)
if a[:2]==['container','exists']:
 sys.exit(1 if (p/'removed').exists() else 0)
if a[:2]==['container','inspect']:
 with (p/'inspected').open('a') as f:f.write('x')
 assert a[2]=='--format' and '{{json .ID}}' in a[3] and '"io.antislop.check-invocation"' in a[3]
 if (p/'bad-inspect').exists():
  mode=(p/'bad-inspect').read_text()
  print('DO_NOT_LOG_ENGINE_SECRET',file=sys.stderr)
  if mode=='duplicate':print('{"id":"first","id":"second"}')
  elif mode=='oversized':print('x'*8192)
  elif mode=='missing':print('{}')
  else:print('[]')
  sys.exit(0)
 print(json.dumps({'id':s['id'],'image':s['image'],'token':s['token']}))
 sys.exit(0)
if a[:2]==['container','rm']:
 assert a[2:]==['--force','--time','0',s['id']]
 (p/'remove-started').touch()
 while (p/'block-remove').exists():time.sleep(.02)
 (p/'removed').touch()
 sys.exit(0)
sys.exit(125)
"#,
    )
    .unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let resource = ContainerResource {
        binary,
        container_id: "a".repeat(64),
        image_id: format!("sha256:{}", "b".repeat(64)),
        owner_token: architect::id(),
    };
    state(root, &resource, false);
    resource
}

fn lock(root: &Path) -> fs::File {
    let file = fs::File::create(root.join("workspace.lease")).unwrap();
    assert_eq!(
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    file
}
fn held(file: &fs::File) -> bool {
    unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) != 0 }
}

#[test]
fn invalid_resource_selectors_are_refused_before_a_process_can_start() {
    let root = tempfile::tempdir().unwrap();
    let original = mock(root.path());
    for (field, value) in [
        ("container_id", "--all"),
        ("container_id", "aabb"),
        ("image_id", "rust:latest"),
        ("owner_token", "unbound"),
    ] {
        let mut value_json = serde_json::to_value(&original).unwrap();
        value_json[field] = json!(value);
        let resource: ContainerResource = serde_json::from_value(value_json).unwrap();
        assert!(architect::process::command_with_container("true", &resource).is_err());
    }
    assert!(!root.path().join("called").exists());
}

#[test]
fn direct_internal_invocation_without_private_lifeline_cannot_remove_a_resource() {
    let root = tempfile::tempdir().unwrap();
    let resource = mock(root.path());
    let output = Command::new(env!("CARGO_BIN_EXE_antislop"))
        .args([
            "__supervise-container",
            &serde_json::to_string(&resource).unwrap(),
            "true",
        ])
        .env_remove("ARCHITECT_SUPERVISOR_LIFELINE")
        .env_remove("ARCHITECT_SUPERVISOR_LEASE_FDS")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!root.path().join("called").exists());
}

#[test]
fn leader_exit_waits_for_container_removal_and_preserves_its_exit_status() {
    let root = tempfile::tempdir().unwrap();
    let resource = mock(root.path());
    fs::write(root.path().join("block-remove"), b"hold").unwrap();
    let lease = lock(root.path());
    let scope = LeaseScope::new(&[lease.as_raw_fd()]);
    let mut command = architect::process::command_with_container("python3", &resource).unwrap();
    command
        .args(["-c", "raise SystemExit(7)"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = SupervisedChild::spawn(&mut command).unwrap();
    drop(scope);
    drop(lease);
    until(|| root.path().join("remove-started").exists());
    let competitor = fs::File::open(root.path().join("workspace.lease")).unwrap();
    assert!(
        held(&competitor),
        "container cleanup released its lease early"
    );
    assert!(child.child.try_wait().unwrap().is_none());
    fs::remove_file(root.path().join("block-remove")).unwrap();
    assert_eq!(child.child.wait().unwrap().code(), Some(7));
    assert!(root.path().join("removed").exists());
    assert!(!held(&competitor));
}

#[test]
fn unavailable_engine_and_foreign_ownership_never_count_as_cleanup() {
    for wrong in ["engine", "id", "image", "token"] {
        let root = tempfile::tempdir().unwrap();
        let resource = mock(root.path());
        let mut mismatch = resource.clone();
        match wrong {
            "id" => mismatch.container_id = "c".repeat(64),
            "image" => mismatch.image_id = format!("sha256:{}", "d".repeat(64)),
            "token" => mismatch.owner_token = architect::id(),
            _ => (),
        }
        state(root.path(), &mismatch, wrong == "engine");
        let lease = lock(root.path());
        let scope = LeaseScope::new(&[lease.as_raw_fd()]);
        let mut command = architect::process::command_with_container("true", &resource).unwrap();
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = SupervisedChild::spawn(&mut command).unwrap();
        drop(scope);
        drop(lease);
        let observed = root.path().join(if wrong == "engine" {
            "unavailable-seen"
        } else {
            "inspected"
        });
        until(|| fs::metadata(&observed).is_ok_and(|m| m.len() >= 2));
        let competitor = fs::File::open(root.path().join("workspace.lease")).unwrap();
        assert!(held(&competitor));
        assert!(child.child.try_wait().unwrap().is_none());
        assert!(!root.path().join("remove-started").exists());
        state(root.path(), &resource, false);
        assert!(child.child.wait().unwrap().success());
        assert!(root.path().join("removed").exists());
        assert!(!held(&competitor));
    }
}

#[derive(Serialize, Deserialize)]
struct ParentFixture {
    resource: ContainerResource,
    program: PathBuf,
    argv: Vec<String>,
}

#[test]
#[ignore = "internal owned subprocess fixture; requires ARCHITECT_CONTAINER_PARENT_ROOT"]
fn container_parent_fixture() {
    let root = PathBuf::from(std::env::var_os("ARCHITECT_CONTAINER_PARENT_ROOT").unwrap());
    let input: ParentFixture =
        serde_json::from_slice(&fs::read(root.join("parent.json")).unwrap()).unwrap();
    let lease = lock(&root);
    let _scope = LeaseScope::new(&[lease.as_raw_fd()]);
    let mut command =
        architect::process::command_with_container(&input.program, &input.resource).unwrap();
    command
        .args(input.argv)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = SupervisedChild::spawn(&mut command).unwrap();
    fs::write(root.join("parent-started"), b"ready").unwrap();
    child.child.wait().unwrap();
}

fn parent(root: &Path, input: &ParentFixture) -> std::process::Child {
    fs::write(root.join("parent.json"), serde_json::to_vec(input).unwrap()).unwrap();
    Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "container_parent_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("ARCHITECT_CONTAINER_PARENT_ROOT", root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

#[test]
fn sigkill_parent_keeps_lease_until_owned_container_removal_finishes() {
    let root = tempfile::tempdir().unwrap();
    let resource = mock(root.path());
    fs::write(root.path().join("block-remove"), b"hold").unwrap();
    let mut process = parent(
        root.path(),
        &ParentFixture {
            resource,
            program: "python3".into(),
            argv: vec!["-c".into(), "import time;time.sleep(30)".into()],
        },
    );
    until(|| root.path().join("parent-started").exists());
    process.kill().unwrap();
    process.wait().unwrap();
    until(|| root.path().join("remove-started").exists());
    let competitor = fs::File::open(root.path().join("workspace.lease")).unwrap();
    assert!(
        held(&competitor),
        "parent death released an active container's lease"
    );
    fs::remove_file(root.path().join("block-remove")).unwrap();
    until(|| root.path().join("removed").exists());
    until(|| !held(&competitor));
}

#[test]
fn cancellation_confirms_resource_removal_before_returning() {
    let root = tempfile::tempdir().unwrap();
    let resource = mock(root.path());
    let mut command = architect::process::command_with_container("python3", &resource).unwrap();
    command
        .args(["-c", "import time;time.sleep(30)"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = SupervisedChild::spawn(&mut command).unwrap();
    child.cancel().unwrap();
    assert!(root.path().join("removed").exists());
}

#[test]
fn failed_local_launch_still_removes_the_registered_resource() {
    let root = tempfile::tempdir().unwrap();
    let resource = mock(root.path());
    let mut command =
        architect::process::command_with_container(root.path().join("missing-program"), &resource)
            .unwrap();
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    assert!(SupervisedChild::spawn(&mut command).is_err());
    assert!(root.path().join("removed").exists());
}

#[test]
fn resource_cleanup_preserves_signal_termination() {
    use std::os::unix::process::ExitStatusExt;
    let root = tempfile::tempdir().unwrap();
    let resource = mock(root.path());
    let mut command = architect::process::command_with_container("python3", &resource).unwrap();
    command
        .args(["-c", "import os,signal;os.kill(os.getpid(),signal.SIGTERM)"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = SupervisedChild::spawn(&mut command).unwrap();
    let status = child.child.wait().unwrap();
    assert_eq!(status.signal(), Some(libc::SIGTERM));
    assert_eq!(status.code(), None);
    assert!(root.path().join("removed").exists());
}

#[test]
fn guardian_termination_requests_stop_and_cleanup_before_releasing_its_lease() {
    for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
        let root = tempfile::tempdir().unwrap();
        let resource = mock(root.path());
        fs::write(root.path().join("block-remove"), b"hold").unwrap();
        let lease = lock(root.path());
        let scope = LeaseScope::new(&[lease.as_raw_fd()]);
        let mut command = architect::process::command_with_container("python3", &resource).unwrap();
        command
            .args(["-c", "import time;time.sleep(30)"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = SupervisedChild::spawn(&mut command).unwrap();
        drop(scope);
        drop(lease);
        assert_eq!(
            unsafe { libc::kill(child.child.id() as libc::pid_t, signal) },
            0
        );
        until(|| root.path().join("remove-started").exists());
        let competitor = fs::File::open(root.path().join("workspace.lease")).unwrap();
        assert!(held(&competitor));
        assert!(child.child.try_wait().unwrap().is_none());
        fs::remove_file(root.path().join("block-remove")).unwrap();
        assert!(!child.child.wait().unwrap().success());
        assert!(root.path().join("removed").exists());
        assert!(!held(&competitor));
    }
}

#[test]
fn malformed_or_oversized_engine_metadata_retains_the_lease_without_logging_payloads() {
    use std::io::Read;
    for mode in ["duplicate", "oversized", "missing", "wrong-shape"] {
        let root = tempfile::tempdir().unwrap();
        let resource = mock(root.path());
        fs::write(root.path().join("bad-inspect"), mode).unwrap();
        let lease = lock(root.path());
        let scope = LeaseScope::new(&[lease.as_raw_fd()]);
        let mut command = architect::process::command_with_container("true", &resource).unwrap();
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = SupervisedChild::spawn(&mut command).unwrap();
        drop(scope);
        drop(lease);
        until(|| fs::metadata(root.path().join("inspected")).is_ok_and(|m| m.len() >= 2));
        let competitor = fs::File::open(root.path().join("workspace.lease")).unwrap();
        assert!(held(&competitor), "{mode}");
        assert!(child.child.try_wait().unwrap().is_none(), "{mode}");
        assert!(!root.path().join("remove-started").exists(), "{mode}");
        fs::remove_file(root.path().join("bad-inspect")).unwrap();
        assert!(child.child.wait().unwrap().success());
        let mut stderr = String::new();
        child
            .child
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        assert!(!stderr.contains("DO_NOT_LOG_ENGINE_SECRET"));
        assert!(root.path().join("removed").exists());
        assert!(!held(&competitor));
    }
}

struct RealContainer(ContainerResource);
impl Drop for RealContainer {
    fn drop(&mut self) {
        // Fallback for an assertion failure in this explicitly owned fixture.
        // The exact full ID was returned by its own create, never a user name,
        // prefix selector or discovery over other containers.
        let _ = Command::new(&self.0.binary)
            .args([
                "container",
                "rm",
                "--force",
                "--time",
                "0",
                &self.0.container_id,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

fn live_call(resource: &ContainerResource, args: &[&str]) -> std::process::Output {
    Command::new(&resource.binary)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn create_live(binary: &Path, image: &str, script: &str) -> RealContainer {
    let token = architect::id();
    let name = format!("antislop-check-{}", architect::id());
    let output = Command::new(binary)
        .args([
            "create",
            "--pull=never",
            "--name",
            &name,
            "--network=none",
            "--read-only",
            "--read-only-tmpfs=false",
            "--cap-drop=all",
            "--security-opt=no-new-privileges",
            "--pid=private",
            "--ipc=private",
            "--user=65534:65534",
            "--pids-limit=32",
            "--memory=256m",
            "--cpus=1",
            "--timeout=40",
            "--http-proxy=false",
            "--unsetenv-all",
            "--image-volume=ignore",
            "--entrypoint=/usr/bin/python3",
            "--label",
            &format!("{OWNERSHIP_LABEL}={token}"),
            image,
            "-c",
            script,
        ])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "owned container creation failed; no image pull or VM start is permitted"
    );
    let resource = ContainerResource {
        binary: binary.to_path_buf(),
        container_id: String::from_utf8(output.stdout).unwrap().trim().into(),
        image_id: image.into(),
        owner_token: token,
    };
    resource.validate().unwrap();
    let owned = RealContainer(resource);
    assert!(
        architect::process::container::observe_creation(
            binary.to_path_buf(),
            &name,
            image,
            &owned.0.owner_token,
        )
        .unwrap()
        .is_some()
    );
    assert!(
        architect::process::container::observe_creation(
            binary.to_path_buf(),
            &name,
            image,
            &architect::id(),
        )
        .is_err(),
        "a foreign ownership token cannot recover even an existing exact name"
    );
    owned
}

#[test]
#[ignore = "explicit owned Podman acceptance; requires ANTISLOP_TEST_PODMAN and ANTISLOP_TEST_CONTAINER_IMAGE"]
fn real_container_cleanup_covers_exit_cancel_and_parent_sigkill_with_session_escape() {
    let binary = PathBuf::from(
        std::env::var_os("ANTISLOP_TEST_PODMAN").expect("explicit engine path required"),
    );
    let image =
        std::env::var("ANTISLOP_TEST_CONTAINER_IMAGE").expect("exact local image ID required");
    let sleeper = "import subprocess,sys,time\nsubprocess.Popen([sys.executable,'-c',\"import os,time;print('CHILD_READY',os.getpid(),os.getsid(0),flush=True);time.sleep(60)\"],start_new_session=True)\nprint('PARENT_READY',flush=True)\ntime.sleep(60)\n";

    let exited = create_live(&binary, &image, "raise SystemExit(7)");
    let mut command = architect::process::command_with_container(&binary, &exited.0).unwrap();
    command
        .args(["start", "--attach", &exited.0.container_id])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = SupervisedChild::spawn(&mut command).unwrap();
    assert_eq!(child.child.wait().unwrap().code(), Some(7));
    assert_eq!(
        live_call(&exited.0, &["container", "exists", &exited.0.container_id])
            .status
            .code(),
        Some(1)
    );

    let cancelled = create_live(&binary, &image, sleeper);
    let mut command = architect::process::command_with_container(&binary, &cancelled.0).unwrap();
    command
        .args(["start", "--attach", &cancelled.0.container_id])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = SupervisedChild::spawn(&mut command).unwrap();
    until(|| {
        String::from_utf8_lossy(
            &live_call(&cancelled.0, &["logs", &cancelled.0.container_id]).stdout,
        )
        .contains("CHILD_READY")
    });
    child.cancel().unwrap();
    assert_eq!(
        live_call(
            &cancelled.0,
            &["container", "exists", &cancelled.0.container_id]
        )
        .status
        .code(),
        Some(1)
    );

    let root = tempfile::tempdir().unwrap();
    let terminated = create_live(&binary, &image, sleeper);
    let mut command = architect::process::command_with_container(&binary, &terminated.0).unwrap();
    command
        .args(["start", "--attach", &terminated.0.container_id])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = SupervisedChild::spawn(&mut command).unwrap();
    until(|| {
        String::from_utf8_lossy(
            &live_call(&terminated.0, &["logs", &terminated.0.container_id]).stdout,
        )
        .contains("CHILD_READY")
    });
    assert_eq!(
        unsafe { libc::kill(child.child.id() as libc::pid_t, libc::SIGTERM) },
        0
    );
    assert!(!child.child.wait().unwrap().success());
    assert_eq!(
        live_call(
            &terminated.0,
            &["container", "exists", &terminated.0.container_id]
        )
        .status
        .code(),
        Some(1)
    );

    let killed = create_live(&binary, &image, sleeper);
    let mut process = parent(
        root.path(),
        &ParentFixture {
            resource: killed.0.clone(),
            program: binary,
            argv: vec![
                "start".into(),
                "--attach".into(),
                killed.0.container_id.clone(),
            ],
        },
    );
    until(|| root.path().join("parent-started").exists());
    until(|| {
        String::from_utf8_lossy(&live_call(&killed.0, &["logs", &killed.0.container_id]).stdout)
            .contains("CHILD_READY")
    });
    let competitor = fs::File::open(root.path().join("workspace.lease")).unwrap();
    assert!(held(&competitor));
    process.kill().unwrap();
    process.wait().unwrap();
    until(|| {
        live_call(&killed.0, &["container", "exists", &killed.0.container_id])
            .status
            .code()
            == Some(1)
    });
    until(|| !held(&competitor));
    println!(
        "Owned Podman exit(7), cancellation, guardian SIGTERM and parent-SIGKILL cleanup passed; session-escaping children were container-bound; no user project mounted."
    );
}
