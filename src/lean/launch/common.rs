use serde_json::Value;

/// Runtime policy always wins over launcher approval and sandbox switches.
/// Keep unrelated configuration overrides intact, including their ordering.
pub(super) fn runtime_args(args: &[String], provider: crate::config::ProviderKind) -> Vec<String> {
    use crate::config::ProviderKind::*;
    let (valued, switches): (&[&str], &[&str]) = match provider {
        Claude => (
            &["--permission-mode", "--permission-prompts"],
            &[
                "--dangerously-skip-permissions",
                "--allow-dangerously-skip-permissions",
            ],
        ),
        Codex => (
            &["-s", "--sandbox", "-a", "--ask-for-approval"],
            &[
                "--full-auto",
                "--enable-auto-review",
                "--auto-review",
                "--dangerously-bypass-approvals-and-sandbox",
                "--yolo",
            ],
        ),
        Agy => (&["--mode"], &["--dangerously-skip-permissions"]),
        Gemini => (&["--approval-mode"], &["--yolo", "-y"]),
        Opencode => (&["--agent"], &["--auto"]),
    };
    let mut kept = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let flag = arg.split('=').next().unwrap_or(arg);
        if valued.contains(&flag) {
            if !arg.contains('=') {
                args.next();
            }
        } else if switches.contains(&flag) {
            continue;
        } else if provider == Codex
            && (arg == "-c"
                || arg == "--config"
                || arg.starts_with("--config=")
                || arg.starts_with("-c="))
        {
            let value = if let Some(value) = arg
                .strip_prefix("--config=")
                .or_else(|| arg.strip_prefix("-c="))
            {
                Some(value)
            } else {
                args.next().map(String::as_str)
            };
            if let Some(value) = value {
                let key = value.split('=').next().unwrap_or(value).trim();
                if !matches!(
                    key,
                    "approval_policy"
                        | "sandbox_mode"
                        | "sandbox_workspace_write"
                        | "sandbox_permissions"
                        | "approvals_reviewer"
                ) && !key.starts_with("sandbox_workspace_write.")
                {
                    kept.extend(["-c".into(), value.into()]);
                }
            }
        } else {
            kept.push(arg.clone());
        }
    }
    kept
}

pub(crate) fn toml_string(text: &str) -> String {
    Value::String(text.into()).to_string()
}
