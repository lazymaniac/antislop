//! What a CLI turn is run with and what comes back. The agent works in a
//! directory with its own tools; the runtime types the prompt into the CLI's
//! terminal (`native.rs`) and receives the answer through the MCP gateway.

use crate::config::ProviderKind;
use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Orchestrator, verifiers and analysis: may read anything, must not edit.
    ReadOnly,
    /// A worker in its own worktree.
    Write,
}

#[derive(Debug, Clone)]
pub struct Spec {
    pub provider: ProviderKind,
    pub binary: PathBuf,
    pub args: Vec<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub cwd: PathBuf,
    /// Further scope the orchestrator named, e.g. a service this project calls.
    pub add_dirs: Vec<PathBuf>,
    pub access: Access,
    /// The CLI's own session/conversation ID from an earlier turn.
    pub resume: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub session: Option<String>,
    pub text: String,
}

/// Writers run unsandboxed: the sandbox denies sockets, daemons and git
/// metadata, which fails the very builds and tests they must run. Their
/// isolation is the task worktree; read-only turns keep the sandbox.
pub(crate) fn codex_sandbox(read_only: bool) -> &'static str {
    if read_only {
        "read-only"
    } else {
        "danger-full-access"
    }
}

/// Ends the process group of a turn left running by a daemon that died, if
/// `pid` still is that turn's CLI: a group leader running `provider`, started
/// when the turn did. A process that only reused the number is left alone.
/// Returns whether the group was ended.
pub fn reap(pid: u32, started_ms: i64, provider: &str) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    // SAFETY: getpgid only reads the process table.
    if pid <= 1 || unsafe { libc::getpgid(pid) } != pid {
        return false;
    }
    let Ok(output) = Command::new("ps")
        .args(["-o", "etime=", "-o", "command=", "-p", &pid.to_string()])
        .output()
    else {
        return false;
    };
    let line = String::from_utf8_lossy(&output.stdout);
    let line = line.trim();
    let (elapsed, command) = line.split_once(' ').unwrap_or((line, ""));
    let Some(elapsed) = elapsed_seconds(elapsed) else {
        return false;
    };
    let now = super::store::now_ms();
    let began = now - elapsed * 1000;
    // `ps` counts whole seconds and the CLI starts just after its turn.
    if (began - started_ms).abs() > 5_000 || !command.contains(provider) {
        return false;
    }
    // SAFETY: signals the process group verified above.
    unsafe { libc::kill(-pid, libc::SIGTERM) };
    for _ in 0..30 {
        // SAFETY: signal 0 only checks that the group still exists.
        if unsafe { libc::kill(-pid, 0) } != 0 {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // SAFETY: as above.
    unsafe { libc::kill(-pid, libc::SIGKILL) };
    true
}

/// `ps` elapsed time, `[[dd-]hh:]mm:ss`, in seconds.
fn elapsed_seconds(text: &str) -> Option<i64> {
    let (days, clock) = match text.split_once('-') {
        Some((days, clock)) => (days.parse::<i64>().ok()?, clock),
        None => (0, text),
    };
    let mut seconds = 0;
    for part in clock.split(':') {
        seconds = seconds * 60 + part.parse::<i64>().ok()?;
    }
    Some(days * 86_400 + seconds)
}

pub fn name(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::Agy => "agy",
        ProviderKind::Codex => "codex",
        ProviderKind::Claude => "claude",
        ProviderKind::Gemini => "gemini",
        ProviderKind::Opencode => "opencode",
    }
}

/// Longest tool output or reasoning kept in an event.
pub const OUTPUT: usize = 2000;

/// Redact credentials before retaining the actionable CLI failure in the UI,
/// reports and events. This function has no diagnostic-output side effects.
/// Unknown, unlabelled secrets cannot be inferred reliably from arbitrary prose.
pub(crate) fn persisted_error(text: &str) -> String {
    redact_error(
        text,
        std::env::vars_os().filter_map(|(key, value)| {
            let key = key.to_string_lossy().to_ascii_uppercase();
            (key.contains("TOKEN")
                || key.contains("SECRET")
                || key.contains("PASSWORD")
                || key.contains("API_KEY")
                || key.contains("PRIVATE_KEY"))
            .then(|| value.into_string().ok())
            .flatten()
        }),
    )
}

