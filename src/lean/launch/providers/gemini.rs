use super::*;

/// Gemini has no inline settings flag. Keep a private, per-session override
/// under runtime state and pass its path without changing Gemini's own files.
pub(super) fn gemini(
    command: &mut CommandBuilder,
    spec: &Spec,
    gateway: &Gateway,
    read_only: bool,
) -> Result<tempfile::NamedTempFile> {
    let directory = spec.cwd.join(".antislop").join("sessions");
    std::fs::create_dir_all(&directory)?;
    let mut settings = tempfile::Builder::new()
        .prefix("gemini-")
        .suffix(".json")
        .tempfile_in(directory)?;
    let config = settings_config(gateway, read_only);
    serde_json::to_writer(&mut settings, &config)?;
    command.env("GEMINI_CLI_SYSTEM_SETTINGS_PATH", settings.path());
    command.env(TOKEN_ENV, gateway.token());
    append_gemini_options(command, spec);
    Ok(settings)
}

fn settings_config(gateway: &Gateway, read_only: bool) -> Value {
    let mut config = json!({
        "mcp": {"allowed": [SERVER]},
        "mcpServers": {SERVER: {
            "httpUrl": gateway.url(),
            "headers": {"Authorization": "Bearer ${ANTISLOP_MCP_TOKEN}"},
            "trust": true,
            "includeTools": gateway.tool_names()
        }},
        "tools": {"exclude": ["delegate_to_agent", "ask_user"]},
        "general": {"enableAutoUpdate": false}
    });
    if read_only {
        // Do not expose a shell: even apparently read-only shell commands can
        // write. Plan mode can also block the MCP tool needed to end a turn.
        config["tools"]["core"] = json!([
            "list_directory",
            "read_file",
            "read_many_files",
            "glob",
            "search_file_content",
            "google_web_search",
            "web_fetch"
        ]);
    }
    if !read_only {
        config["tools"]["allowed"] = json!(["run_shell_command"]);
    }
    config
}

fn append_gemini_options(command: &mut CommandBuilder, spec: &Spec) {
    command.args(["--approval-mode", "auto_edit", "--extensions", "none"]);
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
