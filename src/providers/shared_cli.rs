//! Bounded, supervised print-mode transport shared by CLI adapters.
use super::*;
use crate::{process::SupervisedChild, require};
use std::{
    io::{BufReader, Read, Write},
    process::{Command, ExitStatus, Stdio},
    sync::{atomic::Ordering, mpsc},
    time::Duration,
};

/// Only fixed diagnostic categories leave the draining thread. Raw stderr is
/// scanned in bounded transient buffers and never returned, logged or persisted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StderrCategory {
    InputSize,
    Context,
    OutputSize,
    Schema,
    Authentication,
    RateLimit,
    Network,
}
impl StderrCategory {
    pub(crate) fn classification(self) -> (FailureKind, &'static str) {
        match self {
            Self::InputSize => (FailureKind::Protocol, "input_too_large"),
            Self::Context => (FailureKind::Protocol, "context_length_exceeded"),
            Self::OutputSize => (FailureKind::Protocol, "output_token_limit_exceeded"),
            Self::Schema => (FailureKind::Protocol, "invalid_json_schema"),
            Self::Authentication => (FailureKind::Authentication, "authentication_error"),
            Self::RateLimit => (FailureKind::RateLimit, "rate_limit_or_quota"),
            Self::Network => (FailureKind::Network, "network_transport_error"),
        }
    }
}

pub(crate) fn classify_stderr(mut stderr: impl Read) -> Option<StderrCategory> {
    const MARKERS: &[(&[u8], StderrCategory)] = &[
        (b"input_too_large", StderrCategory::InputSize),
        // Antigravity drops an oversized sole user step before inference.
        (
            b"all steps in trajectory are cleared",
            StderrCategory::InputSize,
        ),
        (
            b"input exceeds the maximum length",
            StderrCategory::InputSize,
        ),
        (b"context_length_exceeded", StderrCategory::Context),
        (b"context_window_exceeded", StderrCategory::Context),
        (b"contextwindowexceeded", StderrCategory::Context),
        (b"context window exceeded", StderrCategory::Context),
        (
            b"your input exceeds the context window",
            StderrCategory::Context,
        ),
        (b"exceeds the context window", StderrCategory::Context),
        (b"exceeded the context window", StderrCategory::Context),
        (b"maximum context length", StderrCategory::Context),
        (b"input is too long", StderrCategory::Context),
        (b"output_token_limit_exceeded", StderrCategory::OutputSize),
        (
            b"exceeded the output token limit",
            StderrCategory::OutputSize,
        ),
        (b"invalid_json_schema", StderrCategory::Schema),
        (b"invalid_response_format", StderrCategory::Schema),
        (
            b"invalid schema for response_format",
            StderrCategory::Schema,
        ),
        (b"invalid_api_key", StderrCategory::Authentication),
        (b"authentication_error", StderrCategory::Authentication),
        (b"unauthorized: 401", StderrCategory::Authentication),
        (b"401 unauthorized", StderrCategory::Authentication),
        (
            b"incorrect api key provided",
            StderrCategory::Authentication,
        ),
        (b"invalid api key", StderrCategory::Authentication),
        (b"authentication failed", StderrCategory::Authentication),
        (b"authentication required", StderrCategory::Authentication),
        (b"not logged in", StderrCategory::Authentication),
        (b"access token expired", StderrCategory::Authentication),
        (b"rate_limit_exceeded", StderrCategory::RateLimit),
        (b"insufficient_quota", StderrCategory::RateLimit),
        (b"quota_exceeded", StderrCategory::RateLimit),
        (b"rate limit reached", StderrCategory::RateLimit),
        (b"rate limit exceeded", StderrCategory::RateLimit),
        (b"usage limit", StderrCategory::RateLimit),
        (b"connection reset by peer", StderrCategory::Network),
        (b"error sending request for url", StderrCategory::Network),
        (b"network_error", StderrCategory::Network),
        (b"connection timed out", StderrCategory::Network),
        (b"connection refused", StderrCategory::Network),
        (b"dns error", StderrCategory::Network),
        (
            b"failed to lookup address information",
            StderrCategory::Network,
        ),
        (
            b"stream disconnected before completion",
            StderrCategory::Network,
        ),
        (b"request timed out", StderrCategory::Network),
        (b"tls handshake", StderrCategory::Network),
    ];
    const OVERLAP: usize = 64;
    let mut buffer = [0u8; 4096 + OVERLAP];
    let mut retained = 0;
    let mut category = None;
    loop {
        let count = match stderr.read(&mut buffer[retained..]) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        };
        let end = retained + count;
        if category.is_none() {
            buffer[..end].make_ascii_lowercase();
            category = MARKERS.iter().find_map(|(marker, category)| {
                buffer[..end]
                    .windows(marker.len())
                    .any(|bytes| bytes == *marker)
                    .then_some(*category)
            });
        }
        retained = end.min(OVERLAP);
        buffer.copy_within(end - retained..end, 0);
    }
    buffer.fill(0);
    category
}