fn redact_error(text: &str, secrets: impl Iterator<Item = String>) -> String {
    let mut secrets: Vec<_> = secrets.filter(|secret| !secret.is_empty()).collect();
    secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    let mut text = text.to_owned();
    for secret in secrets {
        text = text.replace(&secret, "[REDACTED]");
    }
    let mut result = String::new();
    let mut credential_next = false;
    for part in text.split_inclusive(char::is_whitespace) {
        let word = part.trim_end_matches(char::is_whitespace);
        let whitespace = &part[word.len()..];
        if word.is_empty() {
            result.push_str(whitespace);
            continue;
        }
        let lower = word.to_ascii_lowercase();
        let assignment = lower.split_once('=').or_else(|| lower.split_once(':'));
        let sensitive = assignment.is_some_and(|(key, _)| {
            key.contains("token")
                || key.contains("secret")
                || key.contains("password")
                || key.contains("api_key")
                || key.contains("apikey")
                || key.contains("authorization")
        });
        let label = lower == "bearer"
            || sensitive
                && assignment.is_some_and(|(_, value)| value.trim_matches(['\"', '\'']).is_empty());
        if credential_next || word.contains("://") || sensitive && !label {
            result.push_str("[REDACTED]");
        } else {
            result.push_str(word);
        }
        credential_next = label;
        result.push_str(whitespace);
    }
    clip(&result, OUTPUT)
}

/// The start of `text`, at most `limit` characters, marked when cut.
pub fn clip(text: &str, limit: usize) -> String {
    let text = text.trim_end();
    match text.char_indices().nth(limit) {
        Some((end, _)) => format!("{}\n…[truncated]", &text[..end]),
        None => text.to_owned(),
    }
}

/// The end of `text`, at most `limit` characters, marked when cut: of a
/// prompt, the last part is what differs from turn to turn.
pub fn clip_tail(text: &str, limit: usize) -> String {
    let text = text.trim();
    let count = text.chars().count();
    if count <= limit {
        return text.to_owned();
    }
    let start = text
        .char_indices()
        .nth(count - limit)
        .map_or(0, |(start, _)| start);
    format!("[truncated]…\n{}", &text[start..])
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;
    use std::process::Command;

    #[test]
    fn writers_run_unsandboxed_and_readers_stay_read_only() {
        assert_eq!(codex_sandbox(false), "danger-full-access");
        assert_eq!(codex_sandbox(true), "read-only");
    }

    #[test]
    fn persisted_diagnostics_keep_causes_and_redact_credentials() {
        let raw = "codex exited with status 7: login required https://user:file-only-password@host/token=encoded-secret password: hidden token=encoded Bearer   abcdef";
        let stored = redact_error(raw, std::iter::empty());
        for secret in [
            "file-only-password",
            "encoded-secret",
            "hidden",
            "abcdef",
            "token=encoded",
        ] {
            assert!(!stored.contains(secret), "{stored}");
        }
        assert!(stored.contains("codex exited with status 7: login required"));
        for cause in [
            "already has an active writer",
            "thread-store conflict",
            "login required",
        ] {
            assert_eq!(persisted_error(cause), cause);
        }
        assert_eq!(
            redact_error(
                "failure: abcdefgh",
                ["abc".into(), "abcdefgh".into()].into_iter()
            ),
            "failure: [REDACTED]"
        );
    }

    #[test]
    fn long_output_is_clipped_and_marked() {
        let long = "x".repeat(OUTPUT + 500);
        let clipped = clip(&long, OUTPUT);
        assert!(clipped.ends_with("…[truncated]"));
        assert!(clipped.chars().count() < OUTPUT + 20);
        assert_eq!(clip("short\n", OUTPUT), "short");
    }

    #[test]
    fn elapsed_times_read_as_ps_writes_them() {
        assert_eq!(elapsed_seconds("00:07"), Some(7));
        assert_eq!(elapsed_seconds("03:02:01"), Some(3 * 3600 + 121));
        assert_eq!(elapsed_seconds("2-00:00:05"), Some(2 * 86_400 + 5));
        assert_eq!(elapsed_seconds("x"), None);
    }

    #[test]
    fn only_the_turns_own_process_group_is_reaped() {
        let spawn = || {
            Command::new("sleep")
                .arg("30")
                .process_group(0)
                .spawn()
                .unwrap()
        };
        let alive = |pid: u32| unsafe { libc::kill(pid as i32, 0) } == 0;
        let mut other = spawn();
        // Started an hour before the turn: a reused number, not the turn.
        assert!(!reap(
            other.id(),
            super::super::store::now_ms() - 3_600_000,
            "sleep"
        ));
        assert!(!reap(other.id(), super::super::store::now_ms(), "codex"));
        assert!(alive(other.id()));
        other.kill().unwrap();
        other.wait().unwrap();
        let mut orphan = spawn();
        assert!(reap(orphan.id(), super::super::store::now_ms(), "sleep"));
        orphan.wait().unwrap();
    }

    #[test]
    fn a_prompt_keeps_its_end() {
        let text = format!("{}END", "x".repeat(5000));
        let clipped = clip_tail(&text, OUTPUT);
        assert!(clipped.starts_with("[truncated]…"));
        assert!(clipped.ends_with("xxxEND"));
        assert_eq!(clip_tail("short", OUTPUT), "short");
    }
}
