use super::*;
use crate::{invalid, require};
use std::path::PathBuf;

/// Deterministic launch failures use the existing non-retry startup path.
pub(crate) const CONFIGURATION_ERROR: &str = "[pi_configuration]";

/// Per-process status, separate from Pi's durable JSONL session. Never contains credentials.
pub(crate) struct PiLaunchState {
    status: PathBuf,
    expected: Value,
    pending: std::sync::Mutex<Option<String>>,
    submitted: std::sync::Mutex<Option<(String, u64)>>,
}

pub(crate) fn validate_args(args: &[String]) -> Result<()> {
    const RESERVED: &[&str] = &[
        "--session",
        "--session-id",
        "--session-dir",
        "--continue",
        "-c",
        "--resume",
        "-r",
        "--fork",
        "--no-session",
        "--mode",
        "--print",
        "-p",
        "--no-mcp",
        "--thinking",
        "--",
    ];
    for arg in args {
        let name = if arg.starts_with("--") {
            arg.split('=').next().unwrap_or(arg)
        } else {
            arg
        };
        require(
            !RESERVED.contains(&name),
            &format!(
                "Pi argument token {name} is reserved by antislop; remove it from command/args and use profile model/effort fields for those settings"
            ),
        )?;
    }
    Ok(())
}

pub(super) fn prepare(
    command: &mut CommandBuilder,
    spec: &Spec,
    gateway: &Gateway,
) -> Result<(String, PiLaunchState)> {
    validate_args(&spec.args)?;
    let argument_model = argument_model(&spec.args)?;
    if let (Some(argument), Some(profile)) = (argument_model, spec.model.as_deref()) {
        require(
            argument == profile,
            "Pi command model conflicts with profile model",
        )?;
    }
    let model = argument_model.or(spec.model.as_deref());
    if let Some(effort) = &spec.effort {
        require(
            ["low", "medium", "high", "xhigh"].contains(&effort.as_str()),
            "Pi effort must be low, medium, high or xhigh",
        )?;
    }
    let runtime = spec.runtime.join("pi");
    crate::lean::daemon::private_directory(&runtime)?;
    let runtime = runtime.canonicalize()?;
    let launch_id = uuid::Uuid::new_v4().simple().to_string();
    let directory = runtime.join(&launch_id);
    crate::lean::daemon::private_directory(&directory)?;
    let adapter = directory.join("antislop.ts");
    // Unique private launch directory prevents replacement of a live module.
    std::fs::write(&adapter, include_str!("pi.ts"))?;
    let sessions = runtime.join("sessions");
    crate::lean::daemon::private_directory(&sessions)?;
    let session = session_path(&sessions, &launch_id, spec.resume.as_deref())?;
    let expected = json!({
        "launch_id": launch_id,
        // Pi hashes qualified MCP names longer than 64 characters.
        // Keep room for the tool name so it matches the advertised namespace.
        "alias": format!("antislop_{}", &launch_id[..24]),
        "session_file": session,
        "cwd": spec.cwd.canonicalize()?.to_str().ok_or_else(|| invalid("Pi cwd is not UTF-8"))?,
        "model": model,
        "wrapper_model": unsloth_model(spec, model),
        "effort": spec.effort,
    });
    let status = directory.join("status.json");
    command.args(&spec.args);
    command.arg("--extension");
    command.arg(adapter);
    command.arg("--session-dir");
    command.arg(sessions);
    command.args(["--session", &session]);
    if let Some(model) = model.filter(|_| argument_model.is_none()) {
        command.args(["--model", model]);
    }
    if let Some(effort) = &spec.effort {
        command.args(["--thinking", effort]);
    }
    command.env("ANTISLOP_MCP_URL", gateway.url());
    command.env(TOKEN_ENV, gateway.token());
    // Internal v1 protocol: required JSON expectations and absolute output path;
    // absent/empty values are errors, never a request to discover another session.
    command.env("ANTISLOP_PI_EXPECTED", expected.to_string());
    command.env("ANTISLOP_PI_STATUS_PATH", &status);
    Ok((
        session,
        PiLaunchState {
            status,
            expected,
            pending: std::sync::Mutex::new(None),
            submitted: std::sync::Mutex::new(None),
        },
    ))
}

