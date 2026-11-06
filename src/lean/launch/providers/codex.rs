use super::common::toml_string;
use super::*;
pub(super) fn codex(command: &mut CommandBuilder, spec: &Spec, gateway: &Gateway) {
    if let Some(id) = &spec.resume {
        command.args(["resume", id]);
    }
    append_codex_settings(command, gateway);
    command.env(TOKEN_ENV, gateway.token());
    // The shared Codex app server keeps its own sandbox and approval policy;
    // run the TUI standalone so this launch answers to the user's own settings.
    command.arg("--no-daemon");
    command.arg("-C");
    command.arg(&spec.cwd);
    append_codex_paths(command, spec);
    append_codex_model(command, spec);
}

fn append_codex_settings(command: &mut CommandBuilder, gateway: &Gateway) {
    let prefix = format!("mcp_servers.{SERVER}");
    for setting in [
        // A prompt to update would hold an unattended turn.
        "check_for_update_on_startup=false".to_owned(),
        format!("{prefix}.url={}", toml_string(gateway.url())),
        format!("{prefix}.bearer_token_env_var={}", toml_string(TOKEN_ENV)),
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
