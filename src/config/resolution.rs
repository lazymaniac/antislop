//! Resolve configured provider and profile commands for runtime launchers.

use super::{Profile, ProviderConfig};
use crate::{Error, Result};
use std::path::PathBuf;

pub(crate) fn resolve_command(
    provider: &ProviderConfig,
    profile: &Profile,
) -> Result<(PathBuf, Vec<String>)> {
    if let Some(command) = &profile.command {
        return command_and_args(&command.0, &profile.args, "profile");
    }
    if let Some(binary) = &profile.binary {
        return Ok((binary.clone(), profile.args.clone()));
    }
    if let Some(command) = &provider.command {
        return provider_command(provider, &command.0, &profile.args);
    }
    Ok((
        provider.binary.clone(),
        merge_args(&provider.args, &profile.args),
    ))
}

fn provider_command(
    provider: &ProviderConfig,
    command: &[String],
    profile_args: &[String],
) -> Result<(PathBuf, Vec<String>)> {
    let (binary, args) = command_and_args(command, &provider.args, "provider")?;
    Ok((binary, merge_args(&args, profile_args)))
}

fn command_and_args(
    command: &[String],
    extra_args: &[String],
    source: &str,
) -> Result<(PathBuf, Vec<String>)> {
    let Some((binary, command_args)) = command.split_first() else {
        return Err(Error::Invalid(format!(
            "{source} command must contain at least an executable"
        )));
    };
    Ok((PathBuf::from(binary), merge_args(command_args, extra_args)))
}

fn merge_args(first: &[String], second: &[String]) -> Vec<String> {
    first.iter().chain(second).cloned().collect()
}
