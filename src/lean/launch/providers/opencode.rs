use super::*;
pub(super) fn opencode(command: &mut CommandBuilder, spec: &Spec, gateway: &Gateway) -> Result<()> {
    let mut config = read_config()?;
    install_agent(&mut config)?;
    install_gateway(&mut config, gateway)?;
    command.env("OPENCODE_CONFIG_CONTENT", config.to_string());
    command.args(["--agent", SERVER]);
    append_resume_model(command, spec);
    Ok(())
}

fn read_config() -> Result<Value> {
    match std::env::var("OPENCODE_CONFIG_CONTENT") {
        Ok(text) => serde_json::from_str::<Value>(&text)
            .map_err(|error| Error::Invalid(format!("invalid OPENCODE_CONFIG_CONTENT: {error}"))),
        Err(std::env::VarError::NotPresent) => Ok(json!({})),
        Err(error) => Err(Error::Invalid(error.to_string())),
    }
}

fn install_agent(config: &mut Value) -> Result<()> {
    let root = config
        .as_object_mut()
        .ok_or_else(|| Error::Invalid("OPENCODE_CONFIG_CONTENT must be a JSON object".into()))?;
    // A dedicated primary agent keeps user-defined agents from replacing the
    // one this runtime talks to. It sets no permissions: the user's own
    // OpenCode policy decides, and its dialogs reach the runtime.
    root.entry("agent")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| Error::Invalid("OpenCode agent configuration must be an object".into()))?
        .insert(
            SERVER.into(),
            json!({"description": "Antislop managed agent", "mode": "primary"}),
        );
    Ok(())
}

fn install_gateway(config: &mut Value, gateway: &Gateway) -> Result<()> {
    let root = config
        .as_object_mut()
        .ok_or_else(|| Error::Invalid("OPENCODE_CONFIG_CONTENT must be a JSON object".into()))?;
    root.entry("mcp")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| Error::Invalid("OpenCode mcp configuration must be an object".into()))?
        .insert(
            SERVER.into(),
            json!({"type": "remote", "url": gateway.url(),
                   "headers": {"Authorization": format!("Bearer {}", gateway.token())}}),
        );
    Ok(())
}

fn append_resume_model(command: &mut CommandBuilder, spec: &Spec) {
    if let Some(id) = &spec.resume {
        command.args(["--session", id]);
    }
    if let Some(model) = &spec.model {
        command.args(["--model", model]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gateway_overlay_preserves_user_policy_and_other_configuration() {
        let gateway = Gateway::start().unwrap();
        let mut config = json!({
            "permission": {"bash": "ask"},
            "model": "user/model",
            "agent": {"personal": {"mode": "primary", "permission": {"edit": "deny"}}},
            "mcp": {"personal": {"type": "remote", "url": "https://example.invalid/mcp"}}
        });
        let original = config.clone();
        install_agent(&mut config).unwrap();
        install_gateway(&mut config, &gateway).unwrap();
        assert_eq!(config["permission"], original["permission"]);
        assert_eq!(config["model"], original["model"]);
        assert_eq!(config["agent"]["personal"], original["agent"]["personal"]);
        assert_eq!(config["mcp"]["personal"], original["mcp"]["personal"]);
        assert_eq!(config["mcp"][SERVER]["url"], gateway.url());
        assert_eq!(
            config["mcp"][SERVER]["headers"]["Authorization"],
            format!("Bearer {}", gateway.token())
        );
        assert!(config["agent"][SERVER]["permission"].is_null());
    }
}
