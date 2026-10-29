//! Safe, actionable diagnostics for TOML deserialization failures.

use crate::Error;

pub(super) fn toml_error(source: &str, error: toml::de::Error) -> Error {
    let location = toml_location(source, &error);
    // Never include TOML's source excerpt or invalid configuration values.
    let (reason, fix) = toml_diagnostic(error.message());
    Error::Invalid(format!(
        "invalid TOML configuration{location}: {reason}; to fix: {fix}"
    ))
}

fn toml_location(source: &str, error: &toml::de::Error) -> String {
    error
        .span()
        .map(|span| {
            let mut offset = span.start.min(source.len());
            while !source.is_char_boundary(offset) {
                offset -= 1;
            }
            let prefix = &source[..offset];
            let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
            let column = prefix.rsplit('\n').next().unwrap_or("").chars().count() + 1;
            format!(" at line {line}, column {column}")
        })
        .unwrap_or_default()
}

fn toml_diagnostic(message: &str) -> (String, &'static str) {
    if message.starts_with("unknown field ") {
        (
            message.to_owned(),
            "remove the unknown field or use one of the listed fields",
        )
    } else if message.starts_with("missing field ") {
        (message.to_owned(), "add the required field to this table")
    } else if message.starts_with("unknown variant ") {
        unsupported_variant(message)
    } else if let Some((_, expected)) = message.split_once(", expected ") {
        expected_type(expected)
    } else if message.starts_with("duplicate ") {
        (
            "duplicate configuration key or table".to_owned(),
            "remove or merge the duplicate definition",
        )
    } else {
        syntax_diagnostic(message)
    }
}

fn unsupported_variant(message: &str) -> (String, &'static str) {
    let expectation = message.rsplit_once(", expected ").map_or_else(
        || "check the allowed values".to_owned(),
        |(_, expected)| format!("expected {expected}"),
    );
    (
        format!("unsupported value; {expectation}"),
        "replace the value with one of the allowed values",
    )
}

fn expected_type(expected: &str) -> (String, &'static str) {
    (
        format!("incorrect value or type; expected {expected}"),
        "use the expected TOML type or value",
    )
}

fn syntax_diagnostic(message: &str) -> (String, &'static str) {
    let mut quoted = false;
    let mut reason = String::new();
    for character in message.chars() {
        if character == '`' {
            quoted = !quoted;
            if quoted {
                reason.push_str("<token>");
            }
        } else if !quoted && !character.is_control() {
            reason.push(character);
        }
    }
    (
        format!("invalid TOML syntax: {reason}"),
        "check quotes, brackets, separators and key/value assignments at this location",
    )
}
