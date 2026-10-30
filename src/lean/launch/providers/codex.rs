use super::common::toml_string;
use super::*;
pub(super) fn codex(command: &mut CommandBuilder, spec: &Spec, gateway: &Gateway, read_only: bool) {
    if let Some(id) = &spec.resume {
        command.args(["resume", id]);
    }
    append_codex_settings(command, spec, gateway);
    command.env(TOKEN_ENV, gateway.token());
    command.args(["-s", crate::lean::agent::codex_sandbox(read_only), "-C"]);
    command.arg(&spec.cwd);
    append_codex_paths(command, spec);
    append_codex_model(command, spec);
}

fn append_codex_settings(command: &mut CommandBuilder, spec: &Spec, gateway: &Gateway) {
    let prefix = format!("mcp_servers.{SERVER}");
    for setting in [
        // A prompt to update would hold an unattended turn.
        "check_for_update_on_startup=false".to_owned(),
        "approval_policy=\"never\"".to_owned(),
        // A CLI's own subagents would be a team nobody verifies.
        "features.multi_agent=false".to_owned(),
        format!("{prefix}.url={}", toml_string(gateway.url())),
        format!("{prefix}.bearer_token_env_var={}", toml_string(TOKEN_ENV)),
        format!("{prefix}.default_tools_approval_mode=\"approve\""),
        // The user trusted this project in antislop before any agent ran;
        // without this Codex asks again in every new checkout and the turn
        // waits on its dialog. Given per launch, never saved: an inline
        // table, as Codex splits a dotted key at the dots of the path.
        format!(
            "projects={{{}={{trust_level=\"trusted\"}}}}",
            toml_string(&spec.cwd.to_string_lossy())
        ),
    ] {
        command.args(["-c", &setting]);
    }
}

fn append_codex_paths(command: &mut CommandBuilder, spec: &Spec) {
    for dir in &spec.add_dirs {
        command.arg("--add-dir");
        command.arg(dir);
    }
}

fn append_codex_model(command: &mut CommandBuilder, spec: &Spec) {
    if let Some(model) = &spec.model {
        command.args(["-m", model]);
    }
    if let Some(effort) = &spec.effort {
        command.args([
            "-c",
            &format!("model_reasoning_effort={}", toml_string(effort)),
        ]);
    }
}
