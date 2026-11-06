use serde_json::Value;

/// The configured arguments, minus the ones this runtime owns. Permission and
/// sandbox switches are not among them: the CLI's own policy applies, and the
/// dialogs it shows reach the runtime. OpenCode's agent is the runtime's own.
pub(super) fn runtime_args(args: &[String], provider: crate::config::ProviderKind) -> Vec<String> {
    if provider != crate::config::ProviderKind::Opencode {
        return args.to_vec();
    }
    let mut kept = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg.split('=').next() == Some("--agent") {
            if !arg.contains('=') {
                args.next();
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