/// Call only for a native error/terminal-failure payload. Never scan artifact
/// text: an otherwise successful answer may legitimately discuss these errors.
/// Only fixed code/type/message fields are inspected; no raw string is returned.
pub(crate) fn diagnostic_category(value: &Value) -> Option<StderrCategory> {
    fn inspect(value: &Value, depth: u8) -> Option<StderrCategory> {
        if let Some(text) = value.as_str() {
            return classify_stderr(text.as_bytes());
        }
        for field in ["code", "type", "message"] {
            if let Some(text) = value.get(field).and_then(Value::as_str)
                && let Some(category) = classify_stderr(text.as_bytes())
            {
                return Some(category);
            }
        }
        if depth < 2 {
            value
                .get("error")
                .and_then(|error| inspect(error, depth + 1))
        } else {
            None
        }
    }
    inspect(value, 0)
}

/// Prompts above this size cannot fit a ~1M-token context window; refuse before
/// launch so the failure is diagnosed instead of an opaque CLI exit.
pub(crate) const MAX_PROMPT_CHARACTERS: usize = 3_000_000;
pub(crate) fn require_prompt_size(provider: &str, prompt: &str) -> Result<()> {
    let characters = prompt.chars().count();
    if characters > MAX_PROMPT_CHARACTERS {
        return Err(ProviderFailure {
            provider: provider.into(),
            kind: FailureKind::Protocol,
            message: format!(
                "input_too_large; input_characters={characters}; max_characters={MAX_PROMPT_CHARACTERS}"
            ),
            session_id: None,
            stopped: true,
            partial_effects: false,
        }
        .into());
    }
    Ok(())
}
pub(crate) fn exit_failure(
    provider: &str,
    status: ExitStatus,
    category: Option<StderrCategory>,
    native_failure: Option<FailureKind>,
    session_id: Option<String>,
    partial_effects: bool,
) -> crate::Error {
    use std::os::unix::process::ExitStatusExt;
    let (diagnosed_kind, diagnosis) = category
        .map(StderrCategory::classification)
        .unwrap_or((FailureKind::Process, "unknown"));
    let exit = match (status.code(), status.signal()) {
        (Some(code), _) => format!("exit_code={code}"),
        (_, Some(signal)) => format!("signal={signal}"),
        _ => "exit_status=unknown".into(),
    };
    ProviderFailure {
        provider: provider.into(),
        kind: native_failure
            .filter(|kind| *kind != FailureKind::Process)
            .unwrap_or(diagnosed_kind),
        message: format!("CLI exited without a successful completed turn; {exit}; diagnosis={diagnosis}; raw diagnostics not retained"),
        session_id,
        stopped: true,
        partial_effects,
    }
    .into()
}

pub enum StreamEvent {
    Session(String),
    Text {
        content: String,
        delta: bool,
    },
    Result {
        output: Option<String>,
        usage: UsageReport,
    },
    Failed(FailureKind),
    ForbiddenTool,
    Unknown,
}

