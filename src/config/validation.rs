//! Configuration diagnostics and semantic validation.

use super::{Config, WorkKind, resolve_command};
use crate::{Error, Result, require};

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
        (
            format!(
                "unsupported value; {}",
                message.rsplit_once(", expected ").map_or(
                    "check the allowed values".to_owned(),
                    |(_, expected)| format!("expected {expected}")
                )
            ),
            "replace the value with one of the allowed values",
        )
    } else if let Some((_, expected)) = message.split_once(", expected ") {
        (
            format!("incorrect value or type; expected {expected}"),
            "use the expected TOML type or value",
        )
    } else if message.starts_with("duplicate ") {
        (
            "duplicate configuration key or table".to_owned(),
            "remove or merge the duplicate definition",
        )
    } else {
        // Parser diagnostics may quote arbitrary input. Keep their
        // structural explanation while hiding quoted input fragments.
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
}

pub(super) fn validate(config: &Config) -> Result<()> {
    require(
        config.schema_version == 1,
        "schema_version is unsupported; set schema_version = 1",
    )?;
    require(
        config.policy.max_parallel > 0,
        "policy.max_parallel must be at least 1; set max_parallel to a positive integer",
    )?;
    require(
        !config.providers.is_empty(),
        "providers is empty; add a [providers.<name>] table with a binary or command",
    )?;
    require(
        !config.profiles.is_empty(),
        "profiles is empty; add a [profiles.<name>] table with provider and effort",
    )?;
    validate_providers(config)?;
    validate_profiles(config)?;
    validate_routing(config)
}

fn validate_providers(config: &Config) -> Result<()> {
    for (name, provider) in &config.providers {
        if let Some(cmd) = &provider.command {
            require(
                !cmd.0.is_empty() && cmd.0.iter().all(|arg| nonempty_text(arg)),
                &format!(
                    "providers.{name:?}: provider command tokens must be nonempty without control characters; replace the invalid command tokens"
                ),
            )?;
        } else {
            require(
                provider.binary.to_str().is_some_and(nonempty_text),
                &format!(
                    "providers.{name:?}: provider binary must be a nonempty Unicode path without control characters; replace the invalid binary"
                ),
            )?;
        }
        require(
            provider.args.iter().all(|arg| nonempty_text(arg)),
            &format!(
                "providers.{name:?}: provider args must be nonempty and contain no control characters; replace the invalid args"
            ),
        )?;
    }
    Ok(())
}

fn validate_profiles(config: &Config) -> Result<()> {
    for (name, profile) in &config.profiles {
        require(
            identifier(name),
            &format!(
                "profiles.{name:?}: profile name is invalid; rename it using only letters, digits, underscores or hyphens"
            ),
        )?;
        require(
            profile
                .model
                .as_ref()
                .is_none_or(|model| nonempty_text(model)),
            &format!(
                "profiles.{name:?}: model must be nonempty and contain no control characters; set a valid model name or remove model"
            ),
        )?;
        if let Some(bin) = &profile.binary {
            require(
                bin.to_str().is_some_and(nonempty_text),
                &format!(
                    "profiles.{name:?}: binary must be a nonempty Unicode path without control characters; replace binary with a valid executable path"
                ),
            )?;
        }
        if let Some(cmd) = &profile.command {
            require(
                !cmd.0.is_empty() && cmd.0.iter().all(|arg| nonempty_text(arg)),
                &format!(
                    "profiles.{name:?}: command tokens must be nonempty without control characters; provide an executable followed by valid arguments"
                ),
            )?;
        }
        require(
            profile.args.iter().all(|arg| nonempty_text(arg)),
            &format!(
                "profiles.{name:?}: args must be nonempty and contain no control characters; remove or replace invalid arguments"
            ),
        )?;
        let provider = config.providers.get(&profile.provider).ok_or_else(|| {
            Error::Invalid(format!(
                "profiles.{name:?}.provider: profile refers to an unconfigured provider; \
                 add the matching [providers.<name>] table or select a configured provider"
            ))
        })?;
        let (binary, _) = resolve_command(provider, profile)?;
        require(
            binary.to_str().is_some_and(nonempty_text),
            "resolved profile binary must be a nonempty Unicode path without control characters",
        )?;
    }
    Ok(())
}

fn validate_routing(config: &Config) -> Result<()> {
    require_profile(config, "orchestrator.profile", &config.orchestrator.profile)?;
    for (kind, route) in &config.routing {
        if *kind == WorkKind::Orchestration {
            require(
                route.executor == config.orchestrator.profile,
                "routing.orchestration.executor must match orchestrator.profile; set both fields to the same configured profile",
            )?;
        }
        for (role, name) in [("executor", &route.executor), ("verifier", &route.verifier)] {
            let path = format!(
                "routing.{}.{role}",
                crate::lean::tasks::work_kind_name(*kind)
            );
            require_profile(config, &path, name)?;
        }
    }
    require(
        config.routing.contains_key(&WorkKind::Options),
        "routing.options is required even when policy.required_gates = false; \
             add an options route with executor and verifier profiles",
    )?;
    // The gates added to plans that change code need their routes now,
    // not when a plan reaches them.
    if config.policy.required_gates {
        for kind in crate::lean::tasks::GATE_KINDS {
            require(
                config.routing.contains_key(&kind),
                &format!(
                    "policy.required_gates adds {kind:?} tasks to plans that change code, \
                         but there is no route for that work: add one or set required_gates = false"
                ),
            )?;
        }
    }
    Ok(())
}

fn require_profile(config: &Config, path: &str, name: &str) -> Result<()> {
    require(
        config.profiles.contains_key(name),
        &format!(
            "{path} refers to a missing profile; set it to a name defined in [profiles.<name>]"
        ),
    )
}

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn nonempty_text(value: &str) -> bool {
    !value.trim().is_empty() && !value.chars().any(char::is_control)
}
