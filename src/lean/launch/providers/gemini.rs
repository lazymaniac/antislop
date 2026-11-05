use super::*;
use std::path::PathBuf;

pub(crate) const URL_ENV: &str = "ANTISLOP_MCP_URL";

/// Gemini only trusts MCP settings from a root-owned system location and has
/// no inline settings flag. `antislop setup gemini` installs one static file
/// there once; each launch only passes the gateway URL and token by environment.
pub(super) fn gemini(command: &mut CommandBuilder, spec: &Spec, gateway: &Gateway) -> Result<()> {
    let path = system_settings_path();
    crate::require(
        std::env::var_os("ANTISLOP_SKIP_GEMINI_SETTINGS_CHECK").is_some()
            || std::fs::read_to_string(&path).is_ok_and(|text| text == settings_text()),
        &format!(
            "gemini cannot reach antislop yet: run `sudo antislop setup gemini` once to install {}",
            path.display()
        ),
    )?;
    command.env(URL_ENV, gateway.url());
    command.env(TOKEN_ENV, gateway.token());
    append_gemini_options(command, spec);
    Ok(())
}

/// Where Gemini reads its system (administrator) settings.
pub(crate) fn system_settings_path() -> PathBuf {
    if let Some(path) = std::env::var_os("GEMINI_CLI_SYSTEM_SETTINGS_PATH") {
        return PathBuf::from(path);
    }
    if cfg!(target_os = "macos") {
        PathBuf::from("/Library/Application Support/GeminiCli/settings.json")
    } else {
        PathBuf::from("/etc/gemini-cli/settings.json")
    }
}

/// Static content: everything that varies per launch is an environment variable.
pub(crate) fn settings_text() -> String {
    let config = json!({
        "mcp": {"allowed": [SERVER]},
        "mcpServers": {SERVER: {
            "httpUrl": format!("${{{URL_ENV}}}"),
            "headers": {"Authorization": format!("Bearer ${{{TOKEN_ENV}}}")},
            "trust": true
        }},
        "general": {"enableAutoUpdate": false}
    });
    serde_json::to_string_pretty(&config).expect("static settings") + "\n"
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
