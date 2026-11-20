//! Helpers shared by the integration tests.
#![allow(dead_code)]

use std::path::Path;
use std::sync::Once;

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
        let _ = std::fs::write(&self.0, "release");
    }
}

/// Creates a relative fixture root so daemon socket paths stay under this
/// checkout, even when its absolute worktree path is too long for Unix sockets.
pub fn tempdir() -> tempfile::TempDir {
    std::fs::create_dir_all(".antislop/tmp").unwrap();
    tempfile::tempdir_in(".antislop/tmp").unwrap()
}

/// Guard daemon tests before startup: a relative, short state path must resolve
/// beneath this checkout's private test root rather than daemon.rs's fallback.
pub fn assert_test_socket(state: &Path, process_cwd: &Path) {
    let socket = antislop::lean::daemon::socket(state);
    assert_eq!(socket, state.join("core.sock"));
    assert!(socket.is_relative(), "test socket escaped to {socket:?}");
    let workspace = std::env::current_dir().unwrap().canonicalize().unwrap();
    let process_cwd = if process_cwd.is_absolute() {
        process_cwd.to_path_buf()
    } else {
        workspace.join(process_cwd)
    };
    let private_root = workspace.join(".antislop/tmp");
    assert!(
        process_cwd.join(&socket).starts_with(&private_root),
        "test socket is outside .antislop/tmp: {process_cwd:?}/{socket:?}"
    );
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

/// Isolates user configuration and global runtime for this test process.
pub fn runtime_environment(_project: &Path) {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        let root = std::env::current_dir()
            .unwrap()
            .join(".antislop/tmp")
            .join(format!("antislop-test-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        // SAFETY: set once before this process starts its test daemons.
        unsafe {
            std::env::set_var("ANTISLOP_CONFIG_DIR", root.join("config"));
            std::env::set_var("ANTISLOP_STATE_ROOT", root.join("state"));
        }
    });
}

pub fn install_probe_cli(path: &Path, source: &str) {
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