/// Never inherit runtime-loader, startup-prompt or experimental overrides.
/// Authentication remains owned by the CLI; these are credential inputs only.
pub fn command(provider: &str, binary: &str) -> Result<Command> {
    let mut command = crate::process::command(binary)?;
    command.env_clear();
    let common = [
        "PATH", "HOME", "LANG", "TMPDIR", "TMP", "TEMP", "USER", "LOGNAME", "TERM",
    ];
    let credentials: &[&str] = match provider {
        "agy" => &["GEMINI_API_KEY"],
        "codex" => &[
            "OPENAI_API_KEY",
            "CODEX_API_KEY",
            "OPENAI_ORG_ID",
            "OPENAI_PROJECT_ID",
        ],
        "claude" => &[
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_AUTH_TOKEN",
            "CLAUDE_CODE_OAUTH_TOKEN",
        ],
        "gemini" => &[
            "GEMINI_API_KEY",
            "GOOGLE_API_KEY",
            "GOOGLE_GENAI_USE_GCA",
            "GOOGLE_GENAI_USE_VERTEXAI",
            "GOOGLE_CLOUD_PROJECT",
            "GOOGLE_CLOUD_PROJECT_ID",
            "GOOGLE_CLOUD_LOCATION",
        ],
        _ => {
            return Err(failure(
                provider,
                FailureKind::Unsupported,
                "unknown provider environment policy",
            ));
        }
    };
    for (name, value) in std::env::vars_os() {
        if let Some(name_str) = name.to_str()
            && (common.contains(&name_str)
                || credentials.contains(&name_str)
                || name_str.starts_with("LC_"))
        {
            command.env(name, value);
        }
    }
    if std::env::var_os("PATH").is_none() {
        command.env("PATH", "/usr/bin:/bin");
    }
    Ok(command)
}

pub fn exact_session(provider: &str, resume: Option<&str>) -> Result<String> {
    if let Some(session) = resume {
        if uuid::Uuid::parse_str(session).is_err() {
            return Err(failure(
                provider,
                FailureKind::InvalidSession,
                "resume requires an exact session UUID; aliases and indices are forbidden",
            ));
        }
        Ok(session.into())
    } else {
        Ok(crate::id())
    }
}

