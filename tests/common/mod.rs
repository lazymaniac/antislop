//! Helpers shared by the integration tests.
#![allow(dead_code)]

use std::path::Path;
#[path = "../support/resources.rs"]
mod resources;
#[allow(unused_imports)]
pub use resources::{ChildGuard, TestDir, tempdir};

pub fn wait_until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while !done() {
        assert!(std::time::Instant::now() < deadline, "{what}");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

pub struct ReleaseOnDrop(pub std::path::PathBuf);

impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        if let Err(error) = std::fs::write(&self.0, "release") {
            resources::cleanup_failure(format!("release barrier {}: {error}", self.0.display()));
        }
    }
}

/// Resolve relative states against the child's cwd and reject socket fallback.
pub fn assert_test_socket(state: &Path, process_cwd: &Path) {
    let resolved = if state.is_absolute() {
        state.to_path_buf()
    } else {
        process_cwd.canonicalize().unwrap().join(state)
    };
    let socket = antislop::lean::daemon::socket(state);
    assert_eq!(socket, state.join("core.sock"));
    assert!(
        resolved.as_os_str().len() + "/core.sock".len() < 100,
        "test socket too long: {resolved:?}"
    );
    let parent = resolved.parent().unwrap().canonicalize().unwrap();
    let parent = resources::external(&parent);
    let root = std::env::var_os("ANTISLOP_TEST_ROOT").expect("run ./cargo.sh test");
    let root = resources::external(Path::new(&root));
    assert!(
        parent.starts_with(root),
        "test socket must be owned by test root: {resolved:?}"
    );
}

/// Own an autostarted daemon's state for the whole consumer scope.
/// Declare before starting the frontend, after allocating the fixture directory.
pub struct DaemonStateGuard(pub std::path::PathBuf);

impl Drop for DaemonStateGuard {
    fn drop(&mut self) {
        stop_daemon(&antislop::lean::daemon::Client::at(&self.0));
    }
}

pub fn stop_daemon(client: &antislop::lean::daemon::Client) {
    use std::os::unix::fs::FileTypeExt;
    let stopped = antislop::lean::daemon::Client::stop(client.state_dir(), true);
    let released = resources::daemon_lock_released(client.state_dir());
    if let Err(error) = stopped.map(|_| ()).map_err(|e| e.to_string()).and(released) {
        let message = format!(
            "stop owned daemon {}: {error}",
            client.state_dir().display()
        );
        resources::retain_resources(client.state_dir(), &message);
        resources::cleanup_failure(message);
        return;
    }
    // State ownership includes exactly its endpoint, including the long-path fallback.
    let socket = antislop::lean::daemon::socket(client.state_dir());
    match std::fs::symlink_metadata(&socket) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            if let Err(error) = std::fs::remove_file(&socket) {
                resources::cleanup_failure(format!(
                    "remove owned socket {}: {error}",
                    socket.display()
                ));
            }
        }
        Ok(_) => resources::cleanup_failure(format!(
            "owned socket path is no longer a socket: {}",
            socket.display()
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => resources::cleanup_failure(format!(
            "inspect owned socket {}: {error}",
            socket.display()
        )),
    }
}

/// Creates the local `main` ref required before a session branch can start.
pub fn initialize_git_project(project: &Path) {
    let init = std::process::Command::new("git")
        .args(["init", "--quiet", "--initial-branch=main"])
        .current_dir(project)
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "git init failed: {}",
        String::from_utf8_lossy(&init.stderr)
    );
    let commit = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=Antislop Test",
            "-c",
            "user.email=antislop-test@example.invalid",
            "commit",
            "--quiet",
            "--allow-empty",
            "-m",
            "initialize test project",
        ])
        .current_dir(project)
        .output()
        .unwrap();
    assert!(
        commit.status.success(),
        "initial Git commit failed: {}",
        String::from_utf8_lossy(&commit.stderr)
    );
}

/// Validate pre-launch isolation; never mutate environment in a threaded test.
pub fn runtime_environment(_project: &Path) {
    let root = std::env::var_os("ANTISLOP_TEST_ROOT").expect("run ./cargo.sh test");
    let root = resources::external(Path::new(&root));
    for name in ["ANTISLOP_CONFIG_DIR", "ANTISLOP_STATE_ROOT"] {
        let path = std::env::var_os(name).unwrap_or_else(|| {
            panic!("{name} missing; run ./cargo.sh test (tests/support/README.md)")
        });
        let path = resources::external(Path::new(&path));
        assert!(
            path.starts_with(&root),
            "{name} must be owned by test root: {path:?}"
        );
    }
}

pub fn install_probe_cli(path: &Path, source: &str) {
    resources::external(path.parent().unwrap());
    use std::os::unix::fs::PermissionsExt;
    // Reuse the canonical protocol without loading support/mod.rs twice in
    // integration crates that already declare their own support module.
    let source = source.replacen(
        "prompt = sys.argv[-1]",
        &format!(
            "prompt = sys.argv[-1]\n{}",
            include_str!("../support/task_cli.py")
        ),
        1,
    );
    std::fs::write(path, source).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    interactive(path);
}

include!("../support/fixture_cli.rs");
