//! One native CLI turn. The agent works in a directory with its own tools; the
//! runtime only passes the prompt, streams normalized events and keeps the
//! CLI's session ID so the next turn resumes instead of re-sending context.

use crate::{Error, Result, config::ProviderKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    Session {
        id: String,
    },
    Text {
        delta: String,
    },
    Tool {
        name: String,
        detail: String,
    },
    /// Tokens as the CLI reported them; `None` where it did not, which is
    /// not zero.
    Usage {
        input: Option<u64>,
        output: Option<u64>,
    },
    /// The model's reasoning, as far as the CLI shows it.
    Reasoning {
        delta: String,
    },
    /// What a tool printed; clipped, the agent reads files itself.
    Output {
        name: String,
        text: String,
        exit_code: Option<i64>,
    },
    /// The full prompt at the start of a turn.
    Prompt {
        text: String,
    },
    /// The CLI runs as this process group. Kept so that a daemon started
    /// after this one died can end a turn nobody is reading any more.
    Process {
        pid: u32,
    },
    /// Controlled process status, safe to retain independently of diagnostics.
    ProcessExit {
        code: Option<i32>,
        signal: Option<i32>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub session: Option<String>,
    pub text: String,
}

/// Longest tool detail kept in an event; agents read full output themselves.
const DETAIL: usize = 240;

/// OpenCode keeps one SQLite database per user; processes starting in the
/// same instant can lose the race for its lock and exit at once.
const OPENCODE_START_GAP: Duration = Duration::from_millis(1500);
const OPENCODE_LOCK_RETRIES: u32 = 3;
const OPENCODE_LOCK_ERROR: &str = "database is locked";

/// Sleeps in short steps so cancellation stays prompt; false if cancelled.
fn pause(total: Duration, cancel: &AtomicBool) -> bool {
    let end = Instant::now() + total;
    while Instant::now() < end {
        if cancel.load(Ordering::SeqCst) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50).min(end - Instant::now()));
    }
    !cancel.load(Ordering::SeqCst)
}

/// Spaces out OpenCode starts process-wide; false if cancelled while waiting.
fn debounce_start(provider: ProviderKind, cancel: &AtomicBool) -> bool {
    static LAST_START: Mutex<Option<Instant>> = Mutex::new(None);
    if provider != ProviderKind::Opencode {
        return true;
    }
    // Holding the lock while waiting queues concurrent starts one gap apart.
    let mut last = LAST_START.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(wait) = last.and_then(|at| OPENCODE_START_GAP.checked_sub(at.elapsed()))
        && !pause(wait, cancel)
    {
        return false;
    }
    *last = Some(Instant::now());
    true
}

pub fn run(
    spec: &Spec,
    prompt: &str,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(Event),
) -> Result<Outcome> {
    let mut attempt = 0;
    loop {
        if !debounce_start(spec.provider, cancel) {
            return Err(Error::Invalid("cancelled".into()));
        }
        // A retry is only safe while the turn has shown nothing but its pid.
        let mut progressed = false;
        let result = run_once(spec, prompt, cancel, &mut |event| {
            progressed |= !matches!(event, Event::Process { .. } | Event::ProcessExit { .. });
            on_event(event);
        });
        match &result {
            Err(Error::Invalid(message))
                if spec.provider == ProviderKind::Opencode
                    && !progressed
                    && attempt < OPENCODE_LOCK_RETRIES
                    && message.contains(OPENCODE_LOCK_ERROR) =>
            {
                attempt += 1;
                if !pause(Duration::from_millis(500 * u64::from(attempt)), cancel) {
                    return Err(Error::Invalid("cancelled".into()));
                }
            }
            _ => return result,
        }
    }
}

