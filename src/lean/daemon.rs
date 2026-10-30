//! The daemon owns a project's `Core`, so work goes on after a frontend
//! detaches. One request per connection, one JSON line each way. Every reply
//! is small; `events` is a long poll that returns as soon as something new is
//! recorded.

use super::Core;
use crate::{Error, Result, config::Config, require};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

mod client;
mod config_write;
pub use client::Client;
mod dispatch;
mod lifecycle;
mod protocol;
#[cfg(test)]
use config_write::atomic_config_write;
#[cfg(test)]
use config_write::{PreparedConfigWrite, cleanup_config_temporaries, config_temporary_prefix};

/// Creates `path` if needed and insists it is a real directory private to
/// the current user.
pub fn private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    if !path.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)?;
    }
    let metadata = std::fs::symlink_metadata(path)?;
    // SAFETY: geteuid has no arguments or failure modes.
    let uid = unsafe { libc::geteuid() };
    require(
        metadata.is_dir()
            && !metadata.file_type().is_symlink()
            && metadata.uid() == uid
            && metadata.permissions().mode() & 0o077 == 0,
        &format!(
            "{} must be a directory owned by the current user with mode 0700",
            path.display()
        ),
    )
}

/// Identifies a build of the executable: path, size and modification time.
pub fn build_identity(executable: &Path) -> Result<String> {
    let path = executable.canonicalize()?;
    let metadata = std::fs::metadata(&path)?;
    let modified = metadata
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    Ok(crate::hash(format!(
        "{}\0{}\0{modified}",
        path.display(),
        metadata.len()
    )))
}

const PROTOCOL_VERSION: u32 = 3;

pub(crate) use protocol::RequestEnvelope;

// Protocol ownership handoff: RequestEnvelope, Request, WireRequest, their conversions, and
// serde wire-representation implementations belong to runtime-protocol.
// Daemon retains lifecycle and dispatch; keep protocol behavior compatible.
pub use protocol::{
    ControlRequest, ConversationRequest, HistoryRequest, LedgerRequest, Request, SessionRequest,
    TerminalRequest,
};

#[derive(Debug, Serialize, Deserialize)]
struct Response {
    data: Option<Value>,
    error: Option<String>,
}

/// Longest a single `events` call waits for something new.
const LONGEST_WAIT: Duration = Duration::from_secs(20);
/// Events per reply, and their combined size, so replies stay small.
const EVENTS_PER_REPLY: usize = 500;
const REPLY_BYTES: usize = 1024 * 1024;

/// How many of the (non-empty, ordered) events fit one reply: payload
/// bytes add up until `reply_bytes` is crossed, but the first event is
/// always taken whole — even one oversized historical event must not
/// stall the cursor, so a reply's size has no strict ceiling deducible
/// from `reply_bytes` alone; the cursor, not the size, bounds a reply.
/// Everything left over waits for the next poll, which the returned
/// `next_after`/`has_more` invite.
pub fn reply_event_count(events: &[super::store::Event], reply_bytes: usize) -> usize {
    let mut bytes = 0;
    let mut taken = 0;
    for event in events {
        bytes += event.payload.to_string().len();
        if taken > 0 && bytes > reply_bytes {
            break;
        }
        taken += 1;
    }
    taken
}

/// `<state>/core.sock`, or, when that path is too long for a Unix socket, a
/// short one named after the state directory in a private directory in /tmp.
pub fn socket(state: &Path) -> PathBuf {
    let socket = state.join("core.sock");
    if socket.as_os_str().len() < 100 {
        return socket;
    }
    // SAFETY: geteuid has no arguments or failure modes.
    let user = unsafe { libc::geteuid() };
    let hash = crate::hash(state.to_string_lossy().as_bytes());
    PathBuf::from(format!("/tmp/antislop-{user}")).join(format!("{}.sock", &hash[..16]))
}

pub fn serve(project: &Path, config_path: &Path, state: &Path) -> Result<()> {
    lifecycle::serve(project, config_path, state)
}

fn handle(core: &Arc<Core>, project: &Path, config: &Path, request: Request) -> Result<Value> {
    dispatch::handle(core, project, config, request)
}

fn modified(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |time| time.as_nanos() as u64)
}

