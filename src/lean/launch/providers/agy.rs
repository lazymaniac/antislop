use super::*;

/// Agy takes MCP servers only from its own list. `antislop setup agy` puts the
/// stdio bridge there once; the gateway of each session travels in the
/// environment the bridge inherits.
pub(super) fn agy(
    command: &mut CommandBuilder,
    spec: &Spec,
    gateway: &Gateway,
    read_only: bool,
) -> Result<()> {
    if std::env::var_os("ANTISLOP_SKIP_AGY_BRIDGE_CHECK").is_none() && !agy_bridge_registered() {
        return Err(Error::Invalid(format!(
            "agy cannot reach antislop yet: run `antislop setup agy` once to register the \
             `{SERVER}` MCP bridge in Agy's MCP list"
        )));
    }
    command.env("ANTISLOP_MCP_URL", gateway.url());
    command.env("ANTISLOP_MCP_TOKEN", gateway.token());
    command.args(["--disable-slash-commands", "--mode"]);
    command.arg(if read_only { "plan" } else { "accept-edits" });
    if !read_only {
        command.arg("--sandbox");
    }
    agy_overrides(command, spec);
    Ok(())
}

fn agy_overrides(command: &mut CommandBuilder, spec: &Spec) {
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
    if let Some(id) = &spec.resume {
        command.args(["--conversation", id]);
    }
}

fn agy_bridge_registered() -> bool {
    std::env::var_os("HOME")
        .map(|home| std::path::PathBuf::from(home).join(".gemini/config/mcp_config.json"))
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .is_some_and(|config| config["mcpServers"][SERVER].is_object())
}
