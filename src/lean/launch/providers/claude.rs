use super::*;
pub(super) fn claude(
    command: &mut CommandBuilder,
    spec: &Spec,
    gateway: &Gateway,
    read_only: bool,
) -> Option<String> {
    clear_parent_session(command);
    append_gateway_config(command, gateway);
    append_permission_policy(command, spec, read_only);
    append_provider_options(command, spec);
    Some(append_session_id(command, spec))
}

fn clear_parent_session(command: &mut CommandBuilder) {
    for variable in PARENT_SESSION_ENV {
        command.env_remove(variable);
    }
}

fn append_gateway_config(command: &mut CommandBuilder, gateway: &Gateway) {
    let config = json!({"mcpServers": {SERVER: {
        "type": "http",
        "url": gateway.url(),
        "headers": {"Authorization": format!("Bearer {}", gateway.token())}
    }}});
    command.args(["--mcp-config", &config.to_string(), "--strict-mcp-config"]);
}

fn append_permission_policy(command: &mut CommandBuilder, spec: &Spec, read_only: bool) {
    // Plan mode refuses MCP tools, so read-only is the default mode without
    // any editing tool. A CLI's own subagents would be a team nobody verifies.
    let mut allowed: Vec<String> = crate::lean::mcp::TOOLS
        .iter()
        .map(|tool| format!("mcp__{SERVER}__{tool}"))
        .collect();
    let mut denied = vec!["Task", "Agent"];
    allowed.push("Bash".into());
    if read_only {
        denied.extend(["Edit", "Write", "NotebookEdit"]);
        // The shell is a normal one, but the operating system's sandbox keeps
        // it from writing where the agent works.
        let settings = read_only_settings(spec);
        command.args(["--settings", &settings.to_string()]);
    } else {
        command.args(["--permission-mode", "acceptEdits"]);
    }
    command.args(["--allowedTools", &allowed.join(",")]);
    command.args(["--disallowedTools", &denied.join(",")]);
}

fn read_only_settings(spec: &Spec) -> Value {
    let mut protected = vec![spec.cwd.clone()];
    protected.extend(spec.add_dirs.iter().cloned());
    let protected: Vec<String> = protected
        .iter()
        .flat_map(|dir| {
            let resolved = dir.canonicalize().unwrap_or_else(|_| dir.clone());
            [dir.clone(), resolved]
        })
        .map(|dir| dir.to_string_lossy().into_owned())
        .collect();
    json!({"sandbox": {"enabled": true, "autoAllowBashIfSandboxed": true, "filesystem": {"denyWrite": protected}}})
}

fn append_provider_options(command: &mut CommandBuilder, spec: &Spec) {
    for dir in &spec.add_dirs {
        command.arg("--add-dir");
        command.arg(dir);
    }
    if let Some(model) = &spec.model {
        command.args(["--model", model]);
    }
    if let Some(effort) = &spec.effort {
        command.args(["--effort", effort]);
    }
}

fn append_session_id(command: &mut CommandBuilder, spec: &Spec) -> String {
    match &spec.resume {
        Some(id) => {
            command.args(["--resume", id]);
            id.clone()
        }
        None => {
            let id = uuid::Uuid::new_v4().to_string();
            command.args(["--session-id", &id]);
            id
        }
    }
}
