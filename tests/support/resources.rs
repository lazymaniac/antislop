//! Shared fixture allocation. See tests/support/README.md for the launch contract.
use std::path::{Path, PathBuf};
use std::process::Child;

include!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/support/external.rs"
));

pub struct TestDir {
    directory: Option<tempfile::TempDir>,
    diagnostic: PathBuf,
}

pub fn cleanup_failure(message: String) {
    if std::thread::panicking() {
        eprintln!("{message}");
    } else {
        panic!("{message}");
    }
}

/// Confirm release even when the daemon endpoint is absent.
pub fn daemon_lock_released(state: &Path) -> Result<(), String> {
    use std::os::fd::AsRawFd;
    let lock = match std::fs::OpenOptions::new()
        .write(true)
        .open(state.join("core.lock"))
    {
        Ok(lock) => lock,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("inspect daemon lock: {error}")),
    };
    // SAFETY: nonblocking flock on our owned descriptor, released on return.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(format!(
            "daemon lock not released: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// Mark every owning fixture and the launcher root before reporting cleanup failure.
pub fn retain_resources(state: &Path, message: &str) {
    let root = external(&PathBuf::from(
        std::env::var_os("ANTISLOP_TEST_ROOT").unwrap(),
    ));
    assert!(
        state.starts_with(&root),
        "daemon state outside owned test root"
    );
    for path in state.ancestors().take_while(|path| path.starts_with(&root)) {
        if let Err(error) = std::fs::write(path.join("retain-resources.log"), message) {
            eprintln!("cannot mark retained resources {}: {error}", path.display());
        }
    }
}

impl TestDir {
    pub fn path(&self) -> &Path {
        self.directory.as_ref().unwrap().path()
    }
}

pub fn tempdir() -> TestDir {
    let root = std::env::var_os("ANTISLOP_TEST_ROOT")
        .map(PathBuf::from)
        .expect("ANTISLOP_TEST_ROOT missing; run ./cargo.sh test");
    let root = external(&root);
    assert!(
        root.as_os_str().len() < 50,
        "test root too long for sockets: {root:?}"
    );
    let diagnostic = std::env::var_os("ANTISLOP_TEST_DIAGNOSTICS")
        .map(PathBuf::from)
        .expect("ANTISLOP_TEST_DIAGNOSTICS missing; run ./cargo.sh test");
    let diagnostic = external(&diagnostic);
    TestDir {
        directory: Some(
            tempfile::Builder::new()
                .prefix("as-")
                .tempdir_in(root)
                .unwrap(),
        ),
        diagnostic,
    }
}

/// Copy only named diagnostic files, never fixture repositories or databases.
pub fn diagnostics(source: &Path, target: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(source)? {
        let entry = entry?;
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() && entry.file_name() != ".git" {
            diagnostics(&path, &target.join(entry.file_name()))?;
        } else if kind.is_file()
            && (path.extension().is_some_and(|e| {
                matches!(
                    e.to_str(),
                    Some(
                        "log"
                            | "stderr"
                            | "stdout"
                            | "prompts"
                            | "rejections"
                            | "reminders"
                            | "keys"
                    )
                )
            }) || matches!(entry.file_name().to_str(), Some("stderr" | "stdout")))
        {
            std::fs::create_dir_all(target)?;
            std::fs::copy(&path, target.join(entry.file_name()))?;
        }
    }
    Ok(())
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let dir = self.directory.take().unwrap();
        let target = self.diagnostic.join(dir.path().file_name().unwrap());
        if let Err(error) = diagnostics(dir.path(), &target) {
            cleanup_failure(format!(
                "fixture diagnostic copy failed: {error}; retaining {}",
                dir.keep().display()
            ));
            return;
        }
        if dir.path().join("retain-resources.log").exists() {
            eprintln!(
                "unconfirmed resource release; retaining {}",
                dir.keep().display()
            );
            return;
        }
        let path = dir.path().to_path_buf();
        if let Err(error) = dir.close() {
            cleanup_failure(format!(
                "fixture cleanup failed at {}: {error}",
                path.display()
            ));
        }
    }
}

/// Own the child immediately after spawn, before any readiness assertion.
pub struct ChildGuard(Option<Child>);
impl ChildGuard {
    pub fn new(child: Child) -> Self {
        Self(Some(child))
    }
    pub fn wait_with_output(mut self) -> std::io::Result<std::process::Output> {
        self.0.take().unwrap().wait_with_output()
    }
}
impl std::ops::Deref for ChildGuard {
    type Target = Child;
    fn deref(&self) -> &Child {
        self.0.as_ref().unwrap()
    }
}
impl std::ops::DerefMut for ChildGuard {
    fn deref_mut(&mut self) -> &mut Child {
        self.0.as_mut().unwrap()
    }
}
impl ChildGuard {
    pub fn stop(&mut self) {
        let Some(child) = self.0.as_mut() else {
            return;
        };
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => {
                if let Err(error) = child.kill() {
                    cleanup_failure(format!("child termination failed: {error}"));
                }
            }
            Err(error) => cleanup_failure(format!("child status failed: {error}")),
        }
        if let Err(error) = child.wait() {
            cleanup_failure(format!("child reap failed: {error}"));
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.stop();
    }
}
