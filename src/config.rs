//! Explicit routing and workspace policy for the lean runtime.
use crate::{Error, Result, require};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Agy,
    Codex,
    Claude,
    Gemini,
    Opencode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkKind {
    Orchestration,
    Context,
    Requirements,
    Options,
    Planning,
    Architecture,
    Dependencies,
    Implementation,
    Refactoring,
    StaticAnalysis,
    TestDesign,
    Testing,
    Security,
    Performance,
    Integration,
    Research,
    Writing,
    Analysis,
    Delivery,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleKind {
    Executor,
    Verifier,
    Orchestrator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Supervised,
    Autonomous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkspaceMode {
    InPlace,
    /// Each writing task works in a private Git worktree stacked on its
    /// dependencies' results; verified results are merged into the project.
    TaskWorktrees,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceConfig {
    pub mode: WorkspaceMode,
}

impl Default for WorkspaceConfig {
    fn default() -> Self {
        Self {
            mode: WorkspaceMode::TaskWorktrees,
        }
    }
}

impl ProviderKind {
    /// Pass the requested effort to the CLI; model-specific support is checked
    /// by the provider. Gemini has no native effort flag.
    pub fn native_effort(self, effort: Effort) -> Option<&'static str> {
        match self {
            Self::Codex | Self::Agy | Self::Claude | Self::Opencode => Some(match effort {
                Effort::Low => "low",
                Effort::Medium => "medium",
                Effort::High => "high",
                Effort::Xhigh => "xhigh",
            }),
            Self::Gemini => None,
        }
    }
}

/// Command line tokens for launching an agent CLI, specified either as a
/// whitespace-separated string (`"omlx launch codex"`) or an explicit array
/// (`["omlx", "launch", "codex"]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct CommandLine(pub Vec<String>);

impl<'de> Deserialize<'de> for CommandLine {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Text(String),
            Args(Vec<String>),
        }
        match Raw::deserialize(deserializer)? {
            Raw::Text(text) => Ok(Self(text.split_whitespace().map(str::to_owned).collect())),
            Raw::Args(args) => Ok(Self(args)),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    #[serde(default)]
    pub binary: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<CommandLine>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub provider: ProviderKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binary: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<CommandLine>,
    pub model: Option<String>,
    pub effort: Effort,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleRouting {
    pub executor: String,
    pub verifier: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrchestratorConfig {
    pub profile: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub mode: Mode,
    pub local_write: bool,
    pub local_commits: bool,
    /// Every plan that changes code gets the required quality gates from the
    /// predefined agent pool (refactoring, static analysis, testing, security and
    /// performance audits, integration) it lacks.
    #[serde(default = "default_required_gates")]
    pub required_gates: bool,
    /// Most tasks of a plan that run at the same time.
    #[serde(default = "default_max_parallel")]
    pub max_parallel: usize,
}

fn default_required_gates() -> bool {
    true
}

fn default_max_parallel() -> usize {
    4
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    pub providers: BTreeMap<ProviderKind, ProviderConfig>,
    pub profiles: BTreeMap<String, Profile>,
    pub orchestrator: OrchestratorConfig,
    pub routing: BTreeMap<WorkKind, RoleRouting>,
    pub policy: Policy,
}

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

fn identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}
fn nonempty_text(value: &str) -> bool {
    !value.trim().is_empty() && !value.chars().any(char::is_control)
}

fn resolve_command(provider: &ProviderConfig, profile: &Profile) -> Result<(PathBuf, Vec<String>)> {
    if let Some(cmd) = &profile.command {
        let Some((first, rest)) = cmd.0.split_first() else {
            return Err(Error::Invalid(
                "profile command must contain at least an executable".into(),
            ));
        };
        let mut args = rest.to_vec();
        args.extend(profile.args.iter().cloned());
        return Ok((PathBuf::from(first), args));
    }
    if let Some(bin) = &profile.binary {
        return Ok((bin.clone(), profile.args.clone()));
    }
    if let Some(cmd) = &provider.command {
        let Some((first, rest)) = cmd.0.split_first() else {
            return Err(Error::Invalid(
                "provider command must contain at least an executable".into(),
            ));
        };
        let mut args = rest.to_vec();
        args.extend(provider.args.iter().cloned());
        args.extend(profile.args.iter().cloned());
        return Ok((PathBuf::from(first), args));
    }
    let mut args = provider.args.clone();
    args.extend(profile.args.iter().cloned());
    Ok((provider.binary.clone(), args))
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        Self::from_toml(&std::fs::read_to_string(path)?)
    }

    pub fn from_toml(source: &str) -> Result<Self> {
        // TOML errors can render the input and expose accidental credentials.
        let config: Self = toml::from_str(source).map_err(|error: toml::de::Error| {
            let location = error
                .span()
                .map(|span| format!(" at bytes {}..{}", span.start, span.end))
                .unwrap_or_default();
            Error::Invalid(format!(
                "invalid TOML configuration{location}: check schema, syntax and duplicate/unknown fields"
            ))
        })?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        require(
            self.schema_version == 1,
            "unsupported configuration schema version",
        )?;
        require(
            self.policy.max_parallel > 0,
            "policy max_parallel must be at least 1",
        )?;
        require(
            !self.providers.is_empty(),
            "at least one explicit provider is required",
        )?;
        require(
            !self.profiles.is_empty(),
            "at least one explicit profile is required",
        )?;
        for provider in self.providers.values() {
            if let Some(cmd) = &provider.command {
                require(
                    !cmd.0.is_empty() && cmd.0.iter().all(|arg| nonempty_text(arg)),
                    "provider command tokens must be nonempty without control characters",
                )?;
            } else {
                require(
                    provider.binary.to_str().is_some_and(nonempty_text),
                    "provider binary must be a nonempty Unicode path without control characters",
                )?;
            }
            require(
                provider.args.iter().all(|arg| nonempty_text(arg)),
                "provider args must be nonempty and contain no control characters",
            )?;
        }
        for (name, profile) in &self.profiles {
            require(
                identifier(name),
                "profile name must contain only letters, digits, underscores or hyphens",
            )?;
            require(
                profile
                    .model
                    .as_ref()
                    .is_none_or(|model| nonempty_text(model)),
                "model must be nonempty and contain no control characters",
            )?;
            if let Some(bin) = &profile.binary {
                require(
                    bin.to_str().is_some_and(nonempty_text),
                    "profile binary must be a nonempty Unicode path without control characters",
                )?;
            }
            if let Some(cmd) = &profile.command {
                require(
                    !cmd.0.is_empty() && cmd.0.iter().all(|arg| nonempty_text(arg)),
                    "profile command tokens must be nonempty without control characters",
                )?;
            }
            require(
                profile.args.iter().all(|arg| nonempty_text(arg)),
                "profile args must be nonempty and contain no control characters",
            )?;
            let provider = self.providers.get(&profile.provider).ok_or_else(|| {
                Error::Invalid("profile refers to an unconfigured provider".into())
            })?;
            let (binary, _) = resolve_command(provider, profile)?;
            require(
                binary.to_str().is_some_and(nonempty_text),
                "resolved profile binary must be a nonempty Unicode path without control characters",
            )?;
        }
        self.profile(&self.orchestrator.profile)?;
        for (kind, route) in &self.routing {
            if *kind == WorkKind::Orchestration {
                require(
                    route.executor == self.orchestrator.profile,
                    "orchestration routing must match the explicit orchestrator profile",
                )?;
            }
            self.profile(&route.executor)?;
            self.profile(&route.verifier)?;
        }
        // The gates added to plans that change code need their routes now,
        // not when a plan reaches them.
        if self.policy.required_gates {
            for kind in crate::lean::tasks::GATE_KINDS {
                require(
                    self.routing.contains_key(&kind),
                    &format!(
                        "policy.required_gates adds {kind:?} tasks to plans that change code, \
                         but there is no route for that work: add one or set required_gates = false"
                    ),
                )?;
            }
        }
        Ok(())
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
        let name = match role {
            RoleKind::Orchestrator => {
                require(
                    work_kind == WorkKind::Orchestration,
                    "orchestrator role requires orchestration work",
                )?;
                &self.orchestrator.profile
            }
            RoleKind::Executor | RoleKind::Verifier => {
                let route = self.routing.get(&work_kind).ok_or_else(|| {
                    Error::Invalid(
                        "no explicit route for requested work; provider fallback is forbidden"
                            .into(),
                    )
                })?;
                match role {
                    RoleKind::Executor => &route.executor,
                    _ => &route.verifier,
                }
            }
        };
        require(
            !(work_kind == WorkKind::Orchestration && role == RoleKind::Executor),
            "orchestration execution uses the explicit Orchestrator role",
        )?;
        let profile = self.profile(name)?;
        let provider = self
            .providers
            .get(&profile.provider)
            .ok_or_else(|| Error::Invalid("unconfigured provider".into()))?;
        let (binary, args) = resolve_command(provider, profile)?;
        Ok(ResolvedProfile {
            profile: name.clone(),
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
}

impl Policy {
    /// Autonomous mode runs a checked plan without waiting for the user.
    pub fn automatic_progression(&self) -> bool {
        self.mode == Mode::Autonomous
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effort_levels_reach_the_cli() {
        let example = include_str!("../examples/antislop.toml");
        for (name, expected) in [
            ("low", "low"),
            ("medium", "medium"),
            ("high", "high"),
            ("xhigh", "xhigh"),
        ] {
            let config = Config::from_toml(
                &example.replace("effort = \"high\"", &format!("effort = \"{name}\"")),
            )
            .unwrap();
            for provider in [
                ProviderKind::Codex,
                ProviderKind::Agy,
                ProviderKind::Claude,
                ProviderKind::Opencode,
                ProviderKind::Gemini,
            ] {
                let mut config = config.clone();
                let provider_config = config.providers[&ProviderKind::Codex].clone();
                config.providers.insert(provider, provider_config);
                config.profiles.get_mut("orchestrator").unwrap().provider = provider;
                let resolved = config
                    .resolve(WorkKind::Orchestration, RoleKind::Orchestrator)
                    .unwrap();
                assert_eq!(
                    resolved.native_effort.as_deref(),
                    (provider != ProviderKind::Gemini).then_some(expected)
                );
                assert_eq!(serde_json::to_value(resolved.effort).unwrap(), expected);
            }
        }
        assert!(
            Config::from_toml(&example.replace("effort = \"high\"", "effort = \"typo\"")).is_err()
        );
    }

    #[test]
    fn the_example_routes_every_kind_to_a_worker_and_a_verifier() {
        let text = include_str!("../examples/antislop.toml");
        let config = Config::from_toml(text).unwrap();
        for kind in config
            .routing
            .keys()
            .filter(|kind| **kind != WorkKind::Orchestration)
        {
            assert_eq!(
                config.resolve(*kind, RoleKind::Executor).unwrap().work_kind,
                *kind
            );
            let verifier = config.resolve(*kind, RoleKind::Verifier).unwrap();
            assert_eq!(verifier.role, RoleKind::Verifier);
        }
        let orchestrator = config
            .resolve(WorkKind::Orchestration, RoleKind::Orchestrator)
            .unwrap();
        assert_eq!(orchestrator.provider, ProviderKind::Codex);
        assert_eq!(orchestrator.native_effort.as_deref(), Some("high"));
    }

    #[test]
    fn profile_command_supports_platform_launchers_for_workers_and_verifiers() {
        let text = include_str!("../examples/antislop.toml")
            .replace(
                "[profiles.execution]\nprovider = \"agy\"",
                "[profiles.execution]\nprovider = \"codex\"\ncommand = \"omlx launch codex\"",
            )
            .replace(
                "[profiles.review]\nprovider = \"agy\"",
                "[profiles.review]\nprovider = \"codex\"\ncommand = [\"ollama\", \"launch\", \"codex\"]",
            );
        let config = Config::from_toml(&text).unwrap();
        let orchestrator = config
            .resolve(WorkKind::Orchestration, RoleKind::Orchestrator)
            .unwrap();
        assert_eq!(orchestrator.binary, PathBuf::from("codex"));
        assert!(orchestrator.args.is_empty());

        let worker = config
            .resolve(WorkKind::Implementation, RoleKind::Executor)
            .unwrap();
        assert_eq!(worker.provider, ProviderKind::Codex);
        assert_eq!(worker.binary, PathBuf::from("omlx"));
        assert_eq!(worker.args, vec!["launch", "codex"]);
        assert_eq!(worker.native_effort.as_deref(), Some("high"));

        let verifier = config
            .resolve(WorkKind::Implementation, RoleKind::Verifier)
            .unwrap();
        assert_eq!(verifier.provider, ProviderKind::Codex);
        assert_eq!(verifier.binary, PathBuf::from("ollama"));
        assert_eq!(verifier.args, vec!["launch", "codex"]);
        assert_eq!(verifier.native_effort.as_deref(), Some("medium"));
    }

    #[test]
    fn a_route_to_a_missing_profile_is_rejected() {
        let mut config = Config::from_toml(include_str!("../examples/antislop.toml")).unwrap();
        config.routing.get_mut(&WorkKind::Context).unwrap().executor = "nobody".into();
        assert!(config.validate().is_err());
    }

    #[test]
    fn routing_assigns_a_shared_profile_to_different_work_kinds_and_roles() {
        let mut config = Config::from_toml(include_str!("../examples/antislop.toml")).unwrap();
        let shared = config.profiles["execution"].clone();
        config.profiles = BTreeMap::from([("shared".into(), shared)]);
        config.orchestrator.profile = "shared".into();
        for route in config.routing.values_mut() {
            route.executor = "shared".into();
            route.verifier = "shared".into();
        }
        config.validate().unwrap();
        for kind in config.routing.keys().copied() {
            let worker_role = if kind == WorkKind::Orchestration {
                RoleKind::Orchestrator
            } else {
                RoleKind::Executor
            };
            for role in [worker_role, RoleKind::Verifier] {
                let resolved = config.resolve(kind, role).unwrap();
                assert_eq!(resolved.profile, "shared");
                assert_eq!(resolved.work_kind, kind);
                assert_eq!(resolved.role, role);
            }
        }
        // Profile reuse must not introduce implicit routes or invalid roles.
        assert!(
            config
                .resolve(WorkKind::Implementation, RoleKind::Orchestrator)
                .is_err()
        );
        assert!(
            config
                .resolve(WorkKind::Orchestration, RoleKind::Executor)
                .is_err()
        );
        config.routing.remove(&WorkKind::Context);
        assert!(
            config
                .resolve(WorkKind::Context, RoleKind::Executor)
                .is_err()
        );
        assert!(
            config
                .resolve(WorkKind::Context, RoleKind::Verifier)
                .is_err()
        );
    }

    #[test]
    fn assignments_still_require_existing_profiles_and_consistent_orchestration() {
        let original = Config::from_toml(include_str!("../examples/antislop.toml")).unwrap();
        let mut config = original.clone();
        config.orchestrator.profile = "missing".into();
        assert!(config.validate().is_err());

        let mut config = original.clone();
        config.routing.get_mut(&WorkKind::Context).unwrap().verifier = "missing".into();
        assert!(config.validate().is_err());

        let mut config = original;
        config
            .routing
            .get_mut(&WorkKind::Orchestration)
            .unwrap()
            .executor = "execution".into();
        let error = config.validate().unwrap_err().to_string();
        assert!(
            error.contains("orchestration routing must match"),
            "{error}"
        );
    }

    #[test]
    fn platform_workers_use_opencode_and_verifiers_use_claude() {
        // Test a fixed routing fixture rather than the user's editable local
        // antislop.toml. Changing local preferences must not break the suite.
        let mut fixture = include_str!("../examples/antislop.toml")
            .parse::<toml_edit::DocumentMut>()
            .unwrap();
        fixture["providers"]["opencode"]["command"] = toml_edit::value(
            [
                "omlx",
                "launch",
                "opencode",
                "--model",
                "Qwen3.8-Flash-Next-Uncensored-oQ5e-mtp",
                "--",
            ]
            .into_iter()
            .collect::<toml_edit::Array>(),
        );
        fixture["providers"]["claude"]["binary"] = toml_edit::value("claude");
        for name in ["reasoning", "execution"] {
            fixture["profiles"][name]["provider"] = toml_edit::value("opencode");
            fixture["profiles"][name]["model"] =
                toml_edit::value("omlx/Qwen3.8-Flash-Next-Uncensored-oQ5e-mtp");
        }
        fixture["profiles"]["review"]["provider"] = toml_edit::value("claude");
        fixture["profiles"]["review"]["effort"] = toml_edit::value("high");
        let config = Config::from_toml(&fixture.to_string()).unwrap();
        let orchestrator = config
            .resolve(WorkKind::Orchestration, RoleKind::Orchestrator)
            .unwrap();
        assert_eq!(orchestrator.provider, ProviderKind::Codex);
        for kind in config.routing.keys() {
            for role in [RoleKind::Executor, RoleKind::Verifier] {
                if *kind == WorkKind::Orchestration && role == RoleKind::Executor {
                    continue;
                }
                let profile = config.resolve(*kind, role).unwrap();
                if role == RoleKind::Verifier {
                    assert_eq!(profile.provider, ProviderKind::Claude);
                    assert_eq!(profile.binary, PathBuf::from("claude"));
                    assert_eq!(profile.native_effort.as_deref(), Some("high"));
                    continue;
                }
                assert_eq!(profile.provider, ProviderKind::Opencode);
                assert_eq!(profile.binary, PathBuf::from("omlx"));
                assert_eq!(
                    profile.args,
                    [
                        "launch",
                        "opencode",
                        "--model",
                        "Qwen3.8-Flash-Next-Uncensored-oQ5e-mtp",
                        "--"
                    ]
                );
                assert_eq!(
                    profile.model.as_deref(),
                    Some("omlx/Qwen3.8-Flash-Next-Uncensored-oQ5e-mtp")
                );
                assert_eq!(profile.native_effort.as_deref(), Some("high"));
            }
        }
    }

    #[test]
    fn max_parallel_defaults_to_four_and_must_be_positive() {
        let text = include_str!("../examples/antislop.toml");
        let default = Config::from_toml(&text.replace("max_parallel = 4\n", "")).unwrap();
        assert_eq!(default.policy.max_parallel, 4);
        assert!(Config::from_toml(&text.replace("max_parallel = 4", "max_parallel = 0")).is_err());
        let one = Config::from_toml(&text.replace("max_parallel = 4", "max_parallel = 1")).unwrap();
        assert_eq!(one.policy.max_parallel, 1);
    }

    #[test]
    fn obsolete_configuration_is_rejected_without_echoing_values() {
        let example = include_str!("../examples/antislop.toml");
        for (header, field) in [
            ("[providers.codex]", "isolation = \"read_only\""),
            ("[providers.codex]", "isolation_levels = [\"read_only\"]"),
            ("[providers.codex]", "managed_home = \"/secret/home\""),
            ("[providers.codex]", "effort_map = { high = \"secret\" }"),
            ("[profiles.orchestrator]", "native_effort = \"secret\""),
            (
                "[profiles.orchestrator]",
                "work_kinds = [\"orchestration\"]",
            ),
            ("[profiles.orchestrator]", "roles = [\"orchestrator\"]"),
            ("[policy]", "revision = 1"),
            ("[policy]", "local_checks = true"),
            ("[orchestrator]", "review_checks_per_part = 24"),
            ("", "role_providers = { executor = [\"agy\"] }"),
        ] {
            let source = if header.is_empty() {
                format!("{field}\n{example}")
            } else {
                example.replace(header, &format!("{header}\n{field}"))
            };
            let error = Config::from_toml(&source).unwrap_err().to_string();
            assert!(
                error.contains("invalid TOML configuration"),
                "{field}: {error}"
            );
            assert!(!error.contains("secret"), "{error}");
        }
        for effort in ["light", "standard", "deep"] {
            assert!(
                Config::from_toml(
                    &example.replace("effort = \"high\"", &format!("effort = \"{effort}\""))
                )
                .is_err()
            );
        }
        assert!(
            Config::from_toml(&example.replace("mode = \"task-worktrees\"", "mode = \"worktree\""))
                .is_err()
        );
    }

    #[test]
    fn required_gates_need_routes_for_the_work_they_add() {
        let text = include_str!("../examples/antislop.toml");
        assert!(Config::from_toml(text).is_ok());
        // Find the first route table of a gate kind and take it away.
        let header = text
            .lines()
            .find(|line| line.starts_with("[routing.") && line.contains("static_analysis"))
            .expect("the example routes static analysis");
        let start = text.find(header).unwrap();
        let end = text[start + header.len()..]
            .find("\n[")
            .map_or(text.len(), |at| start + header.len() + at);
        let without = format!("{}{}", &text[..start], &text[end..]);
        let error = Config::from_toml(&without).unwrap_err().to_string();
        assert!(error.contains("required_gates"), "{error}");
        // With the gates off, the route is not needed.
        let off = without.replace("required_gates = true", "required_gates = false");
        assert!(Config::from_toml(&off).is_ok());
    }
}
