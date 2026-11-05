use super::*;
pub(super) fn opencode(command: &mut CommandBuilder, spec: &Spec, gateway: &Gateway) -> Result<()> {
    let mut config = read_config()?;
    let permission = json!({"*":"allow", "external_directory":{"*":"allow"}});
    install_agent(&mut config, permission)?;
    install_gateway(&mut config, gateway)?;
    command.env("OPENCODE_CONFIG_CONTENT", config.to_string());
    // Auto mode also covers ask rules merged from on-disk user/project config;
    // explicit role denials remain enforced by the provider.
    command.args(["--auto", "--agent", SERVER]);
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

fn install_agent(config: &mut Value, permission: Value) -> Result<()> {
    let root = config
        .as_object_mut()
        .ok_or_else(|| Error::Invalid("OPENCODE_CONFIG_CONTENT must be a JSON object".into()))?;
    root.insert("permission".into(), permission.clone());
    // A dedicated primary agent keeps user-defined agents from overriding
    // this turn's permissions.
    root.entry("agent")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or_else(|| Error::Invalid("OpenCode agent configuration must be an object".into()))?
        .insert(
            SERVER.into(),
            json!({"description": "Antislop managed agent", "mode": "primary",
                   "permission": permission}),
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