/// Select and validate the durable session independently of process arguments.
fn session_path(
    sessions: &std::path::Path,
    launch_id: &str,
    resume: Option<&str>,
) -> Result<String> {
    let session = match resume {
        Some(path) => {
            let path = PathBuf::from(path);
            require(
                path.is_absolute(),
                "Pi resume requires an absolute JSONL session path",
            )?;
            validate_resume(&path)?;
            path
        }
        None => sessions.join(format!("{launch_id}.jsonl")),
    };
    Ok(session
        .to_str()
        .ok_or_else(|| invalid("Pi session path is not UTF-8"))?
        .to_owned())
}

/// `unsloth start pi` splits a hub repo:variant before resolving the backend
/// entry and launches Pi with provider=unsloth, id=entry.id. The Pi API cannot
/// independently attest the GGUF variant; retain the requested artifact too.
fn unsloth_model(spec: &Spec, model: Option<&str>) -> Option<Value> {
    if spec.binary.file_name()? != "unsloth"
        || !spec.args.starts_with(&["start".into(), "pi".into()])
    {
        return None;
    }
    let artifact = model?;
    let (repo, variant) = artifact.rsplit_once(':')?;
    let parts: Vec<_> = repo.split('/').collect();
    if parts.len() != 2
        || parts
            .iter()
            .any(|part| part.is_empty() || part.starts_with('.'))
        || !repo.ends_with("-GGUF")
        || variant.is_empty()
        || variant.contains('/')
    {
        return None;
    }
    Some(json!({"provider":"unsloth", "id":repo, "artifact":artifact,
        "quantization_observable":false}))
}

// Resolve only the model token; wrapper positional arguments keep their meaning.
fn argument_model(args: &[String]) -> Result<Option<&str>> {
    let mut found = None;
    let mut tokens = args.iter();
    while let Some(token) = tokens.next() {
        let value = if token == "--model" {
            Some(
                tokens
                    .next()
                    .ok_or_else(|| invalid("Pi --model requires a value"))?
                    .as_str(),
            )
        } else {
            token.strip_prefix("--model=")
        };
        if let Some(value) = value {
            require(
                !value.is_empty() && !value.starts_with('-'),
                "Pi --model requires a value",
            )?;
            require(
                found.is_none(),
                "Pi command contains multiple --model declarations",
            )?;
            found = Some(value);
        }
    }
    Ok(found)
}

/// Pi tolerates broken JSONL by opening an empty session. Refuse that silent
/// context loss before launching; stream entries rather than copying history.
fn validate_resume(path: &std::path::Path) -> Result<()> {
    use std::io::BufRead;
    let file = std::fs::File::open(path)
        .map_err(|error| invalid(format!("Pi resume session: {error}")))?;
    let mut entries = 0usize;
    for line in std::io::BufReader::new(file).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let entry: Value = serde_json::from_str(&line)
            .map_err(|error| invalid(format!("Pi resume JSONL entry {}: {error}", entries + 1)))?;
        if entries == 0 {
            require(
                entry["type"] == "session"
                    && entry["version"] == 3
                    && entry["id"].as_str().is_some_and(|id| !id.is_empty()),
                "Pi resume requires a version 3 session header with id",
            )?;
        } else {
            require(
                entry["type"].as_str().is_some_and(|kind| !kind.is_empty())
                    && entry["id"].as_str().is_some_and(|id| !id.is_empty()),
                "Pi resume entry requires type and id",
            )?;
        }
        entries += 1;
    }
    require(entries > 1, "Pi resume session has no durable history")
}

impl PiLaunchState {
    /// None until Pi has observed an agent start after this prompt was submitted.
    pub(crate) fn submitted_turn_idle(&self) -> Result<Option<bool>> {
        let Some(status) = self.status()? else {
            return Ok(None);
        };
        let submitted = self.submitted.lock().map_err(invalid)?;
        let Some((_, generation)) = submitted.as_ref() else {
            return Ok(None);
        };
        if status["generation"]
            .as_u64()
            .is_some_and(|current| current > *generation)
        {
            return status["idle"]
                .as_bool()
                .map(Some)
                .ok_or_else(|| invalid("Pi status lacks an idle state"));
        }
        Ok(None)
    }

