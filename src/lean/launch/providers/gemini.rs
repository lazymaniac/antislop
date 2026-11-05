use super::*;

/// Gemini has no inline settings flag. Keep a private, per-session override
/// under runtime state and pass its path without changing Gemini's own files.
pub(super) fn gemini(
    command: &mut CommandBuilder,
    spec: &Spec,
    gateway: &Gateway,
) -> Result<tempfile::NamedTempFile> {
    let directory = spec.runtime.join("sessions");
    crate::lean::daemon::private_directory(&directory)?;
    let mut settings = tempfile::Builder::new()
        .prefix("gemini-")
        .suffix(".json")
        .tempfile_in(directory)?;
    let config = settings_config(gateway);
    serde_json::to_writer(&mut settings, &config)?;
    command.env("GEMINI_CLI_SYSTEM_SETTINGS_PATH", settings.path());
    command.env(TOKEN_ENV, gateway.token());
    append_gemini_options(command, spec);
    Ok(settings)
}

fn settings_config(gateway: &Gateway) -> Value {
    let config = json!({
        "mcp": {"allowed": [SERVER]},
        "mcpServers": {SERVER: {
            "httpUrl": gateway.url(),
            "headers": {"Authorization": "Bearer ${ANTISLOP_MCP_TOKEN}"},
            "trust": true,
            "includeTools": gateway.tool_names()
        }},
        "general": {"enableAutoUpdate": false}
    });

    config
}

fn append_gemini_options(command: &mut CommandBuilder, spec: &Spec) {
    command.args(["--approval-mode", "yolo", "--extensions", "none"]);
    command.args(["--allowed-mcp-server-names", SERVER]);
    if let Some(model) = &spec.model {
        command.args(["--model", model]);
    }
    for directory in &spec.add_dirs {
        command.arg("--include-directories");
        command.arg(directory);
    }
    if let Some(session) = &spec.resume {
        command.args(["--resume", session]);
    }
}
