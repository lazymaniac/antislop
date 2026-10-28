//! Configuration types shared by routing and runtime setup.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

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
    Orchestration, // Coordinates agents, task dependencies, and progress toward the overall goal.
    Context, // Inspects project files to establish relevant context, contracts, and conventions.
    Requirements, // Extracts requirements, acceptance criteria, and contradictions from the brief.
    Options, // Compares realistic alternatives against common criteria and selects an approach.
    Planning, // Breaks the goal into actionable tasks with dependencies and completion criteria.
    Architecture, // Designs how components and interfaces fit the existing project structure.
    Dependencies, // Evaluates libraries, their maintenance, and dependency license obligations.
    Implementation, // Implements the requested behavior in the project's source files.
    Refactoring, // Improves code structure while preserving existing behavior.
    StaticAnalysis, // Runs configured linters and analyzers and investigates their findings.
    TestDesign, // Defines test scenarios covering requirements, likely failures, and interacting inputs.
    Testing,    // Implements and runs tests that verify behavior through meaningful assertions.
    Security,   // Assesses relevant threats and investigates security vulnerabilities and fixes.
    Performance, // Measures performance against a baseline and evaluates optimizations.
    Integration, // Checks that combined changes build, pass checks, and work across module boundaries.
    Research,    // Gathers evidence from sources to answer questions and resolve unknowns.
    Writing, // Produces documentation or other written content consistent with the brief and sources.
    Analysis, // Examines data and assumptions to derive reproducible conclusions.
    Delivery, // Summarizes delivered results, usage instructions, limitations, and remaining risks.
}

impl WorkKind {
    /// Whether work of this kind changes files unless the plan says otherwise.
    pub fn writes_by_default(self) -> bool {
        matches!(
            self,
            Self::Implementation
                | Self::Refactoring
                | Self::Testing
                | Self::StaticAnalysis
                | Self::Integration
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoleKind {
    Executor,
    Verifier,
    Orchestrator,
    /// Drafts the first plan from the agreed requirements.
    Planner,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Supervised,
    Autonomous,
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
    /// Every writer gets a dedicated read-only options task that compares
    /// approaches before it starts. Off, writers start straight from their
    /// contract: for mechanical work (a refactor to fixed limits) the
    /// comparison costs two agent runs per writer and decides nothing.
    #[serde(default = "default_local_options")]
    pub local_options: bool,
    /// Most tasks a plan may be drafted with, before the runtime adds options
    /// and gates. A larger plan is sent back to be grouped by module.
    #[serde(default = "default_max_tasks")]
    pub max_tasks: usize,
}

fn default_local_options() -> bool {
    true
}

fn default_max_tasks() -> usize {
    24
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
    pub providers: BTreeMap<ProviderKind, ProviderConfig>,
    pub profiles: BTreeMap<String, Profile>,
    pub orchestrator: OrchestratorConfig,
    pub routing: BTreeMap<WorkKind, RoleRouting>,
    pub policy: Policy,
}