    pub(crate) fn track_submission(&self, turn: &str) -> Result<()> {
        let status = self
            .status()?
            .ok_or_else(|| invalid("Pi extension is not ready"))?;
        let generation = status["generation"]
            .as_u64()
            .ok_or_else(|| invalid("Pi status lacks an agent generation"))?;
        *self.submitted.lock().map_err(invalid)? = Some((turn.to_owned(), generation));
        Ok(())
    }

    /// Private control v1: each request carries launch identity, turn and a
    /// unique nonce. A subsequent prompt waits for that exact idle acknowledgement.
    pub(crate) fn request_idle(&self, turn: &str, abort: bool) -> Result<()> {
        let nonce = uuid::Uuid::new_v4().to_string();
        let path = self.status.with_file_name("control.json");
        let temporary = path.with_extension("tmp");
        let submitted = self.submitted.lock().map_err(invalid)?;
        let (submitted_turn, generation) = submitted
            .as_ref()
            .ok_or_else(|| invalid("Pi control has no submitted turn"))?;
        require(
            submitted_turn == turn,
            "Pi control belongs to a different submitted turn",
        )?;
        let control = json!({"version":1,"launch_id":self.expected["launch_id"], "after_generation":generation,
            "turn":turn,"nonce":nonce,"abort":abort});
        std::fs::write(&temporary, control.to_string())?;
        std::fs::rename(temporary, path)?;
        *self.pending.lock().map_err(invalid)? = Some(nonce);
        Ok(())
    }

    pub(crate) fn idle_confirmed(&self) -> Result<bool> {
        let Some(status) = self.status()? else {
            return Ok(false);
        };
        let pending = self.pending.lock().map_err(invalid)?;
        Ok(status["idle"] == true
            && pending
                .as_deref()
                .is_none_or(|nonce| status["ack"].as_str() == Some(nonce)))
    }

    pub(crate) fn alias(&self) -> &str {
        self.expected["alias"].as_str().expect("generated alias")
    }

    /// Pi's built-in MCP extension reports failed connections as operational UI
    /// lines. Only this launch's alias can fail this session.
    pub(crate) fn check_transport(&self, screen: &str) -> Result<()> {
        let prefix = format!("{}: ", self.alias());
        for line in screen.lines().map(str::trim) {
            if line
                .to_ascii_lowercase()
                .starts_with("error: no such command")
            {
                return Err(invalid(format!("{CONFIGURATION_ERROR} Pi command: {line}")));
            }
            if let Some(reason) = line.strip_prefix(&prefix)
                && (reason.starts_with("failed:") || reason.starts_with("needs sign-in"))
            {
                return Err(invalid(format!("Pi MCP transport: {reason}")));
            }
            if line.starts_with("MCP failed to load:") {
                return Err(invalid(format!("Pi MCP transport: {line}")));
            }
        }
        Ok(())
    }

    /// False means the extension has not published its first status yet.
    pub(crate) fn poll(&self) -> Result<bool> {
        self.status().map(|status| status.is_some())
    }

    fn status(&self) -> Result<Option<Value>> {
        self.read_status()
            .map_err(|error| invalid(format!("{CONFIGURATION_ERROR} {error}")))
    }

    fn read_status(&self) -> Result<Option<Value>> {
        let bytes = match std::fs::read(&self.status) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(invalid(format!("Pi extension status: {error}"))),
        };
        let status: Value = serde_json::from_slice(&bytes)
            .map_err(|error| invalid(format!("Pi extension status JSON: {error}")))?;
        require(
            status["version"] == 1,
            "Pi extension status version mismatch",
        )?;
        for key in [
            "launch_id",
            "alias",
            "session_file",
            "cwd",
            "model",
            "effort",
            "wrapper_model",
        ] {
            require(
                status.get(key) == self.expected.get(key),
                &format!("Pi extension status {key} mismatch"),
            )?;
        }
        match status["state"].as_str() {
            Some("ready") => {
                for key in ["session_file", "cwd"] {
                    require(
                        status["actual"][key] == self.expected[key],
                        &format!("Pi actual {key} mismatch"),
                    )?;
                }
                Ok(Some(status))
            }
            Some("error") => Err(invalid(format!(
                "Pi extension: {}",
                status["error"].as_str().unwrap_or("missing error reason")
            ))),
            _ => Err(invalid("Pi extension status has an invalid state")),
        }
    }
}

#[cfg(test)]
mod tests;
