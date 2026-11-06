use super::*;
pub(super) fn claude(
    command: &mut CommandBuilder,
    spec: &Spec,
    gateway: &Gateway,
) -> Option<String> {
    clear_parent_session(command);
    append_gateway_config(command, gateway);
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
