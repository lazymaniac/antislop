//! Explicit routing and workspace policy for the lean runtime.

mod model;
pub use model::*;
#[cfg(test)]
#[path = "config/tests.rs"]
mod tests;
mod validation;

use crate::{Error, Result, require};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedProfile {
    pub profile: String,
    pub provider: ProviderKind,
    pub binary: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    pub model: Option<String>,
    pub effort: Effort,
    /// None explicitly means the configured CLI has no native effort control.
    pub native_effort: Option<String>,
    pub work_kind: WorkKind,
    pub role: RoleKind,
}

pub(crate) fn resolve_command(
    provider: &ProviderConfig,
    profile: &Profile,
) -> Result<(PathBuf, Vec<String>)> {
    if let Some(cmd) = &profile.command {
        return command_and_args(&cmd.0, &profile.args, "profile");
    }
    if let Some(bin) = &profile.binary {
        return Ok((bin.clone(), profile.args.clone()));
    }
    if let Some(cmd) = &provider.command {
        let (binary, mut args) = command_and_args(&cmd.0, &provider.args, "provider")?;
        args.extend(profile.args.iter().cloned());
        return Ok((binary, args));
    }
    let mut args = provider.args.clone();
    args.extend(profile.args.iter().cloned());
    Ok((provider.binary.clone(), args))
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
    let mut args = command_args.to_vec();
    args.extend(extra_args.iter().cloned());
    Ok((PathBuf::from(binary), args))
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        Self::from_toml(&std::fs::read_to_string(path)?).map_err(|error| {
            Error::Invalid(format!("configuration file {}: {error}", path.display()))
        })
    }

    pub fn from_toml(source: &str) -> Result<Self> {
        let config: Self =
            toml::from_str(source).map_err(|error| validation::toml_error(source, error))?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        validation::validate(self)
    }

    fn profile(&self, name: &str) -> Result<&Profile> {
        self.profiles.get(name).ok_or_else(|| {
            Error::Invalid("routing refers to a missing profile; no default is selected".into())
        })
    }

    /// Resolve exact role/work assignments. A verifier always requires its own
    /// session even when executor and verifier deliberately use the same CLI.
    pub fn resolve(&self, work_kind: WorkKind, role: RoleKind) -> Result<ResolvedProfile> {
        self.validate()?;
        let name = self.profile_name(work_kind, role)?;
        let profile = self.profile(name)?;
        let provider = self
            .providers
            .get(&profile.provider)
            .ok_or_else(|| Error::Invalid("unconfigured provider".into()))?;
        let (binary, args) = resolve_command(provider, profile)?;
        Ok(ResolvedProfile {
            profile: name.into(),
            provider: profile.provider,
            binary,
            args,
            model: profile.model.clone(),
            effort: profile.effort,
            native_effort: profile
                .provider
                .native_effort(profile.effort)
                .map(str::to_owned),
            work_kind,
            role,
        })
    }

    fn profile_name(&self, work_kind: WorkKind, role: RoleKind) -> Result<&str> {
        let name = match role {
            RoleKind::Orchestrator => {
                require(
                    work_kind == WorkKind::Orchestration,
                    "orchestrator role requires orchestration work",
                )?;
                &self.orchestrator.profile
            }
            RoleKind::Executor | RoleKind::Verifier | RoleKind::Planner => {
                require(
                    role != RoleKind::Planner || work_kind == WorkKind::Planning,
                    "planner role requires planning work",
                )?;
                let route = self.routing.get(&work_kind).ok_or_else(|| {
                    Error::Invalid(
                        "no explicit route for requested work; provider fallback is forbidden"
                            .into(),
                    )
                })?;
                match role {
                    RoleKind::Verifier => &route.verifier,
                    _ => &route.executor,
                }
            }
        };
        require(
            !(work_kind == WorkKind::Orchestration && role == RoleKind::Executor),
            "orchestration execution uses the explicit Orchestrator role",
        )?;
        Ok(name)
    }
}

impl Policy {
    /// Autonomous mode runs a checked plan without waiting for the user.
    pub fn automatic_progression(&self) -> bool {
        self.mode == Mode::Autonomous
    }
}