pub fn error_kind(value: &Value) -> FailureKind {
    let code = value
        .get("code")
        .or_else(|| value.get("status"))
        .and_then(Value::as_i64);
    let kind = value
        .get("type")
        .or_else(|| value.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    if matches!(code, Some(401 | 403 | 41)) || kind.contains("auth") {
        FailureKind::Authentication
    } else if code == Some(429) || kind.contains("rate") || kind.contains("quota") {
        FailureKind::RateLimit
    } else if matches!(code, Some(500..=599))
        || kind.contains("network")
        || kind.contains("connection")
    {
        FailureKind::Network
    } else if kind.contains("session") {
        FailureKind::InvalidSession
    } else {
        diagnostic_category(value)
            .map(|category| category.classification().0)
            .unwrap_or(FailureKind::Process)
    }
}

/// Capture only bounded public probe output; stderr is never retained.
pub fn capture(command: &mut Command, cancelled: Option<&AtomicBool>) -> Result<String> {
    capture_probe(command, cancelled, false)
}

/// Go-based CLIs may write public `--help` text to stderr. This path appends
/// only that argument; model-turn stderr always uses the private classifier.
pub fn capture_help(command: &mut Command, cancelled: Option<&AtomicBool>) -> Result<String> {
    command.arg("--help");
    capture_probe(command, cancelled, true)
}

fn capture_probe(
    command: &mut Command,
    cancelled: Option<&AtomicBool>,
    help: bool,
) -> Result<String> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(if help { Stdio::piped() } else { Stdio::null() });
    let mut child = SupervisedChild::spawn(command)?;
    let mut pipes: Vec<Box<dyn Read + Send>> =
        vec![Box::new(child.child.stdout.take().expect("probe stdout"))];
    if help {
        pipes.push(Box::new(
            child.child.stderr.take().expect("probe help stderr"),
        ));
    }
    let count = pipes.len();
    let (sender, receiver) = mpsc::sync_channel(count);
    let readers: Vec<_> = pipes
        .into_iter()
        .enumerate()
        .map(|(index, pipe)| {
            let sender = sender.clone();
            std::thread::spawn(move || {
                let mut bytes = Vec::new();
                let result = pipe
                    .take((crate::transport::MAX_FRAME + 1) as u64)
                    .read_to_end(&mut bytes)
                    .map(|_| bytes);
                let _ = sender.send((index, result));
            })
        })
        .collect();
    drop(sender);
    let mut output = std::collections::BTreeMap::new();
    let mut total = 0usize;
    let mut status = None;
    while output.len() < count || status.is_none() {
        if cancelled.is_some_and(|flag| flag.load(Ordering::SeqCst)) {
            child.cancel()?;
            return Err(failure(
                "cli",
                FailureKind::Cancelled,
                "probe stopped by cancellation",
            ));
        }
        status = status.or(child.child.try_wait()?);
        if output.len() < count {
            match receiver.recv_timeout(Duration::from_millis(25)) {
                Ok((index, result)) => {
                    let bytes = result?;
                    total += bytes.len();
                    require(
                        total <= crate::transport::MAX_FRAME,
                        "CLI probe exceeds transport limit",
                    )?;
                    output.insert(index, bytes);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => (),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(failure("cli", FailureKind::Protocol, "probe reader failed"));
                }
            }
        } else if status.is_none() {
            std::thread::sleep(Duration::from_millis(25));
        }
    }
    for reader in readers {
        let _ = reader.join();
    }
    require(status.expect("probe exit").success(), "CLI probe failed")?;
    String::from_utf8(output.into_values().flatten().collect())
        .map_err(|_| failure("cli", FailureKind::Protocol, "probe output is not UTF-8"))
}

pub fn inspect(
    provider: &str,
    binary: &str,
    version: &str,
    flags: &[&str],
    cancelled: Option<&AtomicBool>,
) -> Result<String> {
    let output = capture(command(provider, binary)?.arg("--version"), cancelled)?;
    if !output.trim().starts_with(version) {
        return Err(failure(
            provider,
            FailureKind::Unsupported,
            "CLI version is outside the validated contract",
        ));
    }
    let help = capture(command(provider, binary)?.arg("--help"), cancelled)?;
    if !flags.iter().all(|flag| help.contains(flag)) {
        return Err(failure(
            provider,
            FailureKind::Unsupported,
            "CLI does not advertise the required restriction/transport flags",
        ));
    }
    Ok(output.trim().into())
}