fn run_once(
    spec: &Spec,
    prompt: &str,
    cancel: &AtomicBool,
    on_event: &mut dyn FnMut(Event),
) -> Result<Outcome> {
    let mut command = command(spec, prompt)?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let mut child = command.spawn().map_err(|error| {
        Error::Invalid(format!("cannot start {}: {error}", spec.binary.display()))
    })?;
    on_event(Event::Process { pid: child.id() });
    let stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    // Only the end of stderr matters, so only the end is kept.
    let errors = std::thread::spawn(move || {
        let mut kept: Vec<u8> = vec![];
        let mut buffer = [0; 8192];
        while let Ok(count) = stderr.read(&mut buffer) {
            if count == 0 {
                break;
            }
            kept.extend_from_slice(&buffer[..count]);
            if kept.len() > 16_384 {
                kept.drain(..kept.len() - 8192);
            }
        }
        tail(&String::from_utf8_lossy(&kept), 2000)
    });
    let (lines, received) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    let mut parser = Parser::new(spec.provider);
    let mut diagnostics = String::new();
    let mut exited: Option<std::time::Instant> = None;
    loop {
        if cancel.load(Ordering::SeqCst) {
            stop(&mut child);
            return Err(Error::Invalid("cancelled".into()));
        }
        // A descendant holding stdout open must not keep the turn alive after
        // the CLI itself has exited.
        if exited.is_none() && matches!(child.try_wait(), Ok(Some(_))) {
            exited = Some(std::time::Instant::now());
        }
        if exited.is_some_and(|at| at.elapsed() > Duration::from_secs(2)) {
            break;
        }
        match received.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => {
                let Ok(value) = serde_json::from_str::<Value>(&line) else {
                    // Launchers such as oMLX also report startup failures on
                    // stdout. Preserve a bounded tail when no result arrives.
                    diagnostics = tail(&format!("{diagnostics}\n{line}"), 2000);
                    continue;
                };
                for event in parser.feed(&value) {
                    on_event(event);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait()?;
    use std::os::unix::process::ExitStatusExt;
    on_event(Event::ProcessExit {
        code: status.code(),
        signal: status.signal(),
    });
    // Nothing the turn started outlives it.
    // SAFETY: signals the process group this runtime created for the turn.
    unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
    let stderr = errors.join().unwrap_or_default();
    let stderr = if stderr.trim().is_empty() {
        diagnostics
    } else {
        stderr
    };
    match parser.finish() {
        Finish::Done(text) if status.success() => Ok(Outcome {
            session: parser.session,
            text,
        }),
        Finish::Done(_) => Err(Error::Invalid(format!(
            "{} gave a result but exited with {status}{}",
            name(spec.provider),
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        ))),
        Finish::Failed(message) => Err(Error::Invalid(message)),
        Finish::Missing => Err(Error::Invalid(format!(
            "{} ended without a result ({status}){}",
            name(spec.provider),
            if stderr.trim().is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        ))),
    }
}

fn command(spec: &Spec, prompt: &str) -> Result<Command> {
    // A CLI's own subagents would be a team nobody verifies (design §9.5).
    const NO_CODEX_SUBAGENTS: &str = "features.multi_agent=false";
    const NO_CLAUDE_SUBAGENTS: &str = "Task,Agent";
    let mut command = Command::new(&spec.binary);
    command.args(&spec.args);
    command.current_dir(&spec.cwd);
    let read_only = spec.access == Access::ReadOnly;
    match spec.provider {
        ProviderKind::Codex => {
            command.arg("exec");
            if let Some(session) = &spec.resume {
                // `exec resume` takes no -C/-s/--add-dir; cwd is the process's.
                command.arg("resume");
                command.arg("-c").arg("approval_policy=\"never\"");
                let sandbox = if read_only {
                    "read-only"
                } else {
                    "workspace-write"
                };
                command.arg("-c").arg(format!("sandbox_mode=\"{sandbox}\""));
                command.arg("-c").arg(NO_CODEX_SUBAGENTS);
                command.arg("--json").arg("--skip-git-repo-check");
                model_args(&mut command, spec, "-m", "model_reasoning_effort");
                command.arg(session).arg(prompt);
                return Ok(command);
            }
            command
                .arg("-c")
                .arg("approval_policy=\"never\"")
                .arg("-c")
                .arg(NO_CODEX_SUBAGENTS)
                .args(["--json", "--skip-git-repo-check", "-C"])
                .arg(&spec.cwd)
                .arg("-s")
                .arg(if read_only {
                    "read-only"
                } else {
                    "workspace-write"
                });
            for dir in &spec.add_dirs {
                command.arg("--add-dir").arg(dir);
            }
            model_args(&mut command, spec, "-m", "model_reasoning_effort");
            command.arg(prompt);
        }
        ProviderKind::Agy => {
            command
                .args(["--output-format", "stream-json", "--disable-slash-commands"])
                .arg("--mode")
                .arg(if read_only { "plan" } else { "accept-edits" });
            if !read_only {
                command.arg("--sandbox");
            }
            for dir in &spec.add_dirs {
                command.arg("--add-dir").arg(dir);
            }
            if let Some(session) = &spec.resume {
                command.arg("--conversation").arg(session);
            }
            if let Some(model) = &spec.model {
                command.arg("--model").arg(model);
            }
            if let Some(effort) = &spec.effort {
                command.arg("--effort").arg(effort);
            }
            command.arg("-p").arg(prompt);
        }
        ProviderKind::Claude => {
            command
                .args(["-p", "--output-format", "stream-json", "--verbose"])
                // Before another option: the list takes every word up to one.
                .arg("--disallowedTools")
                .arg(NO_CLAUDE_SUBAGENTS)
                .arg("--permission-mode")
                .arg(if read_only { "plan" } else { "acceptEdits" });
            for dir in &spec.add_dirs {
                command.arg("--add-dir").arg(dir);
            }
            if let Some(session) = &spec.resume {
                command.arg("--resume").arg(session);
            }
            if let Some(model) = &spec.model {
                command.arg("--model").arg(model);
            }
            if let Some(effort) = &spec.effort {
                command.arg("--effort").arg(effort);
            }
            command.arg(prompt);
        }
        ProviderKind::Gemini => {
            // `plan` keeps it read-only; `auto_edit` approves file edits but
            // not shell commands. Gemini has no reasoning-effort flag.
            command
                .args(["--output-format", "stream-json", "--approval-mode"])
                .arg(if read_only { "plan" } else { "auto_edit" });
            for dir in &spec.add_dirs {
                command.arg("--include-directories").arg(dir);
            }
            if let Some(session) = &spec.resume {
                command.arg("--resume").arg(session);
            }
            if let Some(model) = &spec.model {
                command.arg("--model").arg(model);
            }
            command.arg("-p").arg(prompt);
        }
        ProviderKind::Opencode => {
            // A dedicated primary agent prevents user-defined build/plan agents
            // from overriding this turn's permissions. Keep inline provider
            // configuration (including launcher configuration) intact.
            let mut config = match std::env::var("OPENCODE_CONFIG_CONTENT") {
                Ok(text) => serde_json::from_str::<Value>(&text).map_err(|error| {
                    Error::Invalid(format!("invalid OPENCODE_CONFIG_CONTENT: {error}"))
                })?,
                Err(std::env::VarError::NotPresent) => serde_json::json!({}),
                Err(error) => return Err(Error::Invalid(error.to_string())),
            };
            require_object(&mut config, "OPENCODE_CONFIG_CONTENT")?;
            let agents = config
                .as_object_mut()
                .expect("checked object")
                .entry("agent")
                .or_insert_with(|| serde_json::json!({}));
            require_object(agents, "OpenCode agent configuration")?;
            let mut permission = serde_json::json!({
                "*": if read_only { "deny" } else { "allow" },
                "read": "allow", "glob": "allow", "grep": "allow",
                "list": "allow", "webfetch": "allow", "websearch": "allow",
                "task": "deny", "question": "deny",
                "plan_enter": "deny", "plan_exit": "deny",
                "external_directory": { "*": "deny" }
            });
            for dir in &spec.add_dirs {
                permission["external_directory"][format!("{}/*", dir.display())] =
                    Value::String("allow".into());
            }
            agents["antislop"] = serde_json::json!({
                "description": "Antislop managed agent",
                "mode": "primary",
                "permission": permission
            });
            command.env("OPENCODE_CONFIG_CONTENT", config.to_string());
            command.args(["run", "--format", "json", "--agent", "antislop"]);
            if let Some(session) = &spec.resume {
                command.arg("--session").arg(session);
            }
            if let Some(model) = &spec.model {
                command.arg("--model").arg(model);
            }
            if let Some(effort) = &spec.effort {
                command.arg("--variant").arg(effort);
            }
            command.arg("--").arg(prompt);
        }
    }
    Ok(command)
}

fn require_object(value: &mut Value, name: &str) -> Result<()> {
    if !value.is_object() {
        return Err(Error::Invalid(format!("{name} must be a JSON object")));
    }
    Ok(())
}

fn model_args(command: &mut Command, spec: &Spec, model_flag: &str, effort_key: &str) {
    if let Some(model) = &spec.model {
        command.arg(model_flag).arg(model);
    }
    if let Some(effort) = &spec.effort {
        command.arg("-c").arg(format!("{effort_key}=\"{effort}\""));
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

fn stop(child: &mut std::process::Child) {
    let group = -(child.id() as i32);
    // SAFETY: signals the process group this runtime created for the turn.
    unsafe { libc::kill(group, libc::SIGTERM) };
    for _ in 0..30 {
        if matches!(child.try_wait(), Ok(Some(_))) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // SAFETY: as above. The leader exiting does not end the group, so the
    // rest of it is killed either way.
    unsafe { libc::kill(group, libc::SIGKILL) };
    let _ = child.wait();
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

fn tail(text: &str, max: usize) -> String {
    let start = text.len().saturating_sub(max);
    let start = (start..=text.len())
        .find(|&i| text.is_char_boundary(i))
        .unwrap_or(text.len());
    text[start..].to_owned()
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

fn short(text: &str) -> String {
    let line = text.trim().replace(['\r', '\n'], " ");
    match line.char_indices().nth(DETAIL) {
        Some((end, _)) => format!("{}…", &line[..end]),
        None => line,
    }
}

enum Finish {
    Done(String),
    Failed(String),
    Missing,
}

/// Normalizes one provider's JSON lines.
struct Parser {
    provider: ProviderKind,
    session: Option<String>,
    text: String,
    /// The last fatal error the CLI reported before its result.
    error: Option<String>,
    finish: Option<Result<String>>,
    /// Name of each tool call by its id, to label the output that follows.
    tools: std::collections::HashMap<String, String>,
}

impl Parser {
    fn new(provider: ProviderKind) -> Self {
        Self {
            provider,
            session: None,
            text: String::new(),
            error: None,
            finish: None,
            tools: Default::default(),
        }
    }
    fn session(&mut self, id: Option<&str>, events: &mut Vec<Event>) {
        if let Some(id) = id.filter(|id| !id.is_empty())
            && self.session.as_deref() != Some(id)
        {
            self.session = Some(id.to_owned());
            events.push(Event::Session { id: id.to_owned() });
        }
    }
    fn feed(&mut self, line: &Value) -> Vec<Event> {
        let mut events = vec![];
        match self.provider {
            ProviderKind::Codex => self.codex(line, &mut events),
            ProviderKind::Agy => self.agy(line, &mut events),
            ProviderKind::Claude => self.claude(line, &mut events),
            ProviderKind::Gemini => self.gemini(line, &mut events),
            ProviderKind::Opencode => self.opencode(line, &mut events),
        }
        events
    }
    fn codex(&mut self, line: &Value, events: &mut Vec<Event>) {
        let item = &line["item"];
        match line["type"].as_str().unwrap_or_default() {
            "thread.started" => self.session(line["thread_id"].as_str(), events),
            "item.started" if item["type"] == "command_execution" => events.push(Event::Tool {
                name: "shell".into(),
                detail: short(item["command"].as_str().unwrap_or_default()),
            }),
            "item.completed" => match item["type"].as_str().unwrap_or_default() {
                "agent_message" => {
                    // The last agent message is the turn's answer.
                    let text = item["text"].as_str().unwrap_or_default();
                    self.text = text.to_owned();
                    events.push(Event::Text { delta: text.into() });
                }
                "reasoning" => {
                    let text = item["text"].as_str().unwrap_or_default();
                    if !text.trim().is_empty() {
                        events.push(Event::Reasoning {
                            delta: clip(text, OUTPUT),
                        });
                    }
                }
                "command_execution" => events.push(Event::Output {
                    name: "shell".into(),
                    text: clip(
                        item["aggregated_output"].as_str().unwrap_or_default(),
                        OUTPUT,
                    ),
                    exit_code: item["exit_code"].as_i64(),
                }),
                "mcp_tool_call" => events.push(Event::Tool {
                    name: format!(
                        "{}.{}",
                        item["server"].as_str().unwrap_or("mcp"),
                        item["tool"].as_str().unwrap_or("tool")
                    ),
                    detail: short(&item["arguments"].to_string()),
                }),
                "web_search" => events.push(Event::Tool {
                    name: "web_search".into(),
                    detail: short(item["query"].as_str().unwrap_or_default()),
                }),
                "file_change" => events.push(Event::Tool {
                    name: "edit".into(),
                    detail: short(
                        &item["changes"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(|change| change["path"].as_str())
                            .collect::<Vec<_>>()
                            .join(", "),
                    ),
                }),
                _ => {}
            },
            "turn.completed" => {
                events.push(Event::Usage {
                    input: line["usage"]["input_tokens"].as_u64(),
                    output: line["usage"]["output_tokens"].as_u64(),
                });
                self.finish = Some(Ok(self.text.clone()));
            }
            "turn.failed" => {
                self.finish = Some(Err(Error::Invalid(format!(
                    "codex: {}",
                    line["error"]["message"].as_str().unwrap_or("turn failed")
                ))))
            }
            "error" if self.finish.is_none() => {
                self.finish = Some(Err(Error::Invalid(format!(
                    "codex: {}",
                    line["message"].as_str().unwrap_or("error")
                ))))
            }
            _ => {}
        }
    }
    fn agy(&mut self, line: &Value, events: &mut Vec<Event>) {
        match line["event"].as_str().unwrap_or_default() {
            "init" => self.session(line["conversation_id"].as_str(), events),
            "step_update" => {
                let step = &line["step_update"];
                self.session(step["conversation_id"].as_str(), events);
                match step["step_type"].as_str().unwrap_or_default() {
                    "tool" if step["state"] == "ACTIVE" => {
                        let parameters = &step["tool_info"]["parameters"];
                        let detail = parameters
                            .as_object()
                            .and_then(|p| p.values().find_map(Value::as_str))
                            .unwrap_or_default();
                        events.push(Event::Tool {
                            name: step["tool_name"].as_str().unwrap_or("tool").into(),
                            detail: short(detail),
                        });
                    }
                    "thinking" | "reasoning" => {
                        if let Some(delta) = step["text_delta"].as_str().or(step["text"].as_str()) {
                            events.push(Event::Reasoning {
                                delta: clip(delta, OUTPUT),
                            });
                        }
                    }
                    "agent_response" => {
                        if let Some(delta) = step["text_delta"].as_str() {
                            self.text.push_str(delta);
                            events.push(Event::Text {
                                delta: delta.into(),
                            });
                        }
                    }
                    _ => {}
                }
            }
            "result" => {
                let result = &line["result"];
                self.session(result["conversation_id"].as_str(), events);
                events.push(Event::Usage {
                    input: result["usage"]["input_tokens"].as_u64(),
                    output: result["usage"]["output_tokens"].as_u64(),
                });
                self.finish = Some(if result["status"] == "SUCCESS" {
                    Ok(result["response"]
                        .as_str()
                        .map_or_else(|| self.text.clone(), str::to_owned))
                } else {
                    Err(Error::Invalid(format!(
                        "agy: {}",
                        result["error"].as_str().unwrap_or("turn failed")
                    )))
                });
            }
            _ => {}
        }
    }
    fn claude(&mut self, line: &Value, events: &mut Vec<Event>) {
        self.session(line["session_id"].as_str(), events);
        match line["type"].as_str().unwrap_or_default() {
            "assistant" => {
                for block in line["message"]["content"].as_array().into_iter().flatten() {
                    match block["type"].as_str().unwrap_or_default() {
                        "text" => {
                            let text = block["text"].as_str().unwrap_or_default();
                            events.push(Event::Text { delta: text.into() });
                        }
                        "thinking" => {
                            let text = block["thinking"].as_str().unwrap_or_default();
                            if !text.trim().is_empty() {
                                events.push(Event::Reasoning {
                                    delta: clip(text, OUTPUT),
                                });
                            }
                        }
                        "tool_use" => {
                            let detail = block["input"]
                                .as_object()
                                .and_then(|input| input.values().find_map(Value::as_str))
                                .unwrap_or_default();
                            let name = block["name"].as_str().unwrap_or("tool");
                            if let Some(id) = block["id"].as_str() {
                                self.tools.insert(id.to_owned(), name.to_owned());
                            }
                            events.push(Event::Tool {
                                name: name.into(),
                                detail: short(detail),
                            });
                        }
                        _ => {}
                    }
                }
            }
            "user" => {
                for block in line["message"]["content"].as_array().into_iter().flatten() {
                    if block["type"] != "tool_result" {
                        continue;
                    }
                    let text = match &block["content"] {
                        Value::String(text) => text.clone(),
                        Value::Array(parts) => parts
                            .iter()
                            .filter_map(|part| part["text"].as_str())
                            .collect::<Vec<_>>()
                            .join("\n"),
                        _ => String::new(),
                    };
                    events.push(Event::Output {
                        name: block["tool_use_id"]
                            .as_str()
                            .and_then(|id| self.tools.get(id))
                            .cloned()
                            .unwrap_or_else(|| "tool".into()),
                        text: clip(&text, OUTPUT),
                        exit_code: (block["is_error"] == true).then_some(1),
                    });
                }
            }
            "result" => {
                // Claude counts tokens read from or written to its prompt
                // cache apart from `input_tokens`; all of them were read.
                let usage = &line["usage"];
                let cached = ["cache_creation_input_tokens", "cache_read_input_tokens"]
                    .iter()
                    .filter_map(|key| usage[*key].as_u64())
                    .sum::<u64>();
                events.push(Event::Usage {
                    input: usage["input_tokens"].as_u64().map(|input| input + cached),
                    output: usage["output_tokens"].as_u64(),
                });
                let text = line["result"].as_str().unwrap_or_default().to_owned();
                self.finish = Some(if line["is_error"] == true {
                    Err(Error::Invalid(format!("claude: {text}")))
                } else {
                    Ok(text)
                });
            }
            _ => {}
        }
    }
    /// Gemini CLI versions spell some fields differently (`session_id` or
    /// `sessionId`, `tool_name`/`parameters` or `name`/`args`); both are read.
    fn gemini(&mut self, line: &Value, events: &mut Vec<Event>) {
        let field = |a: &str, b: &str| {
            if line[a].is_null() {
                line[b].clone()
            } else {
                line[a].clone()
            }
        };
        self.session(field("session_id", "sessionId").as_str(), events);
        match line["type"].as_str().unwrap_or_default() {
            "message" if line["role"] == "assistant" => {
                let text = line["content"].as_str().unwrap_or_default();
                // A whole message replaces what came before it; deltas add.
                if line["delta"] != true {
                    self.text.clear();
                }
                self.text.push_str(text);
                events.push(Event::Text { delta: text.into() });
            }
            "tool_use" => {
                let parameters = field("parameters", "args");
                let detail = parameters
                    .as_object()
                    .and_then(|p| p.values().find_map(Value::as_str))
                    .unwrap_or_default();
                let name = field("tool_name", "name")
                    .as_str()
                    .unwrap_or("tool")
                    .to_owned();
                if let Some(id) = field("tool_id", "toolId").as_str() {
                    self.tools.insert(id.to_owned(), name.clone());
                }
                events.push(Event::Tool {
                    name,
                    detail: short(detail),
                });
            }
            "tool_result" => {
                let output = field("output", "result");
                let text = output
                    .as_str()
                    .map_or_else(|| output.to_string(), str::to_owned);
                events.push(Event::Output {
                    name: field("tool_id", "toolId")
                        .as_str()
                        .and_then(|id| self.tools.get(id))
                        .cloned()
                        .unwrap_or_else(|| "tool".into()),
                    text: clip(&text, OUTPUT),
                    exit_code: (line["status"] == "error").then_some(1),
                });
            }
            "error" if line["severity"] == "error" => {
                self.error = line["message"].as_str().map(str::to_owned);
            }
            "result" => {
                let stats = &line["stats"];
                events.push(Event::Usage {
                    input: stats["input_tokens"].as_u64(),
                    output: stats["output_tokens"].as_u64(),
                });
                self.finish = Some(if line["status"] == "success" {
                    Ok(line["response"]
                        .as_str()
                        .map_or_else(|| self.text.clone(), str::to_owned))
                } else {
                    let error = line["error"]["message"]
                        .as_str()
                        .or(line["error"].as_str())
                        .map(str::to_owned)
                        .or_else(|| self.error.clone())
                        .unwrap_or_else(|| "turn failed".into());
                    Err(Error::Invalid(format!("gemini: {error}")))
                });
            }
            _ => {}
        }
    }
    fn opencode(&mut self, line: &Value, events: &mut Vec<Event>) {
        self.session(line["sessionID"].as_str(), events);
        let part = &line["part"];
        match line["type"].as_str().unwrap_or_default() {
            "step_start" => {
                // Each model step can contain commentary before tool calls.
                // Only the final step is the answer to the runtime protocol.
                self.text.clear();
                self.finish = None;
            }
            "text" => {
                if let Some(text) = part["text"].as_str() {
                    self.text.push_str(text);
                    events.push(Event::Text { delta: text.into() });
                }
            }
            "reasoning" => {
                if let Some(text) = part["text"].as_str().filter(|t| !t.trim().is_empty()) {
                    events.push(Event::Reasoning {
                        delta: clip(text, OUTPUT),
                    });
                }
            }
            "tool_use" => {
                let input = &part["state"]["input"];
                let detail = input["command"]
                    .as_str()
                    .or(input["filePath"].as_str())
                    .or_else(|| input.as_object()?.values().find_map(Value::as_str))
                    .unwrap_or_default();
                let name = part["tool"].as_str().unwrap_or("tool");
                events.push(Event::Tool {
                    name: name.into(),
                    detail: short(detail),
                });
                if let Some(output) = part["state"]["output"]
                    .as_str()
                    .filter(|output| !output.trim().is_empty())
                {
                    events.push(Event::Output {
                        name: name.into(),
                        text: clip(output, OUTPUT),
                        exit_code: part["state"]["metadata"]["exit"].as_i64(),
                    });
                }
            }
            "step_finish" => {
                let tokens = &part["tokens"];
                let cached = ["read", "write"]
                    .iter()
                    .filter_map(|key| tokens["cache"][*key].as_u64())
                    .sum::<u64>();
                events.push(Event::Usage {
                    input: tokens["input"].as_u64().map(|input| input + cached),
                    output: tokens["output"].as_u64(),
                });
                self.finish = match part["reason"].as_str() {
                    Some("stop") => Some(Ok(self.text.clone())),
                    Some("tool-calls") => None,
                    Some(reason) => Some(Err(Error::Invalid(format!(
                        "opencode: step ended with {reason}"
                    )))),
                    None => None,
                };
            }
            "error" => {
                let error = &line["error"];
                let message = error["data"]["message"]
                    .as_str()
                    .or(error["message"].as_str())
                    .or(error.as_str())
                    .or(error["name"].as_str())
                    .map(str::to_owned)
                    .unwrap_or_else(|| error.to_string());
                self.error = Some(format!("opencode: {message}"));
            }
            _ => {}
        }
    }
    fn finish(&mut self) -> Finish {
        if self.provider == ProviderKind::Opencode
            && let Some(error) = self.error.take()
        {
            return Finish::Failed(error);
        }
        match self.finish.take() {
            Some(Ok(text)) => Finish::Done(text),
            Some(Err(error)) => Finish::Failed(error.to_string()),
            None => Finish::Missing,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
    fn codex_writer_permissions_are_explicit_on_start_and_resume() {
        for resume in [None, Some("session".to_owned())] {
            for access in [Access::Write, Access::ReadOnly] {
                let spec = Spec {
                    provider: ProviderKind::Codex,
                    binary: "codex".into(),
                    args: vec![],
                    model: None,
                    effort: None,
                    cwd: ".".into(),
                    add_dirs: vec![],
                    access,
                    resume: resume.clone(),
                };
                let args: Vec<String> = command(&spec, "inspect")
                    .unwrap()
                    .get_args()
                    .map(|arg| arg.to_string_lossy().into_owned())
                    .collect();
                let expected = if access == Access::Write {
                    "workspace-write"
                } else {
                    "read-only"
                };
                assert!(
                    args.iter().any(
                        |arg| arg == expected || arg == &format!("sandbox_mode=\"{expected}\"")
                    ),
                    "{args:?}"
                );
                assert!(args.iter().any(|arg| arg == "approval_policy=\"never\""));
                assert!(!args.iter().any(|arg| arg.contains("danger-full-access")));
            }
        }
    }

    fn feed(provider: ProviderKind, lines: &[Value]) -> (Vec<Event>, Parser) {
        let mut parser = Parser::new(provider);
        let events = lines.iter().flat_map(|line| parser.feed(line)).collect();
        (events, parser)
    }

    fn reasoning_and_output(events: &[Event]) -> Vec<Event> {
        events
            .iter()
            .filter(|event| matches!(event, Event::Reasoning { .. } | Event::Output { .. }))
            .cloned()
            .collect()
    }

    #[test]
    fn every_cli_reports_reasoning_and_tool_output() {
        let reasoning = |text: &str| Event::Reasoning { delta: text.into() };
        let output = |name: &str, text: &str, exit_code| Event::Output {
            name: name.into(),
            text: text.into(),
            exit_code,
        };
        let (events, _) = feed(
            ProviderKind::Codex,
            &[
                json!({"type":"item.completed","item":{"type":"reasoning","text":"Plan the edit"}}),
                json!({"type":"item.completed","item":{"type":"command_execution","command":"ls","aggregated_output":"a\nb\n","exit_code":2}}),
            ],
        );
        assert_eq!(
            reasoning_and_output(&events),
            [reasoning("Plan the edit"), output("shell", "a\nb", Some(2))]
        );
        let (events, _) = feed(
            ProviderKind::Claude,
            &[
                json!({"type":"assistant","message":{"content":[
                    {"type":"thinking","thinking":"Look first"},
                    {"type":"tool_use","id":"u1","name":"Read","input":{"file_path":"a.rs"}}]}}),
                json!({"type":"user","message":{"content":[
                    {"type":"tool_result","tool_use_id":"u1","content":[{"type":"text","text":"fn a() {}"}]}]}}),
                json!({"type":"user","message":{"content":[
                    {"type":"tool_result","tool_use_id":"u2","content":"boom","is_error":true}]}}),
            ],
        );
        assert_eq!(
            reasoning_and_output(&events),
            [
                reasoning("Look first"),
                output("Read", "fn a() {}", None),
                output("tool", "boom", Some(1)),
            ]
        );
        let (events, _) = feed(
            ProviderKind::Gemini,
            &[
                json!({"type":"tool_use","tool_name":"run_shell_command","tool_id":"g1","parameters":{"command":"ls"}}),
                json!({"type":"tool_result","tool_id":"g1","status":"success","output":"done"}),
            ],
        );
        assert_eq!(
            reasoning_and_output(&events),
            [output("run_shell_command", "done", None)]
        );
        let (events, _) = feed(
            ProviderKind::Opencode,
            &[
                json!({"type":"reasoning","part":{"text":"Think"}}),
                json!({"type":"tool_use","part":{"tool":"bash","state":{"input":{"command":"ls"},"output":"x","metadata":{"exit":0}}}}),
            ],
        );
        assert_eq!(
            reasoning_and_output(&events),
            [reasoning("Think"), output("bash", "x", Some(0))]
        );
        let (events, _) = feed(
            ProviderKind::Agy,
            &[
                json!({"event":"step_update","step_update":{"step_type":"thinking","text_delta":"hm"}}),
            ],
        );
        assert_eq!(reasoning_and_output(&events), [reasoning("hm")]);
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
    fn codex_stream_yields_session_tools_and_last_message() {
        let (events, mut parser) = feed(
            ProviderKind::Codex,
            &[
                json!({"type":"thread.started","thread_id":"t1"}),
                json!({"type":"item.started","item":{"type":"command_execution","command":"cat note.txt"}}),
                json!({"type":"item.completed","item":{"type":"agent_message","text":"draft"}}),
                json!({"type":"item.completed","item":{"type":"agent_message","text":"hello"}}),
                json!({"type":"turn.completed","usage":{"input_tokens":3,"output_tokens":1}}),
            ],
        );
        assert_eq!(events[0], Event::Session { id: "t1".into() });
        assert_eq!(
            events[1],
            Event::Tool {
                name: "shell".into(),
                detail: "cat note.txt".into()
            }
        );
        assert!(matches!(parser.finish(), Finish::Done(text) if text == "hello"));
        assert_eq!(parser.session.as_deref(), Some("t1"));
    }

    #[test]
    fn agy_stream_yields_deltas_and_reports_cli_errors() {
        let (events, mut parser) = feed(
            ProviderKind::Agy,
            &[
                json!({"event":"init","conversation_id":"c1"}),
                json!({"event":"step_update","step_update":{"conversation_id":"c1","state":"ACTIVE","step_type":"tool","tool_name":"view_file","tool_info":{"parameters":{"AbsolutePath":"/p/note.txt"}}}}),
                json!({"event":"step_update","step_update":{"conversation_id":"c1","state":"ACTIVE","step_type":"agent_response","text_delta":"hel"}}),
                json!({"event":"result","result":{"conversation_id":"c1","status":"SUCCESS","response":"hello\n"}}),
            ],
        );
        assert_eq!(events.len(), 4, "{events:?}");
        assert!(matches!(parser.finish(), Finish::Done(text) if text == "hello\n"));
        let (_, mut failed) = feed(
            ProviderKind::Agy,
            &[json!({"event":"result","result":{"status":"ERROR","error":"bad model"}})],
        );
        assert!(matches!(failed.finish(), Finish::Failed(message) if message == "agy: bad model"));
    }

    #[test]
    fn claude_stream_yields_session_and_result() {
        let (events, mut parser) = feed(
            ProviderKind::Claude,
            &[
                json!({"type":"system","subtype":"init","session_id":"s1"}),
                json!({"type":"assistant","session_id":"s1","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/p/a.rs"}}]}}),
                json!({"type":"result","session_id":"s1","is_error":false,"result":"done","usage":{"input_tokens":1,"cache_creation_input_tokens":20,"cache_read_input_tokens":300,"output_tokens":1}}),
            ],
        );
        assert_eq!(events[0], Event::Session { id: "s1".into() });
        assert!(events.contains(&Event::Usage {
            input: Some(321),
            output: Some(1)
        }));
        assert!(matches!(parser.finish(), Finish::Done(text) if text == "done"));
    }

    #[test]
    fn gemini_stream_yields_session_tools_and_assistant_text() {
        let (events, mut parser) = feed(
            ProviderKind::Gemini,
            &[
                json!({"type":"init","session_id":"g1","model":"gemini-3-pro"}),
                json!({"type":"message","role":"user","content":"hi"}),
                json!({"type":"tool_use","tool_name":"read_file","tool_id":"x","parameters":{"file_path":"src/a.rs"}}),
                json!({"type":"tool_result","tool_id":"x","status":"success","output":"..."}),
                json!({"type":"message","role":"assistant","content":"Read ","delta":true}),
                json!({"type":"message","role":"assistant","content":"it.","delta":true}),
                json!({"type":"result","status":"success","stats":{"input_tokens":5,"output_tokens":2}}),
            ],
        );
        assert_eq!(events[0], Event::Session { id: "g1".into() });
        assert_eq!(
            events[1],
            Event::Tool {
                name: "read_file".into(),
                detail: "src/a.rs".into()
            }
        );
        assert!(matches!(parser.finish(), Finish::Done(text) if text == "Read it."));

        // The other spelling, and a failed turn.
        let (events, mut parser) = feed(
            ProviderKind::Gemini,
            &[
                json!({"type":"init","sessionId":"g2"}),
                json!({"type":"tool_use","name":"bash","args":{"command":"ls"}}),
                json!({"type":"error","severity":"error","message":"quota exceeded"}),
                json!({"type":"result","status":"error"}),
            ],
        );
        assert_eq!(events[0], Event::Session { id: "g2".into() });
        assert!(matches!(&events[1], Event::Tool { name, .. } if name == "bash"));
        assert!(
            matches!(parser.finish(), Finish::Failed(error) if error.contains("quota exceeded"))
        );
    }

    #[test]
    fn opencode_keeps_only_the_final_step_and_reports_usage() {
        let (events, mut parser) = feed(
            ProviderKind::Opencode,
            &[
                json!({"type":"step_start","sessionID":"ses_1"}),
                json!({"type":"text","sessionID":"ses_1","part":{"text":"Checking files."}}),
                json!({"type":"tool_use","sessionID":"ses_1","part":{"tool":"read","state":{"status":"completed","input":{"filePath":"src/lib.rs"}}}}),
                json!({"type":"step_finish","part":{"reason":"tool-calls","tokens":{"input":10,"output":2,"cache":{"read":5,"write":3}}}}),
                json!({"type":"step_start","sessionID":"ses_1"}),
                json!({"type":"text","part":{"text":"{\"approved\":"}}),
                json!({"type":"text","part":{"text":"true}"}}),
                json!({"type":"step_finish","part":{"reason":"stop","tokens":{"input":20,"output":4}}}),
            ],
        );
        assert_eq!(events[0], Event::Session { id: "ses_1".into() });
        assert_eq!(
            events[2],
            Event::Tool {
                name: "read".into(),
                detail: "src/lib.rs".into()
            }
        );
        assert_eq!(
            events[3],
            Event::Usage {
                input: Some(18),
                output: Some(2)
            }
        );
        assert_eq!(
            events.last(),
            Some(&Event::Usage {
                input: Some(20),
                output: Some(4)
            })
        );
        assert!(matches!(parser.finish(), Finish::Done(text) if text == "{\"approved\":true}"));
    }

    #[test]
    fn opencode_does_not_accept_errors_or_incomplete_steps_as_success() {
        for reason in ["tool-calls", "length"] {
            let (_, mut parser) = feed(
                ProviderKind::Opencode,
                &[
                    json!({"type":"text","part":{"text":"partial"}}),
                    json!({"type":"step_finish","part":{"reason":reason}}),
                ],
            );
            assert!(!matches!(parser.finish(), Finish::Done(_)));
        }
        for lines in [
            vec![json!({"type":"text","part":{"text":"partial"}})],
            vec![json!({"type":"step_start"})],
        ] {
            let (_, mut parser) = feed(ProviderKind::Opencode, &lines);
            assert!(matches!(parser.finish(), Finish::Missing));
        }
        let (_, mut parser) = feed(
            ProviderKind::Opencode,
            &[
                json!({"type":"step_finish","part":{"reason":"stop"}}),
                json!({"type":"error","sessionID":"ses_2","error":{"name":"APIError","data":{"message":"model not found"}}}),
                json!({"type":"step_finish","part":{"reason":"stop"}}),
            ],
        );
        assert!(
            matches!(parser.finish(), Finish::Failed(error) if error == "opencode: model not found")
        );
    }

    #[test]
    fn codex_runs_through_a_launcher_that_forwards_its_arguments() {
        for launcher in ["omlx", "ollama"] {
            let dir = tempfile::tempdir().unwrap();
            let script = dir.path().join(launcher);
            std::fs::write(
                &script,
                r##"#!/usr/bin/env python3
import json, sys
args = sys.argv[1:]
# The launcher's own words come first, then the CLI's exactly as without it.
assert args[:3] == ['launch', 'codex', '--'], args
assert args[3] == 'exec', args
assert '--json' in args and '-s' in args and 'read-only' in args, args
assert args[-1] == '--leading dash and unicode: żółw', args
print(json.dumps({'type': 'thread.started', 'thread_id': 'th1'}))
print(json.dumps({'type': 'item.completed', 'item': {'type': 'agent_message', 'text': 'via launcher'}}))
print(json.dumps({'type': 'turn.completed', 'usage': {}}))
"##,
            )
            .unwrap();
            std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
                .unwrap();
            let spec = Spec {
                provider: ProviderKind::Codex,
                binary: script,
                args: ["launch", "codex", "--"].map(str::to_owned).to_vec(),
                model: None,
                effort: None,
                cwd: dir.path().into(),
                add_dirs: vec![],
                access: Access::ReadOnly,
                resume: None,
            };
            let outcome = run(
                &spec,
                "--leading dash and unicode: żółw",
                &AtomicBool::new(false),
                &mut |_| {},
            )
            .unwrap();
            assert_eq!(outcome.text, "via launcher");
            assert_eq!(outcome.session.as_deref(), Some("th1"));
        }
    }

    #[test]
    fn opencode_launcher_runs_and_resumes_with_role_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("omlx");
        std::fs::write(
            &script,
            r##"#!/usr/bin/env python3
import json, os, sys
args = sys.argv[1:]
assert args[:6] == ['launch', 'opencode', '--model', 'local-model', '--', 'run'], args
assert args[6:10] == ['--format', 'json', '--agent', 'antislop'], args
assert args[-2:] == ['--', '--prompt with flags and unicode: żółw 👩‍💻'], args
assert args[args.index('--model', 6) + 1] == 'omlx/local-model', args
assert args[args.index('--variant') + 1] == 'high', args
permission = json.loads(os.environ['OPENCODE_CONFIG_CONTENT'])['agent']['antislop']['permission']
assert permission['task'] == 'deny'
assert permission['external_directory']['*'] == 'deny'
if '--session' in args:
    assert args[args.index('--session') + 1] == 'ses_test', args
    assert permission['*'] == 'deny', permission
    assert permission['read'] == 'allow', permission
    assert permission['external_directory']['/extra scope/*'] == 'allow', permission
else:
    assert permission['*'] == 'allow', permission
print(json.dumps({'type':'step_start', 'sessionID':'ses_test'}))
print(json.dumps({'type':'text', 'part':{'text':'done'}}))
print(json.dumps({'type':'step_finish', 'part':{'reason':'stop'}}))
"##,
        )
        .unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let mut spec = Spec {
            provider: ProviderKind::Opencode,
            binary: script,
            args: ["launch", "opencode", "--model", "local-model", "--"]
                .map(str::to_owned)
                .to_vec(),
            model: Some("omlx/local-model".into()),
            effort: Some("high".into()),
            cwd: dir.path().into(),
            add_dirs: vec![],
            access: Access::Write,
            resume: None,
        };
        let prompt = "--prompt with flags and unicode: żółw 👩‍💻";
        let outcome = run(&spec, prompt, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(outcome.text, "done");
        assert_eq!(outcome.session.as_deref(), Some("ses_test"));
        spec.resume = outcome.session;
        spec.access = Access::ReadOnly;
        spec.add_dirs = vec!["/extra scope".into()];
        assert_eq!(
            run(&spec, prompt, &AtomicBool::new(false), &mut |_| {})
                .unwrap()
                .text,
            "done"
        );
    }

    #[test]
    fn opencode_retries_a_lost_database_lock_race() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("omlx");
        let marker = dir.path().join("locked-once");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nif [ ! -e '{m}' ]; then touch '{m}'; echo 'Error: database is locked' >&2; exit 1; fi\n\
                 echo '{{\"type\":\"step_start\",\"sessionID\":\"s\"}}'\n\
                 echo '{{\"type\":\"text\",\"part\":{{\"text\":\"ok\"}}}}'\n\
                 echo '{{\"type\":\"step_finish\",\"part\":{{\"reason\":\"stop\"}}}}'\n",
                m = marker.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let spec = Spec {
            provider: ProviderKind::Opencode,
            binary: script,
            args: vec![],
            model: None,
            effort: None,
            cwd: dir.path().into(),
            add_dirs: vec![],
            access: Access::ReadOnly,
            resume: None,
        };
        let outcome = run(&spec, "hi", &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(outcome.text, "ok");
    }

    #[test]
    fn launcher_stdout_failures_are_actionable() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("omlx");
        std::fs::write(
            &script,
            "#!/bin/sh\necho 'oMLX server is not running at http://127.0.0.1:8000'\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let spec = Spec {
            provider: ProviderKind::Opencode,
            binary: script,
            args: vec![],
            model: None,
            effort: None,
            cwd: dir.path().into(),
            add_dirs: vec![],
            access: Access::ReadOnly,
            resume: None,
        };
        let error = run(&spec, "hi", &AtomicBool::new(false), &mut |_| {}).unwrap_err();
        assert!(
            error.to_string().contains("oMLX server is not running"),
            "{error}"
        );
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
    fn native_subagents_are_turned_off() {
        let args = |provider, resume: Option<&str>| -> Vec<String> {
            let spec = Spec {
                provider,
                binary: "cli".into(),
                args: vec![],
                model: None,
                effort: None,
                cwd: "/".into(),
                add_dirs: vec![],
                access: Access::Write,
                resume: resume.map(str::to_owned),
            };
            command(&spec, "prompt")
                .unwrap()
                .get_args()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect()
        };
        for codex in [
            args(ProviderKind::Codex, None),
            args(ProviderKind::Codex, Some("s")),
        ] {
            assert!(
                codex
                    .windows(2)
                    .any(|pair| pair == ["-c", "features.multi_agent=false"]),
                "{codex:?}"
            );
        }
        let claude = args(ProviderKind::Claude, None);
        let at = claude
            .iter()
            .position(|arg| arg == "--disallowedTools")
            .unwrap();
        assert_eq!(claude[at + 1], "Task,Agent");
        assert!(claude[at + 2].starts_with("--"), "{claude:?}");
        assert_eq!(claude.last().unwrap(), "prompt");
    }

    #[test]
    fn launcher_prefix_args_precede_provider_flags() {
        let spec = Spec {
            provider: ProviderKind::Codex,
            binary: "omlx".into(),
            args: vec!["launch".into(), "codex".into()],
            model: None,
            effort: Some("high".into()),
            cwd: "/".into(),
            add_dirs: vec![],
            access: Access::Write,
            resume: None,
        };
        let cmd = command(&spec, "prompt").unwrap();
        assert_eq!(cmd.get_program(), "omlx");
        let args: Vec<String> = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(&args[..3], ["launch", "codex", "exec"]);
        assert_eq!(args.last().unwrap(), "prompt");
    }

    #[test]
    fn a_missing_result_reports_exit_status_and_stderr() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("agent");
        std::fs::write(&script, "#!/bin/sh\necho 'login required' >&2\nexit 3\n").unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let spec = Spec {
            provider: ProviderKind::Codex,
            binary: script,
            args: vec![],
            model: None,
            effort: None,
            cwd: dir.path().into(),
            add_dirs: vec![],
            access: Access::ReadOnly,
            resume: None,
        };
        let error = run(&spec, "hi", &AtomicBool::new(false), &mut |_| {}).unwrap_err();
        assert!(error.to_string().contains("login required"), "{error}");
    }

    #[test]
    fn cancel_stops_the_whole_process_group() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("agent");
        std::fs::write(&script, "#!/bin/sh\nsleep 30 &\nsleep 30\n").unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let spec = Spec {
            provider: ProviderKind::Agy,
            binary: script,
            args: vec![],
            model: None,
            effort: None,
            cwd: dir.path().into(),
            add_dirs: vec![],
            access: Access::ReadOnly,
            resume: None,
        };
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            flag.store(true, Ordering::SeqCst);
        });
        let started = std::time::Instant::now();
        assert!(run(&spec, "hi", &cancel, &mut |_| {}).is_err());
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    fn script_spec(dir: &std::path::Path, body: &str) -> Spec {
        let script = dir.join("agent");
        std::fs::write(&script, format!("#!/bin/sh\n{body}")).unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        Spec {
            provider: ProviderKind::Codex,
            binary: script,
            args: vec![],
            model: None,
            effort: None,
            cwd: dir.into(),
            add_dirs: vec![],
            access: Access::ReadOnly,
            resume: None,
        }
    }

    const RESULT: &str = r#"echo '{"type":"item.completed","item":{"type":"agent_message","text":"ok"}}'
echo '{"type":"turn.completed","usage":{}}'
"#;

    #[test]
    fn a_result_from_a_crashing_cli_is_not_a_success() {
        let dir = tempfile::tempdir().unwrap();
        let spec = script_spec(dir.path(), &format!("{RESULT}echo 'panic' >&2\nexit 101\n"));
        let error = run(&spec, "hi", &AtomicBool::new(false), &mut |_| {}).unwrap_err();
        assert!(error.to_string().contains("panic"), "{error}");
    }

    #[test]
    fn descendants_holding_output_do_not_outlive_the_turn() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("alive");
        let spec = script_spec(
            dir.path(),
            &format!("(sleep 5; touch '{}') &\n{RESULT}", marker.display()),
        );
        let started = std::time::Instant::now();
        let outcome = run(&spec, "hi", &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(outcome.text, "ok");
        // Well under the descendant's sleep, with room for a loaded machine.
        assert!(started.elapsed() < Duration::from_millis(4500));
        std::thread::sleep(Duration::from_millis(5500) - started.elapsed());
        assert!(!marker.exists(), "a descendant kept running");
    }
}

#[cfg(test)]
mod clip_tests {
    use super::*;

    #[test]
    fn a_prompt_keeps_its_end() {
        let text = format!("{}END", "x".repeat(5000));
        let clipped = clip_tail(&text, OUTPUT);
        assert!(clipped.starts_with("[truncated]…"));
        assert!(clipped.ends_with("xxxEND"));
        assert_eq!(clip_tail("short", OUTPUT), "short");
    }
}
