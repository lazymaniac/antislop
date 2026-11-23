use super::*;
use crate::{invalid, require};
use std::path::PathBuf;

/// Deterministic launch failures use the existing non-retry startup path.
pub(crate) const CONFIGURATION_ERROR: &str = "[pi_configuration]";

/// Per-process status, separate from Pi's durable JSONL session. Never contains credentials.
pub(crate) struct PiLaunchState {
    status: PathBuf,
    expected: Value,
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
        "--model",
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
    let session = match &spec.resume {
        Some(path) => {
            let path = PathBuf::from(path);
            require(
                path.is_absolute(),
                "Pi resume requires an absolute JSONL session path",
            )?;
            // Pi owns the format and validates its contents when opening it.
            std::fs::File::open(&path)
                .map_err(|error| invalid(format!("Pi resume session: {error}")))?;
            path
        }
        None => sessions.join(format!("{launch_id}.jsonl")),
    };
    let session = session
        .to_str()
        .ok_or_else(|| invalid("Pi session path is not UTF-8"))?
        .to_owned();
    let expected = json!({
        "launch_id": launch_id,
        "alias": format!("antislop_{launch_id}"),
        "session_file": session,
        "cwd": spec.cwd.canonicalize()?.to_str().ok_or_else(|| invalid("Pi cwd is not UTF-8"))?,
        "model": spec.model,
        "effort": spec.effort,
    });
    let status = directory.join("status.json");
    command.args(&spec.args);
    command.arg("--extension");
    command.arg(adapter);
    command.arg("--session-dir");
    command.arg(sessions);
    command.args(["--session", &session]);
    if let Some(model) = &spec.model {
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
    Ok((session, PiLaunchState { status, expected }))
}

impl PiLaunchState {
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
        self.read_status()
            .map_err(|error| invalid(format!("{CONFIGURATION_ERROR} {error}")))
    }

    fn read_status(&self) -> Result<bool> {
        let bytes = match std::fs::read(&self.status) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
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
                Ok(true)
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
