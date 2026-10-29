//! Helpers shared by the integration tests.
#![allow(dead_code)]

use antislop::lean::trust::Registry;
use std::path::Path;
use std::sync::{Mutex, Once};

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

/// Points the list of trusted directories at a private temporary place for
/// this test process (so nothing touches the user's own list) and trusts
/// `project` in it.
pub fn trust(project: &Path) {
    static ONCE: Once = Once::new();
    let dir = std::env::current_dir()
        .unwrap()
        .join(".antislop/tmp")
        .join(format!("antislop-test-config-{}", std::process::id()));
    ONCE.call_once(|| {
        std::fs::create_dir_all(&dir).unwrap();
        // SAFETY: set once, before any test of this process starts a daemon.
        unsafe { std::env::set_var("ANTISLOP_CONFIG_DIR", &dir) };
    });
    // The tests of a process run on several threads; the list is rewritten whole.
    static WRITING: Mutex<()> = Mutex::new(());
    let _writing = WRITING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Registry::at(&dir).trust(project).unwrap();
}

pub fn install_probe_cli(path: &Path, source: &str) {
    use std::os::unix::fs::PermissionsExt;
    // Reuse the canonical protocol without loading support/mod.rs twice in
    // integration crates that already declare their own support module.
    let source = source.replacen(
        "prompt = sys.argv[-1]",
        &format!(
            "prompt = sys.argv[-1]\n{}",
            include_str!("../support/ledger_cli.py")
        ),
        1,
    );
    std::fs::write(path, source).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    interactive(path);
}

include!("../support/fixture_cli.rs");
