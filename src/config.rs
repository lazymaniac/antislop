//! Explicit, versioned routing and autonomy policy. Parsing never authorizes an
//! external effect. A caller must separately authenticate user grants before
//! presenting them to `authorize_external`.
use crate::{Error, Result, require};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Agy,
    Codex,
    Claude,
    Gemini,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    Light,
    Standard,
    Deep,
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
    Worktree,
    InPlace,
    /// Each writing task works in a private Git worktree stacked on its
    /// dependencies' results; verified results are merged into the project.
    TaskWorktrees,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IsolationProfile {
    #[default]
    ToolFree,
    PinnedContext,
    ReadOnly,
    WorkspaceWrite,
    RestrictedExecution,
    ExtendedAccess,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    pub binary: PathBuf,
    /// Path to a CLI-owned isolated home; credentials are never config values.
    #[serde(default)]
    pub managed_home: Option<PathBuf>,
    #[serde(default)]
    pub isolation: IsolationProfile,
    /// Declared capability, checked against the actual CLI probe by the adapter.
    /// False is an explicit unsupported mapping, not an ignored native request.
    pub native_effort: bool,
    #[serde(default)]
    pub effort_map: BTreeMap<Effort, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub provider: ProviderKind,
    pub model: Option<String>,
    pub effort: Effort,
    pub work_kinds: BTreeSet<WorkKind>,
    pub roles: BTreeSet<RoleKind>,
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
    /// Reply pagination, never a limit on the complete set of checks. Zero
    /// retains the legacy single-report transport for compatibility/probes.
    #[serde(
        default = "default_review_checks",
        skip_serializing_if = "default_review_page"
    )]
    pub review_checks_per_part: usize,
}
pub fn default_review_checks() -> usize {
    24
}
fn default_review_page(value: &usize) -> bool {
    *value == default_review_checks()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub mode: Mode,
    pub revision: u64,
    pub local_write: bool,
    pub local_checks: bool,
    pub local_commits: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    #[serde(default)]
    pub workspace: WorkspaceConfig,
    pub providers: BTreeMap<ProviderKind, ProviderConfig>,
    pub profiles: BTreeMap<String, Profile>,
    /// Explicit user limits retained when the orchestrator proposes new profiles.
    /// An absent role is unrestricted; a present role needs a nonempty set.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub role_providers: BTreeMap<RoleKind, BTreeSet<ProviderKind>>,
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
    pub managed_home: Option<PathBuf>,
    pub isolation: IsolationProfile,
    pub model: Option<String>,
    pub effort: Effort,
    /// None explicitly means the configured CLI has no native effort control.
    pub native_effort: Option<String>,
    pub work_kind: WorkKind,
    pub role: RoleKind,
    pub requires_independent_session: bool,
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

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        Self::from_toml(&std::fs::read_to_string(path)?)
    }

    pub fn from_toml(source: &str) -> Result<Self> {
        // TOML errors can render the input and expose accidental credentials.
        let config: Self = toml::from_str(source).map_err(|error: toml::de::Error| {
            let location = error.span().map(|span| format!(" at bytes {}..{}", span.start, span.end)).unwrap_or_default();
            Error::Invalid(format!("invalid TOML configuration{location}: check schema, syntax and duplicate/unknown fields"))
        })?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<()> {
        require(
            self.schema_version == 1,
            "unsupported configuration schema version",
        )?;
        require(self.policy.revision > 0, "policy revision must be positive")?;
        require(
            !self.providers.is_empty(),
            "at least one explicit provider is required",
        )?;
        require(
            !self.profiles.is_empty(),
            "at least one explicit profile is required",
        )?;
        for provider in self.providers.values() {
            require(
                provider.binary.to_str().is_some_and(nonempty_text),
                "provider binary must be a nonempty Unicode path without control characters",
            )?;
            require(
                provider.managed_home.as_ref().is_none_or(|path| {
                    path.is_absolute() && path.to_str().is_some_and(nonempty_text)
                }),
                "managed provider home must be an absolute Unicode path without control characters",
            )?;
            require(
                provider.native_effort || provider.effort_map.is_empty(),
                "native effort mappings require declared CLI support",
            )?;
            require(
                provider
                    .effort_map
                    .values()
                    .all(|value| nonempty_text(value)),
                "native effort mapping must be nonempty and contain no control characters",
            )?;
        }
        for providers in self.role_providers.values() {
            require(
                !providers.is_empty() && providers.iter().all(|p| self.providers.contains_key(p)),
                "role provider limits require nonempty configured provider choices",
            )?;
        }
        for (name, profile) in &self.profiles {
            require(
                identifier(name),
                "profile name must contain only letters, digits, underscores or hyphens",
            )?;
            require(
                !profile.work_kinds.is_empty() && !profile.roles.is_empty(),
                "profile requires explicit work kinds and roles",
            )?;
            require(
                profile
                    .model
                    .as_ref()
                    .is_none_or(|model| nonempty_text(model)),
                "model must be nonempty and contain no control characters",
            )?;
            require(
                profile.roles.iter().all(|role| {
                    self.role_providers
                        .get(role)
                        .is_none_or(|allowed| allowed.contains(&profile.provider))
                }),
                "profile provider violates the user-configured role provider limit",
            )?;
            let provider = self.providers.get(&profile.provider).ok_or_else(|| {
                Error::Invalid("profile refers to an unconfigured provider".into())
            })?;
            require(
                !provider.native_effort || provider.effort_map.contains_key(&profile.effort),
                "profile effort has no native CLI mapping",
            )?;
            if profile.roles.contains(&RoleKind::Orchestrator) {
                require(
                    profile.work_kinds.contains(&WorkKind::Orchestration),
                    "orchestrator profile must permit orchestration",
                )?;
            }
        }
        self.profile(
            &self.orchestrator.profile,
            WorkKind::Orchestration,
            RoleKind::Orchestrator,
        )?;
        for (kind, route) in &self.routing {
            if *kind == WorkKind::Orchestration {
                require(
                    route.executor == self.orchestrator.profile,
                    "orchestration routing must match the explicit orchestrator profile",
                )?;
                self.profile(&route.executor, *kind, RoleKind::Orchestrator)?;
            } else {
                self.profile(&route.executor, *kind, RoleKind::Executor)?;
            }
            self.profile(&route.verifier, *kind, RoleKind::Verifier)?;
        }
        Ok(())
    }

    fn profile(&self, name: &str, work_kind: WorkKind, role: RoleKind) -> Result<&Profile> {
        let profile = self.profiles.get(name).ok_or_else(|| {
            Error::Invalid("routing refers to a missing profile; no default is selected".into())
        })?;
        require(
            profile.work_kinds.contains(&work_kind),
            "profile does not allow the requested work kind",
        )?;
        require(
            profile.roles.contains(&role),
            "profile does not allow the requested role",
        )?;
        Ok(profile)
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
        let profile = self.profile(name, work_kind, role)?;
        let provider = self
            .providers
            .get(&profile.provider)
            .ok_or_else(|| Error::Invalid("unconfigured provider".into()))?;
        Ok(ResolvedProfile {
            profile: name.clone(),
            provider: profile.provider,
            binary: provider.binary.clone(),
            managed_home: provider.managed_home.clone(),
            isolation: provider.isolation,
            model: profile.model.clone(),
            effort: profile.effort,
            native_effort: provider.effort_map.get(&profile.effort).cloned(),
            work_kind,
            role,
            requires_independent_session: role == RoleKind::Verifier,
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
    fn the_example_routes_every_kind_to_a_worker_and_a_verifier() {
        let text = include_str!("../examples/architect.toml");
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
            assert!(verifier.requires_independent_session);
        }
        let orchestrator = config
            .resolve(WorkKind::Orchestration, RoleKind::Orchestrator)
            .unwrap();
        assert_eq!(orchestrator.provider, ProviderKind::Codex);
        assert_eq!(orchestrator.native_effort.as_deref(), Some("high"));
    }

    #[test]
    fn a_route_to_a_missing_profile_is_rejected() {
        let text = include_str!("../examples/architect.toml").replacen(
            "executor = \"reasoning\"",
            "executor = \"nobody\"",
            1,
        );
        assert!(Config::from_toml(&text).is_err());
    }
}