pub fn run(
    provider: &str,
    command: &mut Command,
    request: &TurnRequest<'_>,
    session_id: &str,
    cancelled: Arc<AtomicBool>,
    events: &mut dyn FnMut(&ProviderEvent) -> Result<()>,
    normalize: fn(&Value) -> Result<StreamEvent>,
) -> Result<TurnReceipt> {
    require(
        !request.turn_id.trim().is_empty(),
        "missing durable turn ID",
    )?;
    if cancelled.load(Ordering::SeqCst) {
        return Err(failure(
            provider,
            FailureKind::Cancelled,
            "turn cancelled before start",
        ));
    }
    // Callers persist these callbacks before permitting the following effect.
    events(&ProviderEvent::DeliveryPending {
        turn_id: request.turn_id.into(),
        session_id: session_id.into(),
    })?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = SupervisedChild::spawn(command)?;
    let stdout = child.child.stdout.take().expect("provider stdout");
    let stderr = child.child.stderr.take().expect("provider stderr");
    let diagnostics = std::thread::spawn(move || classify_stderr(stderr));
    let (sender, receiver) = mpsc::sync_channel(16);
    let reader = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let frame = crate::transport::read::<Value>(&mut reader);
            let terminal = matches!(frame, Ok(None) | Err(_));
            if sender.send(frame).is_err() || terminal {
                break;
            }
        }
    });
    let mut stdin = child.child.stdin.take().expect("provider stdin");
    let prompt = request.prompt.to_owned();
    let (sent_sender, sent_receiver) = mpsc::sync_channel(1);
    let writer = std::thread::spawn(move || {
        let result = stdin
            .write_all(prompt.as_bytes())
            .and_then(|_| stdin.flush());
        drop(stdin);
        let _ = sent_sender.send(result);
    });
    let mut delivered = false;
    let mut write_failed = false;
    let mut observed = false;
    let mut completed = false;
    let mut eof = false;
    let mut status = None;
    let mut output = String::new();
    let mut usage = UsageReport::Unavailable;
    let mut failed = None;
    let mut native_diagnosis = None;
    while !eof || status.is_none() {
        if cancelled.load(Ordering::SeqCst) {
            let callback = events(&ProviderEvent::CancellationRequested {
                turn_id: request.turn_id.into(),
            });
            child.cancel()?;
            callback?;
            events(&ProviderEvent::Stopped {
                turn_id: request.turn_id.into(),
            })?;
            return Err(ProviderFailure {
                provider: provider.into(),
                kind: FailureKind::Cancelled,
                message: "process group stopped; prior effects require reconciliation".into(),
                session_id: Some(session_id.into()),
                stopped: true,
                partial_effects: delivered,
            }
            .into());
        }
        if !delivered
            && !write_failed
            && let Ok(result) = sent_receiver.try_recv()
        {
            if result.is_ok() {
                delivered = true;
                events(&ProviderEvent::DeliverySent {
                    turn_id: request.turn_id.into(),
                })?;
            } else {
                write_failed = true;
            }
        }
        status = status.or(child.child.try_wait()?);
        if eof {
            std::thread::sleep(Duration::from_millis(25));
            continue;
        }
        let frame = match receiver.recv_timeout(Duration::from_millis(25)) {
            Ok(Ok(Some(value))) => value,
            Ok(Ok(None)) => {
                eof = true;
                continue;
            }
            Ok(Err(_)) => {
                return Err(failure(
                    provider,
                    FailureKind::Protocol,
                    "malformed, duplicate-key or oversized CLI frame",
                ));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(failure(
                    provider,
                    FailureKind::Protocol,
                    "CLI event stream disconnected",
                ));
            }
        };
        if let Some(id) = frame.get("session_id").and_then(Value::as_str)
            && id != session_id
        {
            return Err(failure(
                provider,
                FailureKind::InvalidSession,
                "event refers to a different session than requested",
            ));
        }
        match normalize(&frame)? {
            StreamEvent::Session(id) => {
                if observed || completed {
                    return Err(failure(
                        provider,
                        FailureKind::Protocol,
                        "duplicate or post-terminal session initialization",
                    ));
                }
                if id != session_id {
                    return Err(failure(
                        provider,
                        FailureKind::InvalidSession,
                        "CLI returned a different session than the durable requested ID",
                    ));
                }
                if !observed {
                    events(&ProviderEvent::SessionObserved { session_id: id })?;
                    observed = true;
                }
            }
            StreamEvent::Text { content, delta } => {
                if completed || !observed {
                    return Err(failure(
                        provider,
                        FailureKind::Protocol,
                        "text before session or after terminal result",
                    ));
                }
                if delta {
                    output.push_str(&content);
                } else {
                    output = content;
                }
                require(
                    output.len() <= crate::transport::MAX_FRAME,
                    "provider artifact exceeds transport limit",
                )?;
            }
            StreamEvent::Result {
                output: result,
                usage: reported,
            } => {
                if completed || !observed {
                    return Err(failure(
                        provider,
                        FailureKind::Protocol,
                        "terminal result before session or duplicated",
                    ));
                }
                completed = true;
                if let Some(value) = result {
                    output = value;
                }
                usage = reported;
            }
            StreamEvent::Failed(kind) => {
                failed = Some(kind);
                native_diagnosis = diagnostic_category(&frame);
            }
            StreamEvent::ForbiddenTool => {
                return Err(failure(
                    provider,
                    FailureKind::Policy,
                    "tool/subagent event violates the tool-free profile",
                ));
            }
            StreamEvent::Unknown => (),
        }
    }
    let _ = writer.join();
    if !delivered && !write_failed {
        if matches!(sent_receiver.recv(), Ok(Ok(()))) {
            delivered = true;
            events(&ProviderEvent::DeliverySent {
                turn_id: request.turn_id.into(),
            })?;
        } else {
            write_failed = true;
        }
    }
    let _ = reader.join();
    let diagnosis = native_diagnosis.or(diagnostics.join().ok().flatten());
    let status = status.expect("provider exit");
    if failed.is_some() || !status.success() || write_failed {
        let native_failure = if status.code() == Some(41) {
            Some(FailureKind::Authentication)
        } else {
            failed
        };
        return Err(exit_failure(
            provider,
            status,
            diagnosis,
            native_failure,
            Some(session_id.into()),
            delivered || write_failed,
        ));
    }
    if !completed || !observed {
        return Err(exit_failure(
            provider,
            status,
            diagnosis,
            diagnosis.is_none().then_some(FailureKind::Protocol),
            Some(session_id.into()),
            delivered,
        ));
    }
    let _: Value = crate::transport::decode(output.as_bytes()).map_err(|_| {
        failure(
            provider,
            FailureKind::Protocol,
            "CLI artifact is not strict JSON",
        )
    })?;
    events(&ProviderEvent::Completed {
        turn_id: request.turn_id.into(),
    })?;
    Ok(TurnReceipt {
        turn_id: request.turn_id.into(),
        provider: provider.into(),
        session_id: session_id.into(),
        delivered,
        completed: true,
        stopped: true,
        output,
        usage,
    })
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn stderr_classifier_matches_codes_split_across_arbitrary_reads() {
        struct ByteReader(std::io::Cursor<Vec<u8>>);
        impl Read for ByteReader {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                self.0.read(&mut buffer[..1])
            }
        }
        for (message, expected) in [
            (
                "private-secret context_length_exceeded more-private-secret",
                StderrCategory::Context,
            ),
            (
                "private-secret Your input exceeds the context window more-private-secret",
                StderrCategory::Context,
            ),
            (
                "private-secret input_too_large more-private-secret",
                StderrCategory::InputSize,
            ),
            (
                "private-secret Input exceeds the maximum length more-private-secret",
                StderrCategory::InputSize,
            ),
            (
                "private-secret Your previous response was cut off because it exceeded the output token limit",
                StderrCategory::OutputSize,
            ),
            (
                "generating and executing: serializing trajectory: no renderable messages: all steps in trajectory are cleared",
                StderrCategory::InputSize,
            ),
        ] {
            assert_eq!(
                classify_stderr(ByteReader(std::io::Cursor::new(
                    message.as_bytes().to_vec()
                ))),
                Some(expected)
            );
        }
    }

    #[test]
    fn stderr_classifier_drains_large_stream_even_after_classification() {
        let mut input = b"invalid_json_schema".to_vec();
        input.resize(2_000_000, b'x');
        let mut reader = std::io::Cursor::new(input);
        assert_eq!(classify_stderr(&mut reader), Some(StderrCategory::Schema));
        assert_eq!(reader.position(), 2_000_000);
    }

    #[test]
    fn oversized_prompts_are_refused_before_launch() {
        assert!(require_prompt_size("agy", &"x".repeat(MAX_PROMPT_CHARACTERS)).is_ok());
        let error = require_prompt_size("agy", &"x".repeat(MAX_PROMPT_CHARACTERS + 1))
            .unwrap_err()
            .to_string();
        assert!(error.contains("input_too_large"), "{error}");
    }
}
