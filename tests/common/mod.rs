//! Helpers shared by the integration tests.
#![allow(dead_code)]

use antislop::lean::trust::Registry;
use std::path::Path;
use std::sync::{Mutex, Once};

/// Points the list of trusted directories at a private temporary place for
/// this test process (so nothing touches the user's own list) and trusts
/// `project` in it.
pub fn trust(project: &Path) {
    static ONCE: Once = Once::new();
    let dir = std::env::temp_dir().join(format!("antislop-test-config-{}", std::process::id()));
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
}

/// Makes the headless-style test script at `path` answer as an interactive
/// agent CLI: the daemon starts it in a terminal, types the prompt into it and
/// hears the answer through its MCP gateway. The script keeps computing the
/// answer; it moves to `<path>.legacy`.
pub fn interactive(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut legacy = path.as_os_str().to_owned();
    legacy.push(".legacy");
    std::fs::rename(path, &legacy).unwrap();
    std::fs::write(path, include_str!("../support/pty_shim.py")).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}
