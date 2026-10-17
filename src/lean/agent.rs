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
}

#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    pub session: Option<String>,
    pub text: String,
}

/// Longest tool detail kept in an event; agents read full output themselves.
const DETAIL: usize = 240;

pub fn run(
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
    // Nothing the turn started outlives it.
    // SAFETY: signals the process group this runtime created for the turn.
    unsafe { libc::kill(-(child.id() as i32), libc::SIGKILL) };
    let stderr = errors.join().unwrap_or_default();
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
    command.current_dir(&spec.cwd);
    let read_only = spec.access == Access::ReadOnly;
    match spec.provider {
        ProviderKind::Codex => {
            command.arg("exec");
            if let Some(session) = &spec.resume {
                // `exec resume` takes no -C/-s/--add-dir; cwd is the process's.
                command.arg("resume");
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
    }
    Ok(command)
}

fn model_args(command: &mut Command, spec: &Spec, model_flag: &str, effort_key: &str) {
    if let Some(model) = &spec.model {
        command.arg(model_flag).arg(model);
    }
    if let Some(effort) = &spec.effort {
        command.arg("-c").arg(format!("{effort_key}=\"{effort}\""));
    }
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
    }
}

fn tail(text: &str, max: usize) -> String {
    let start = text.len().saturating_sub(max);
    let start = (start..=text.len())
        .find(|&i| text.is_char_boundary(i))
        .unwrap_or(text.len());
    text[start..].to_owned()
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
}

impl Parser {
    fn new(provider: ProviderKind) -> Self {
        Self {
            provider,
            session: None,
            text: String::new(),
            error: None,
            finish: None,
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
                        "tool_use" => {
                            let detail = block["input"]
                                .as_object()
                                .and_then(|input| input.values().find_map(Value::as_str))
                                .unwrap_or_default();
                            events.push(Event::Tool {
                                name: block["name"].as_str().unwrap_or("tool").into(),
                                detail: short(detail),
                            });
                        }
                        _ => {}
                    }
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
                events.push(Event::Tool {
                    name: field("tool_name", "name").as_str().unwrap_or("tool").into(),
                    detail: short(detail),
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
    fn finish(&mut self) -> Finish {
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

    fn feed(provider: ProviderKind, lines: &[Value]) -> (Vec<Event>, Parser) {
        let mut parser = Parser::new(provider);
        let events = lines.iter().flat_map(|line| parser.feed(line)).collect();
        (events, parser)
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
    fn native_subagents_are_turned_off() {
        let args = |provider, resume: Option<&str>| -> Vec<String> {
            let spec = Spec {
                provider,
                binary: "cli".into(),
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
    fn a_missing_result_reports_exit_status_and_stderr() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("agent");
        std::fs::write(&script, "#!/bin/sh\necho 'login required' >&2\nexit 3\n").unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let spec = Spec {
            provider: ProviderKind::Codex,
            binary: script,
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
