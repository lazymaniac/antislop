use serde_json::Value;

/// The runtime picks the sandbox for each turn from the role; a launcher's
/// own `--sandbox` would fight it.
pub(super) fn strip_sandbox(args: &[String]) -> Vec<String> {
    let mut kept = Vec::new();
    let mut skip = false;
    for arg in args {
        if std::mem::take(&mut skip) {
            continue;
        }
        if arg == "-s" || arg == "--sandbox" {
            skip = true;
        } else if !arg.starts_with("--sandbox=") {
            kept.push(arg.clone());
        }
    }
    kept
}

pub(crate) fn toml_string(text: &str) -> String {
    Value::String(text.into()).to_string()
}