fn build() -> &'static str {
    static BUILD: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    BUILD.get_or_init(|| {
        std::env::current_exe()
            .and_then(|exe| build_identity(&exe).map_err(std::io::Error::other))
            .unwrap_or_default()
    })
}

#[cfg(test)]
mod live_config_tests {
    use super::*;

    #[test]
    fn abrupt_exit_before_and_after_config_publish_keeps_a_complete_file() {
        const CHILD: &str = "ANTISLOP_TEST_CONFIG_EXIT_PHASE";
        if let Some(phase) = std::env::var_os(CHILD) {
            let path = PathBuf::from(std::env::var_os("ANTISLOP_TEST_CONFIG_EXIT_PATH").unwrap());
            let text = std::fs::read_to_string(&path)
                .unwrap()
                .replace("max_parallel = 4", "max_parallel = 1");
            let prepared = PreparedConfigWrite::prepare(&path, &text).unwrap();
            if phase == "published" {
                prepared.publish().unwrap();
            }
            // Exit without dropping PreparedConfigWrite, as after a process
            // interruption between preparation, publication and directory sync.
            std::process::exit(0);
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = include_str!("../../examples/antislop.toml");
        for phase in ["prepared", "published"] {
            std::fs::write(&path, original).unwrap();
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "lean::daemon::live_config_tests::abrupt_exit_before_and_after_config_publish_keeps_a_complete_file"])
                .env(CHILD, phase)
                .env("ANTISLOP_TEST_CONFIG_EXIT_PATH", &path)
                .stdout(std::process::Stdio::null())
                .status().unwrap();
            assert!(status.success());
            let expected = if phase == "prepared" {
                original.to_owned()
            } else {
                original.replace("max_parallel = 4", "max_parallel = 1")
            };
            assert_eq!(std::fs::read_to_string(&path).unwrap(), expected);
            Config::load(&path).unwrap();
            atomic_config_write(&path, original).unwrap();
            assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        }
    }

    #[test]
    fn configuration_requests_validate_before_writing_and_record_the_update() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let original = include_str!("../../examples/antislop.toml");
        std::fs::write(&path, original).unwrap();
        let core = Core::open(
            &dir.path().join(".antislop"),
            Config::from_toml(original).unwrap(),
        )
        .unwrap();
        let old = core.config_snapshot();
        let call = |request| handle(&core, dir.path(), &path, request);
        assert!(
            call(Request::Control(
                crate::lean::daemon::ControlRequest::SetConfig {
                    toml: "bad = [".into(),
                    expected_revision: crate::hash(original),
                }
            ))
            .is_err()
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert_eq!(*core.config_snapshot(), *old);
        let updated = original.replace("max_parallel = 4", "max_parallel = 1");
        call(Request::Control(
            crate::lean::daemon::ControlRequest::SetConfig {
                toml: updated.clone(),
                expected_revision: crate::hash(original),
            },
        ))
        .unwrap();
        assert_eq!(core.config_snapshot().policy.max_parallel, 1);
        assert_eq!(old.policy.max_parallel, 4);
        let stale = call(Request::Control(
            crate::lean::daemon::ControlRequest::SetConfig {
                toml: original.into(),
                expected_revision: crate::hash(original),
            },
        ))
        .unwrap_err();
        assert!(stale.to_string().contains("configuration changed"));
        assert_eq!(core.config_snapshot().policy.max_parallel, 1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), updated);
        assert_eq!(
            call(Request::Control(
                crate::lean::daemon::ControlRequest::GetConfig
            ))
            .unwrap()["toml"],
            updated
        );
        assert_eq!(
            call(Request::Control(
                crate::lean::daemon::ControlRequest::Identity
            ))
            .unwrap()["config_modified"],
            modified(&path)
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), updated);
        let conversation = core.start(dir.path()).unwrap();
        assert!(
            core.store
                .events_since(&conversation.id, 0, 100)
                .unwrap()
                .iter()
                .any(|event| event.kind == "config_updated")
        );
        let db = rusqlite::Connection::open(core.state.join("core.db")).unwrap();
        db.execute_batch("CREATE TRIGGER reject_config_event BEFORE INSERT ON events WHEN NEW.kind='config_updated' BEGIN SELECT RAISE(FAIL,'injected log failure'); END;").unwrap();
        let failure = call(Request::Control(
            crate::lean::daemon::ControlRequest::SetConfig {
                toml: original.into(),
                expected_revision: crate::hash(&updated),
            },
        ))
        .unwrap_err();
        assert!(failure.to_string().contains("injected log failure"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), updated);
        assert_eq!(core.config_snapshot().policy.max_parallel, 1);
        db.execute_batch("DROP TRIGGER reject_config_event;
            CREATE TABLE deferred_failure(id TEXT REFERENCES conversations(id) DEFERRABLE INITIALLY DEFERRED);
            CREATE TRIGGER fail_config_commit AFTER INSERT ON events WHEN NEW.kind='config_updated'
            BEGIN INSERT INTO deferred_failure VALUES('missing'); END;").unwrap();
        let applied = call(Request::Control(
            crate::lean::daemon::ControlRequest::SetConfig {
                toml: original.into(),
                expected_revision: crate::hash(&updated),
            },
        ))
        .unwrap();
        assert_eq!(applied["status"], "applied_with_warnings");
        assert_eq!(applied["warnings"][0]["stage"], "event_commit");
        assert!(
            applied["warnings"][0]["message"]
                .as_str()
                .unwrap()
                .contains("FOREIGN KEY")
        );
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        assert_eq!(core.config_snapshot().policy.max_parallel, 4);
    }

    #[test]
    fn orphan_cleanup_preserves_active_writes_other_targets_and_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");
        let other = dir.path().join("other.toml");
        std::fs::write(&target, "original").unwrap();
        std::fs::write(&other, "other").unwrap();
        let mut active = PreparedConfigWrite::prepare(&target, "active").unwrap();
        let other_active = PreparedConfigWrite::prepare(&other, "other active").unwrap();
        let prefix = config_temporary_prefix(&target.canonicalize().unwrap());
        let abandoned = dir.path().join(format!("{prefix}abandoned.tmp"));
        let symlink = dir.path().join(format!("{prefix}symlink.tmp"));
        std::fs::write(&abandoned, "abandoned").unwrap();
        std::os::unix::fs::symlink(&target, &symlink).unwrap();
        assert_eq!(cleanup_config_temporaries(&target).unwrap(), 1);
        assert!(!abandoned.exists());
        assert!(active.temporary.exists());
        assert!(other_active.temporary.exists());
        assert!(
            std::fs::symlink_metadata(&symlink)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "original");
        // Simulate a crashed writer: its descriptor closes without Drop cleanup.
        let temporary = active.temporary.clone();
        let lease = std::mem::replace(&mut active.lease, std::fs::File::open(&target).unwrap());
        drop(lease);
        assert_eq!(cleanup_config_temporaries(&target).unwrap(), 1);
        assert!(!temporary.exists());
    }

    #[test]
    fn preparing_config_does_not_publish_and_drop_removes_the_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("config.toml");
        std::fs::write(&target, "old").unwrap();
        let prepared = PreparedConfigWrite::prepare(&target, "new").unwrap();
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "old");
        assert_eq!(std::fs::read_to_string(&prepared.temporary).unwrap(), "new");
        let temporary = prepared.temporary.clone();
        drop(prepared);
        assert!(!temporary.exists());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "old");
    }

    #[test]
    fn atomic_write_preserves_symlink_and_rejects_missing_target() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.toml");
        let link = dir.path().join("link.toml");
        std::fs::write(&target, "old").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        atomic_config_write(&link, "new").unwrap();
        assert!(
            std::fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "new");
        std::fs::remove_file(&target).unwrap();
        assert!(atomic_config_write(&link, "again").is_err());
        assert!(!target.exists());
    }

    #[test]
    fn failed_atomic_rename_preserves_destination_and_cleans_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("config.toml");
        std::fs::create_dir(&destination).unwrap();
        std::fs::write(destination.join("original"), "unchanged").unwrap();
        assert!(atomic_config_write(&destination, "replacement").is_err());
        assert_eq!(
            std::fs::read_to_string(destination.join("original")).unwrap(),
            "unchanged"
        );
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
