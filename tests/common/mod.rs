//! Helpers shared by the integration tests.
#![allow(dead_code)]

use architect::lean::trust::Registry;
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
