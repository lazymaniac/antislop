//! Persistable teams and a dependency scheduler. Leases coordinate managed tasks;
//! they are not a filesystem security boundary. Runtime supplies authenticated
//! sessions, current gate evidence, an effect runner and a durable checkpoint.
pub mod changes;
pub mod checks;
pub mod models;
use crate::{
    Error, Result,
    config::{Config, ResolvedProfile, RoleKind, WorkKind},
    domain::RunSpec,
    hash,
    quality::TaskFamily,
    require,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogRole {
    Orchestrator,
    ContextAnalyst,
    RequirementsAnalyst,
    Planner,
    OptionsAnalyst,
    Architect,
    DependencyAnalyst,
    Implementer,
    Refactorer,
    StaticAnalysisOperator,
    ComplianceAuditor,
    TestDesigner,
    TestImplementer,
    SecurityAnalyst,
    PerformanceEngineer,
    Integrator,
    Researcher,
    DataAnalyst,
    Writer,
    DeliveryAuthor,
}
impl CatalogRole {
    pub const ALL: [Self; 20] = [
        Self::Orchestrator,
        Self::ContextAnalyst,
        Self::RequirementsAnalyst,
        Self::Planner,
        Self::OptionsAnalyst,
        Self::Architect,
        Self::DependencyAnalyst,
        Self::Implementer,
        Self::Refactorer,
        Self::StaticAnalysisOperator,
        Self::ComplianceAuditor,
        Self::TestDesigner,
        Self::TestImplementer,
        Self::SecurityAnalyst,
        Self::PerformanceEngineer,
        Self::Integrator,
        Self::Researcher,
        Self::DataAnalyst,
        Self::Writer,
        Self::DeliveryAuthor,
    ];
    pub fn work_kind(self) -> WorkKind {
        match self {
            Self::Orchestrator => WorkKind::Orchestration,
            Self::ContextAnalyst => WorkKind::Context,
            Self::RequirementsAnalyst | Self::ComplianceAuditor => WorkKind::Requirements,
            Self::Planner => WorkKind::Planning,
            Self::OptionsAnalyst => WorkKind::Options,
            Self::Architect => WorkKind::Architecture,
            Self::DependencyAnalyst => WorkKind::Dependencies,
            Self::Implementer => WorkKind::Implementation,
            Self::Refactorer => WorkKind::Refactoring,
            Self::StaticAnalysisOperator => WorkKind::StaticAnalysis,
            Self::TestDesigner => WorkKind::TestDesign,
            Self::TestImplementer => WorkKind::Testing,
            Self::SecurityAnalyst => WorkKind::Security,
            Self::PerformanceEngineer => WorkKind::Performance,
            Self::Integrator => WorkKind::Integration,
            Self::Researcher => WorkKind::Research,
            Self::DataAnalyst => WorkKind::Analysis,
            Self::Writer => WorkKind::Writing,
            Self::DeliveryAuthor => WorkKind::Delivery,
        }
    }
    fn key(self) -> String {
        format!("{self:?}").to_ascii_lowercase()
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TemplateKind {
    SmallChange,
    Feature,
    Refactoring,
    SecurityFix,
    Greenfield,
    Research,
    Document,
    Analysis,
}
impl TemplateKind {
    pub const ALL: [Self; 8] = [
        Self::SmallChange,
        Self::Feature,
        Self::Refactoring,
        Self::SecurityFix,
        Self::Greenfield,
        Self::Research,
        Self::Document,
        Self::Analysis,
    ];
    pub fn family(self) -> TaskFamily {
        match self {
            Self::Research => TaskFamily::Research,
            Self::Document => TaskFamily::Document,
            Self::Analysis => TaskFamily::Analysis,
            _ => TaskFamily::Software,
        }
    }
    pub fn competencies(self) -> BTreeSet<CatalogRole> {
        required_roles(self.family(), self)
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::SmallChange => {
                "Local correction with the common base and all mandatory software pairs; artifacts may be concise but complete."
            }
            Self::Feature => {
                "Feature in an existing project: context, planning, implementation by module, tests and integration with independent verification."
            }
            Self::Refactoring => {
                "Structural change: context/architecture, characterization of existing behavior, refactorer-led transformations and integration with independent verification. No artificial patch is required when assessment finds no necessary transformation."
            }
            Self::SecurityFix => {
                "Security correction: threat analysis and reproduction, plan, fix, adversarial tests and integration with independent verification."
            }
            Self::Greenfield => {
                "New project: requirements, architecture, a vertical slice, incremental implementation, validation and milestones."
            }
            Self::Research => {
                "Research question: research plan, independent areas, synthesis, source verification and synthesis verification; no fictitious compilation tests for non-code research."
            }
            Self::Document => {
                "Document, specification, instructions or report: brief/outline, research when needed, writing and independent factual/editorial review."
            }
            Self::Analysis => {
                "Data or alternatives analysis: plan/method, analyst, independent recalculation and synthesis review."
            }
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStage {
    Discovery,
    Planning,
    Execution,
    Validation,
    Integration,
    Delivery,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Planned,
    Resuming,
    Running,
    AwaitingVerification,
    Verifying,
    Verified,
    ChangesRequested,
    Blocked,
    Failed,
    Cancelled,
    Stale,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationStrategy {
    ArtifactOnly,
    InPlace,
    SeparateWorktree,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EscalationPolicy {
    pub on_blocked: String,
    pub on_conflict: String,
    pub on_failure: String,
    pub completion: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRecord {
    pub id: String,
    pub role: CatalogRole,
    pub kind: RoleKind,
    pub supervisor_id: Option<String>,
    pub verifier_id: Option<String>,
    pub session_id: Option<String>,
    pub session_history: BTreeSet<String>,
    pub profile: Option<ResolvedProfile>,
    pub rationale: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRef {
    pub id: String,
    pub task_id: String,
    pub created_by: String,
    pub session_id: String,
    pub attempt: u64,
    pub spec_revision: u64,
    pub content_hash: String,
    pub source_hashes: BTreeMap<String, String>,
    #[serde(default)]
    pub dependency_hashes: BTreeMap<String, String>,
    #[serde(default)]
    pub execution_id: Option<String>,
    pub evidence_refs: BTreeSet<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewDecision {
    pub verifier_id: String,
    pub session_id: String,
    pub artifact_hash: String,
    pub spec_revision: u64,
    pub criteria: BTreeMap<String, bool>,
    pub evidence_refs: BTreeSet<String>,
    pub finding_ids: BTreeSet<String>,
    pub accepted: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepairRound {
    pub finding_ids: BTreeSet<String>,
    pub plan: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptRecord {
    pub attempt: u64,
    pub artifact: Option<ArtifactRef>,
    pub review: Option<ReviewDecision>,
    pub repair: Option<RepairRound>,
    #[serde(default)]
    pub source_refresh: Option<SourceRefresh>,
    /// A verified result whose integration into the project conflicted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub integration_conflict: Option<IntegrationConflict>,
}
/// Merge conflict of a verified task result with the current project. The next
/// attempt starts from `start`: the project state with conflict markers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntegrationConflict {
    pub paths: BTreeSet<String>,
    pub result: String,
    pub view: String,
    pub start: String,
}
impl IntegrationConflict {
    pub fn finding_ids(&self) -> BTreeSet<String> {
        self.paths
            .iter()
            .map(|path| format!("INTEGRATION:{path}"))
            .collect()
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceRefresh {
    pub approval_scope: String,
    pub target_sources: BTreeMap<String, String>,
    pub target_dependencies: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskRecord {
    pub id: String,
    pub owner_id: String,
    pub verifier_id: String,
    pub description: String,
    pub stage: TaskStage,
    pub requirement_ids: BTreeSet<String>,
    pub criterion_ids: BTreeSet<String>,
    pub plan_item_ids: BTreeSet<String>,
    pub deliverable: bool,
    pub dependencies: BTreeSet<String>,
    pub required_gates: BTreeSet<String>,
    pub source_dependencies: BTreeSet<String>,
    pub write_scopes: BTreeSet<String>,
    #[serde(default)]
    pub check_plan: Vec<checks::PlannedCheck>,
    #[serde(default)]
    pub change_contract: Option<changes::ChangeContract>,
    pub integration: IntegrationStrategy,
    pub escalation: EscalationPolicy,
    pub state: TaskState,
    pub attempt: u64,
    pub history: Vec<AttemptRecord>,
    #[serde(default)]
    pub verification_retries: Vec<VerificationRetry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_resumes: Vec<NativeResume>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub native_replacements: Vec<crate::runtime::team::replacement::Replacement>,
    pub artifact: Option<ArtifactRef>,
    pub review: Option<ReviewDecision>,
    pub repair: Option<RepairRound>,
    pub diagnostic: Option<String>,
}
/// Explicit replacement of an incomplete verifier turn; executor attempt stays pinned.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationRetry {
    pub id: String,
    pub attempt: u64,
    pub artifact_hash: String,
    pub previous_session: Option<String>,
    pub approval_scope: String,
}
/// A separately approved continuation of the same native session and attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeResume {
    pub id: String,
    pub attempt: u64,
    pub launch_attempt: u64,
    pub operation: TaskOperation,
    pub session_id: String,
    pub previous_turn_id: String,
    pub native_session_id: String,
    pub approval_scope: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TeamPlan {
    pub schema_version: u32,
    pub spec_revision: u64,
    pub spec_hash: String,
    pub family: TaskFamily,
    pub template: TemplateKind,
    pub agents: Vec<AgentRecord>,
    pub tasks: Vec<TaskRecord>,
    pub communicates_with: BTreeSet<(String, String)>,
    pub revision: u64,
    pub checkpoint_uncertain: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateBinding {
    pub spec_revision: u64,
    pub source_hashes: BTreeMap<String, String>,
    pub evidence_refs: BTreeSet<String>,
    pub passed: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GateContext {
    pub spec_revision: u64,
    pub source_hashes: BTreeMap<String, String>,
    pub gates: BTreeMap<String, GateBinding>,
    #[serde(default)]
    pub accepted_results: Option<BTreeSet<String>>,
    #[serde(default)]
    pub gate_tasks: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOperation {
    Execute,
    Verify,
}
#[derive(Debug, Clone)]
pub struct TaskInvocation {
    pub task: TaskRecord,
    pub agent: AgentRecord,
    pub operation: TaskOperation,
    pub dependency_hashes: BTreeMap<String, String>,
    pub cancelled: Arc<AtomicBool>,
}
#[derive(Debug, Clone)]
pub enum WorkerProgress {
    SessionStarted { session_id: String },
}
#[derive(Debug, Clone)]
pub enum TaskOutcome {
    Executed(ArtifactRef),
    Reviewed(ReviewDecision),
}
#[derive(Debug, Clone)]
pub enum SchedulerEvent {
    WaveStarted { task_ids: Vec<String> },
    SessionRecorded { task_id: String, agent_id: String },
    TaskFinished { task_id: String },
    Interrupted { task_ids: Vec<String> },
}
#[derive(Debug, Clone)]
pub struct CompletedTask {
    pub task_id: String,
    pub state: TaskState,
}

fn checkpoint_call<C>(callback: &mut C, team: &TeamPlan, event: &SchedulerEvent) -> Result<()>
where
    C: FnMut(&TeamPlan, &SchedulerEvent) -> Result<()>,
{
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| callback(team, event))).unwrap_or_else(
        |_| {
            Err(Error::Invalid(
                "checkpoint callback panicked; commit outcome requires reconciliation".into(),
            ))
        },
    )
}
fn text(value: &str) -> bool {
    !value.trim().is_empty() && !value.chars().any(char::is_control)
}
fn criterion_ids(spec: &RunSpec) -> BTreeSet<String> {
    spec.requirements
        .iter()
        .flat_map(|r| r.criteria.keys().map(|c| format!("REQ:{}:{c}", r.id)))
        .collect()
}
fn dag(graph: &BTreeMap<String, BTreeSet<String>>, message: &str) -> Result<()> {
    require(
        graph.values().flatten().all(|id| graph.contains_key(id)),
        "graph contains an unknown node",
    )?;
    let mut done = BTreeSet::new();
    while done.len() < graph.len() {
        let before = done.len();
        for (id, deps) in graph {
            if deps.iter().all(|dep| done.contains(dep)) {
                done.insert(id.clone());
            }
        }
        require(done.len() > before, message)?;
    }
    Ok(())
}
fn scope_parts(scope: &str) -> Result<Vec<String>> {
    require(text(scope), "write scope must be nonempty")?;
    Path::new(scope)
        .components()
        .map(|part| match part {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| Error::Invalid("non-Unicode write scope".into())),
            Component::CurDir => Ok(String::new()),
            _ => Err(Error::Invalid(
                "write scope must be a relative path without parent traversal".into(),
            )),
        })
        .filter(|result| !matches!(result,Ok(value) if value.is_empty()))
        .collect()
}
fn validate_scope_at(root: &Path, scope: &str) -> Result<()> {
    let parts = scope_parts(scope)?;
    let mut path = std::fs::canonicalize(root)?;
    require(path.is_dir(), "workspace scope root must be a directory")?;
    for (index, part) in parts.iter().enumerate() {
        path.push(part);
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        require(
            !metadata.file_type().is_symlink(),
            "write scope contains a symlink alias",
        )?;
        require(
            index + 1 == parts.len() || metadata.is_dir(),
            "write scope ancestor is not a directory",
        )?;
    }
    let mut pending = vec![path];
    while let Some(path) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&path)?;
        require(
            !metadata.file_type().is_symlink(),
            "write scope contains a symlink alias",
        )?;
        if metadata.is_dir() {
            for entry in std::fs::read_dir(path)? {
                let entry = entry?;
                if entry.file_name() != ".git" {
                    pending.push(entry.path());
                }
            }
        } else {
            require(
                metadata.is_file(),
                "write scope contains an unsupported special file",
            )?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                require(
                    metadata.nlink() == 1,
                    "write scope contains a hardlink alias",
                )?;
            }
        }
    }
    Ok(())
}
pub fn scopes_overlap(first: &BTreeSet<String>, second: &BTreeSet<String>) -> Result<bool> {
    let first = first
        .iter()
        .map(|scope| scope_parts(scope))
        .collect::<Result<Vec<_>>>()?;
    let second = second
        .iter()
        .map(|scope| scope_parts(scope))
        .collect::<Result<Vec<_>>>()?;
    for a in &first {
        for b in &second {
            if a.starts_with(b) || b.starts_with(a) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}
fn required_roles(family: TaskFamily, template: TemplateKind) -> BTreeSet<CatalogRole> {
    use CatalogRole::*;
    let mut roles = BTreeSet::from([
        Orchestrator,
        ContextAnalyst,
        RequirementsAnalyst,
        OptionsAnalyst,
        Planner,
        ComplianceAuditor,
        DeliveryAuthor,
    ]);
    if family == TaskFamily::Software {
        roles.extend([
            Architect,
            DependencyAnalyst,
            Implementer,
            Refactorer,
            StaticAnalysisOperator,
            TestDesigner,
            TestImplementer,
            Integrator,
        ]);
    }
    match family {
        TaskFamily::Research => {
            roles.extend([Researcher, Integrator]);
        }
        TaskFamily::Document => {
            roles.extend([Writer, Integrator]);
        }
        TaskFamily::Analysis => {
            roles.extend([DataAnalyst, Integrator]);
        }
        _ => {}
    }
    if template == TemplateKind::SecurityFix {
        roles.insert(SecurityAnalyst);
    }
    roles
}
pub(crate) fn execution_gates(family: TaskFamily) -> BTreeSet<String> {
    let mut gates = BTreeSet::from([
        "context_ready".into(),
        "solution_selected".into(),
        "plan_ready".into(),
    ]);
    if family == TaskFamily::Software {
        gates.extend([
            "architecture_verified:design".into(),
            "input_domain_verified:design".into(),
            "dependencies_verified:design".into(),
        ]);
    }
    gates
}
pub fn role_quality_gates(
    role: CatalogRole,
    _family: TaskFamily,
) -> Vec<(&'static str, crate::quality::GatePhase)> {
    use CatalogRole::*;
    match role {
        ContextAnalyst => vec![("context_ready", crate::quality::GatePhase::Design)],
        OptionsAnalyst => vec![("solution_selected", crate::quality::GatePhase::Design)],
        Planner => vec![("plan_ready", crate::quality::GatePhase::Design)],
        Architect => vec![
            ("architecture_verified", crate::quality::GatePhase::Design),
            ("input_domain_verified", crate::quality::GatePhase::Design),
        ],
        RequirementsAnalyst => vec![],
        DependencyAnalyst => vec![("dependencies_verified", crate::quality::GatePhase::Design)],
        Implementer => vec![
            ("scope_clean", crate::quality::GatePhase::Delivery),
            ("standards_clean", crate::quality::GatePhase::Delivery),
        ],
        Refactorer => vec![("refactor_complete", crate::quality::GatePhase::Delivery)],
        StaticAnalysisOperator => vec![("static_clean", crate::quality::GatePhase::Delivery)],
        TestImplementer => vec![("behavior_verified", crate::quality::GatePhase::Delivery)],
        Integrator => vec![
            ("integration_verified", crate::quality::GatePhase::Delivery),
            ("architecture_verified", crate::quality::GatePhase::Delivery),
            ("input_domain_verified", crate::quality::GatePhase::Delivery),
            ("dependencies_verified", crate::quality::GatePhase::Delivery),
        ],
        ComplianceAuditor => vec![("requirements_complete", crate::quality::GatePhase::Delivery)],
        DeliveryAuthor => vec![("review_complete", crate::quality::GatePhase::Delivery)],
        Researcher => vec![
            ("context_ready", crate::quality::GatePhase::Design),
            ("requirements_complete", crate::quality::GatePhase::Delivery),
        ],
        DataAnalyst => vec![
            ("context_ready", crate::quality::GatePhase::Design),
            ("requirements_complete", crate::quality::GatePhase::Delivery),
        ],
        Writer => vec![
            ("context_ready", crate::quality::GatePhase::Design),
            ("requirements_complete", crate::quality::GatePhase::Delivery),
        ],
        _ => vec![],
    }
}

impl TeamPlan {
    pub fn bootstrap(family: TaskFamily, spec: &RunSpec) -> Result<Self> {
        require(
            family == spec.task_family,
            "bootstrap family differs from the task contract",
        )?;
        let template = spec.team_template.unwrap_or(match family {
            TaskFamily::Software => TemplateKind::SmallChange,
            TaskFamily::Research => TemplateKind::Research,
            TaskFamily::Document => TemplateKind::Document,
            TaskFamily::Analysis => TemplateKind::Analysis,
        });
        Self::bootstrap_template(template, spec)
    }
    pub fn bootstrap_template(template: TemplateKind, spec: &RunSpec) -> Result<Self> {
        let family = template.family();
        require(
            family == spec.task_family
                && spec
                    .team_template
                    .is_none_or(|selected| selected == template),
            "template does not match the task family or reviewed selection",
        )?;
        let mut team = Self {
            schema_version: 1,
            spec_revision: 1,
            spec_hash: hash(serde_json::to_vec(spec)?),
            family,
            template,
            agents: vec![],
            tasks: vec![],
            communicates_with: BTreeSet::new(),
            revision: 1,
            checkpoint_uncertain: false,
        };
        let requirements = spec
            .requirements
            .iter()
            .map(|r| r.id.clone())
            .collect::<BTreeSet<_>>();
        let criteria = criterion_ids(spec);
        let primary = match (family, template) {
            (TaskFamily::Software, TemplateKind::Refactoring) => CatalogRole::Refactorer,
            (TaskFamily::Software, _) => CatalogRole::Implementer,
            (TaskFamily::Research, _) => CatalogRole::Researcher,
            (TaskFamily::Document, _) => CatalogRole::Writer,
            (TaskFamily::Analysis, _) => CatalogRole::DataAnalyst,
        };
        let roles = required_roles(family, template);
        for role in roles.iter().filter(|role| **role != primary) {
            let key = role.key();
            let (owner, verifier) = team.add_pair(
                *role,
                &key,
                format!("Required {role:?} competency for {family:?}"),
            )?;
            let stage = match role {
                CatalogRole::Orchestrator
                | CatalogRole::ContextAnalyst
                | CatalogRole::RequirementsAnalyst => TaskStage::Discovery,
                CatalogRole::Planner
                | CatalogRole::OptionsAnalyst
                | CatalogRole::Architect
                | CatalogRole::DependencyAnalyst
                | CatalogRole::TestDesigner => TaskStage::Planning,
                CatalogRole::Refactorer => TaskStage::Execution,
                CatalogRole::StaticAnalysisOperator
                | CatalogRole::TestImplementer
                | CatalogRole::SecurityAnalyst
                | CatalogRole::PerformanceEngineer => TaskStage::Validation,
                CatalogRole::Integrator => TaskStage::Integration,
                _ => TaskStage::Delivery,
            };
            team.tasks.push(TaskRecord::new(
                format!("task-{key}"),
                owner,
                verifier,
                format!("Produce and independently verify {role:?} artifacts"),
                stage,
                requirements.clone(),
                criteria.clone(),
            ));
        }
        if template == TemplateKind::SecurityFix {
            let security = team
                .tasks
                .iter_mut()
                .find(|task| task.owner_id == "executor-securityanalyst")
                .expect("security-fix competency");
            security.description = "Independently verify the actual fix against the original threat model and reproduction; assess adversarial scenarios, bypasses and regressions against current evidence. Unexecuted checks or missing observations remain explicit gaps, never a passing security result.".into();
            let mut threat = TaskRecord::new(
                "task-security-threat".into(),
                security.owner_id.clone(),
                security.verifier_id.clone(),
                "Establish the source-grounded threat model and original vulnerability reproduction before planning the fix. Distinguish observed findings from hypotheses and false positives, retain original-state evidence and define adversarial acceptance scenarios. Missing reproduction requires explicit investigation and justified alternative proof; do not invent a run or grant permissions.".into(),
                TaskStage::Discovery,
                requirements.clone(),
                criteria.clone(),
            );
            threat.dependencies.extend([
                "task-contextanalyst".into(),
                "task-requirementsanalyst".into(),
            ]);
            team.tasks.push(threat);
        }
        let plan_tasks = spec
            .plan_items
            .iter()
            .map(|p| (p.id.clone(), format!("task-plan-{}", &hash(&p.id)[..16])))
            .collect::<BTreeMap<_, _>>();
        for item in &spec.plan_items {
            let id = plan_tasks[&item.id].clone();
            let (owner, verifier) = team.add_pair(
                primary,
                &id,
                format!("Deliver original plan item {}", item.id),
            )?;
            let own_criteria = spec
                .requirements
                .iter()
                .filter(|r| item.requirement_ids.contains(&r.id))
                .flat_map(|r| r.criteria.keys().map(|c| format!("REQ:{}:{c}", r.id)))
                .collect();
            let mut task = TaskRecord::new(
                id,
                owner,
                verifier,
                item.description.clone(),
                TaskStage::Execution,
                item.requirement_ids.clone(),
                own_criteria,
            );
            task.deliverable = true;
            task.plan_item_ids.insert(item.id.clone());
            task.dependencies =
                item.dependencies
                    .iter()
                    .map(|dep| {
                        plan_tasks.get(dep).cloned().ok_or_else(|| {
                            Error::Invalid("unknown original plan dependency".into())
                        })
                    })
                    .collect::<Result<_>>()?;
            task.required_gates = execution_gates(family);
            if family == TaskFamily::Software {
                task.write_scopes = spec.editable_files.iter().cloned().collect();
                if !task.write_scopes.is_empty() {
                    task.integration = IntegrationStrategy::InPlace;
                }
            }
            team.tasks.push(task);
        }
        let discovery = team
            .tasks
            .iter()
            .filter(|t| t.stage == TaskStage::Discovery)
            .map(|t| t.id.clone())
            .collect::<BTreeSet<_>>();
        let planning = team
            .tasks
            .iter()
            .filter(|t| t.stage == TaskStage::Planning)
            .map(|t| t.id.clone())
            .collect::<BTreeSet<_>>();
        let deliverables = team
            .tasks
            .iter()
            .filter(|t| t.deliverable)
            .map(|t| t.id.clone())
            .collect::<BTreeSet<_>>();
        for task in &mut team.tasks {
            match task.stage {
                TaskStage::Planning => task.dependencies.extend(discovery.clone()),
                TaskStage::Execution => task.dependencies.extend(planning.clone()),
                TaskStage::Validation | TaskStage::Integration => {
                    task.dependencies.extend(deliverables.clone())
                }
                _ => {}
            }
            if task.owner_id == "executor-refactorer" {
                task.dependencies.extend(deliverables.clone());
            }
        }
        let planning_inputs = team
            .tasks
            .iter()
            .filter(|t| {
                [
                    "executor-optionsanalyst",
                    "executor-architect",
                    "executor-dependencyanalyst",
                    "executor-testdesigner",
                ]
                .contains(&t.owner_id.as_str())
            })
            .map(|t| t.id.clone())
            .collect::<BTreeSet<_>>();
        if let Some(planner) = team
            .tasks
            .iter_mut()
            .find(|t| t.owner_id == "executor-planner")
        {
            planner.dependencies.extend(planning_inputs);
        }
        if let Some(delivery) = team
            .tasks
            .iter_mut()
            .find(|t| t.owner_id == "executor-deliveryauthor")
        {
            delivery.required_gates.extend([
                "requirements_complete".into(),
                "integration_verified".into(),
            ]);
        }
        let refactoring = team
            .tasks
            .iter()
            .filter(|t| t.owner_id == "executor-refactorer")
            .map(|t| t.id.clone())
            .collect::<BTreeSet<_>>();
        let validation = team
            .tasks
            .iter()
            .filter(|t| t.stage == TaskStage::Validation)
            .map(|t| t.id.clone())
            .collect::<BTreeSet<_>>();
        for task in &mut team.tasks {
            if task.stage == TaskStage::Validation {
                task.dependencies.extend(refactoring.clone());
            }
            if task.stage == TaskStage::Integration {
                task.dependencies.extend(validation.clone());
                task.dependencies.extend(refactoring.clone());
            }
        }
        let integrated = team
            .tasks
            .iter()
            .filter(|t| {
                matches!(t.stage, TaskStage::Validation | TaskStage::Integration)
                    || t.owner_id == "executor-refactorer"
            })
            .map(|t| t.id.clone())
            .collect::<BTreeSet<_>>();
        for task in team
            .tasks
            .iter_mut()
            .filter(|t| t.stage == TaskStage::Delivery)
        {
            task.dependencies.extend(integrated.clone());
            task.dependencies.extend(deliverables.clone());
            if task.owner_id == "executor-deliveryauthor" {
                task.dependencies.insert("task-complianceauditor".into());
            }
        }
        for task in &mut team.tasks {
            if team
                .agents
                .iter()
                .find(|agent| agent.id == task.owner_id)
                .is_some_and(|agent| {
                    matches!(
                        agent.role,
                        CatalogRole::Implementer | CatalogRole::Refactorer
                    )
                })
            {
                task.required_gates
                    .extend(execution_gates(TaskFamily::Software));
            }
            task.source_dependencies
                .extend(spec.input_files.iter().map(|path| format!("file:{path}")));
            task.source_dependencies
                .extend(task.plan_item_ids.iter().map(|id| format!("plan:{id}")));
        }
        for task in &mut team.tasks {
            let role = team
                .agents
                .iter()
                .find(|a| a.id == task.owner_id)
                .unwrap()
                .role;
            let gates = checks::runner_gates(role, family);
            if template == TemplateKind::Refactoring
                && task.deliverable
                && !spec.check_command.is_empty()
            {
                task.check_plan.push(checks::PlannedCheck {
                    id:"characterization-baseline".into(), argv:spec.check_command.clone(),
                    phase:checks::CheckPhase::BeforeExecution,expected_exit_code:0,
                    criterion_ids:task.criterion_ids.clone(),gate_ids:BTreeSet::new(),
                    rationale:"Observe existing behavior before refactoring with the configured characterization check. This is a historical baseline, not completion of assigned criteria or a successful current gate. Independent review must establish actual existing-contract coverage and compare the post-change result; process exit zero alone is insufficient.".into(),
                });
            }
            if !spec.check_command.is_empty() && !gates.is_empty() {
                task.check_plan.push(checks::PlannedCheck { id: "configured-check".into(), argv: spec.check_command.clone(), phase: checks::CheckPhase::BeforeVerification, expected_exit_code: 0,
                    criterion_ids: task.criterion_ids.clone(), gate_ids: gates,
                    rationale: "Configured local check; independent reviewer must confirm its actual coverage".into() });
            }
        }
        team.validate(spec)?;
        Ok(team)
    }
    /// AI expansion uses this same checked representation; no numeric limits.
    pub fn add_pair(
        &mut self,
        role: CatalogRole,
        key: &str,
        rationale: String,
    ) -> Result<(String, String)> {
        require(
            text(key) && text(&rationale),
            "agent pair requires identity and responsibility",
        )?;
        let owner = format!("executor-{key}");
        let verifier = format!("verifier-{key}");
        require(
            !self
                .agents
                .iter()
                .any(|a| a.id == owner || a.id == verifier),
            "agent identity already exists",
        )?;
        let supervisor = if role == CatalogRole::Orchestrator {
            None
        } else {
            Some("executor-orchestrator".into())
        };
        self.agents.push(AgentRecord {
            id: owner.clone(),
            role,
            kind: if role == CatalogRole::Orchestrator {
                RoleKind::Orchestrator
            } else {
                RoleKind::Executor
            },
            supervisor_id: supervisor.clone(),
            verifier_id: Some(verifier.clone()),
            session_id: None,
            session_history: BTreeSet::new(),
            profile: None,
            rationale: rationale.clone(),
        });
        self.agents.push(AgentRecord {
            id: verifier.clone(),
            role,
            kind: RoleKind::Verifier,
            supervisor_id: supervisor,
            verifier_id: None,
            session_id: None,
            session_history: BTreeSet::new(),
            profile: None,
            rationale: format!("Independently verify: {rationale}"),
        });
        Ok((owner, verifier))
    }
    pub fn validate(&self, spec: &RunSpec) -> Result<()> {
        spec.validate()?;
        require(
            self.schema_version == 1 && self.spec_revision > 0 && self.revision > 0,
            "invalid team revision",
        )?;
        require(
            self.family == spec.task_family
                && self.template.family() == self.family
                && spec
                    .team_template
                    .is_none_or(|selected| selected == self.template),
            "team family or reviewed template selection mismatch",
        )?;
        require(
            self.spec_hash == hash(serde_json::to_vec(spec)?),
            "team source specification changed without a new plan",
        )?;
        let agents = self
            .agents
            .iter()
            .map(|a| (a.id.clone(), a))
            .collect::<BTreeMap<_, _>>();
        require(
            agents.len() == self.agents.len() && !agents.is_empty(),
            "duplicate or empty team agents",
        )?;
        let mut verifiers = BTreeSet::new();
        let mut sessions = BTreeSet::new();
        for agent in &self.agents {
            require(
                text(&agent.id) && text(&agent.rationale),
                "agent identity and responsibility required",
            )?;
            for session in &agent.session_history {
                require(
                    text(session) && sessions.insert(session.clone()),
                    "agents must not share historical session identity",
                )?;
            }
            if let Some(session) = &agent.session_id {
                require(
                    text(session) && sessions.insert(session.clone()),
                    "agents must not share session identity",
                )?;
            }
            if agent.kind != RoleKind::Verifier {
                let verifier = agent
                    .verifier_id
                    .as_ref()
                    .and_then(|id| agents.get(id))
                    .ok_or_else(|| Error::Invalid("executor has no dedicated verifier".into()))?;
                require(
                    verifier.kind == RoleKind::Verifier
                        && verifier.role == agent.role
                        && verifiers.insert(verifier.id.clone()),
                    "verifier must be dedicated to exactly one matching executor role",
                )?;
                require(
                    (agent.kind == RoleKind::Orchestrator)
                        == (agent.role == CatalogRole::Orchestrator),
                    "orchestrator catalog and execution identity must match",
                )?;
            } else {
                require(
                    agent.verifier_id.is_none(),
                    "verifiers do not recursively verify other verifiers",
                )?;
            }
            if let Some(profile) = &agent.profile {
                require(
                    profile.role == agent.kind && profile.work_kind == agent.role.work_kind(),
                    "profile does not match team role",
                )?;
            }
        }
        require(
            self.agents
                .iter()
                .filter(|agent| agent.kind == RoleKind::Orchestrator)
                .count()
                == 1,
            "team requires exactly one explicit orchestrator",
        )?;
        require(
            verifiers.len()
                == self
                    .agents
                    .iter()
                    .filter(|a| a.kind == RoleKind::Verifier)
                    .count(),
            "orphan verifier has no executor",
        )?;
        let delegation = self
            .agents
            .iter()
            .map(|a| (a.id.clone(), a.supervisor_id.clone().into_iter().collect()))
            .collect();
        dag(&delegation, "delegation cycle")?;
        for (from, to) in &self.communicates_with {
            require(
                agents.contains_key(from) && agents.contains_key(to),
                "communication references unknown agent",
            )?;
        }
        let actual_roles = self
            .agents
            .iter()
            .filter(|a| a.kind != RoleKind::Verifier)
            .map(|a| a.role)
            .collect::<BTreeSet<_>>();
        require(
            required_roles(self.family, self.template).is_subset(&actual_roles),
            "team omits a mandatory competency",
        )?;
        let tasks = self
            .tasks
            .iter()
            .map(|t| (t.id.clone(), t))
            .collect::<BTreeMap<_, _>>();
        require(
            !tasks.is_empty() && tasks.len() == self.tasks.len(),
            "duplicate or empty tasks",
        )?;
        let required_requirements = spec
            .requirements
            .iter()
            .map(|r| r.id.clone())
            .collect::<BTreeSet<_>>();
        let required_criteria = criterion_ids(spec);
        let required_plans = spec
            .plan_items
            .iter()
            .map(|p| p.id.clone())
            .collect::<BTreeSet<_>>();
        let allowed_sources = std::iter::once("objective".to_owned())
            .chain(
                required_requirements
                    .iter()
                    .map(|id| format!("requirement:{id}")),
            )
            .chain(required_plans.iter().map(|id| format!("plan:{id}")))
            .chain(spec.input_files.iter().map(|path| format!("file:{path}")))
            .collect::<BTreeSet<_>>();
        let mut requirements = BTreeSet::new();
        let mut criteria = BTreeSet::new();
        let mut plans = BTreeSet::new();
        let mut owners = BTreeSet::new();
        for task in &self.tasks {
            require(
                text(&task.id) && text(&task.description) && !task.criterion_ids.is_empty(),
                "task needs identity, result and acceptance criteria",
            )?;
            let owner = agents
                .get(&task.owner_id)
                .ok_or_else(|| Error::Invalid("task has unknown owner".into()))?;
            require(
                owner.kind != RoleKind::Verifier
                    && owner.verifier_id.as_ref() == Some(&task.verifier_id),
                "task owner/verifier pair mismatch",
            )?;
            owners.insert(owner.id.clone());
            require(
                !task.requirement_ids.is_empty()
                    && task.requirement_ids.is_subset(&required_requirements)
                    && task.criterion_ids.is_subset(&required_criteria)
                    && task.plan_item_ids.is_subset(&required_plans),
                "task contains absent or unknown requirements/criteria/plan items",
            )?;
            let own_criteria = spec
                .requirements
                .iter()
                .filter(|r| task.requirement_ids.contains(&r.id))
                .flat_map(|r| r.criteria.keys().map(|c| format!("REQ:{}:{c}", r.id)))
                .collect::<BTreeSet<_>>();
            require(
                task.criterion_ids.is_subset(&own_criteria),
                "task criterion belongs to an unassigned requirement",
            )?;
            let mandatory_sources = std::iter::once("objective".to_owned())
                .chain(
                    task.requirement_ids
                        .iter()
                        .map(|id| format!("requirement:{id}")),
                )
                .chain(task.plan_item_ids.iter().map(|id| format!("plan:{id}")))
                .collect::<BTreeSet<_>>();
            require(
                mandatory_sources.is_subset(&task.source_dependencies)
                    && task.source_dependencies.is_subset(&allowed_sources),
                "task source dependency coverage invalid",
            )?;
            if task.deliverable {
                requirements.extend(task.requirement_ids.clone());
                criteria.extend(task.criterion_ids.clone());
                plans.extend(task.plan_item_ids.clone());
            }
            require(
                [
                    &task.escalation.on_blocked,
                    &task.escalation.on_conflict,
                    &task.escalation.on_failure,
                    &task.escalation.completion,
                ]
                .iter()
                .all(|v| text(v)),
                "task escalation and completion conditions required",
            )?;
            for scope in &task.write_scopes {
                validate_scope_at(Path::new(&spec.workspace), scope)?;
            }
            require(
                task.write_scopes.is_empty()
                    || task.integration != IntegrationStrategy::ArtifactOnly,
                "write task requires an integration strategy",
            )?;
            let code_role = matches!(
                owner.role,
                CatalogRole::Implementer | CatalogRole::Refactorer
            );
            let solution_deliverable = task.deliverable && task.stage >= TaskStage::Execution;
            if solution_deliverable || code_role || !task.write_scopes.is_empty() {
                let family = if code_role {
                    TaskFamily::Software
                } else {
                    self.family
                };
                let missing = execution_gates(family)
                    .difference(&task.required_gates)
                    .cloned()
                    .collect::<Vec<_>>();
                require(
                    missing.is_empty(),
                    &format!(
                        "implementation or solution-writing task {} ({:?}, {:?}) lacks current implementation gates: {}",
                        task.id,
                        owner.role,
                        task.stage,
                        missing.join(", ")
                    ),
                )?;
            }
            checks::validate_plan(task, false, owner.role, self.family)?;
            changes::validate_plan(task, false, owner.role, self.family)?;
            self.validate_history(task)?;
            if let Some(artifact) = &task.artifact {
                self.validate_artifact(task, artifact)?;
            }
            match task.state {
                TaskState::Resuming => require(
                    task.artifact.is_none()
                        && task.review.is_none()
                        && (task.native_resumes.last().is_some_and(|r| {
                            r.operation == TaskOperation::Execute && r.attempt == task.attempt
                        }) || task.native_replacements.last().is_some_and(|r| {
                            r.operation == TaskOperation::Execute && r.attempt == task.attempt
                        })),
                    "executor continuation needs its scoped native checkpoint",
                )?,
                TaskState::Planned => require(
                    task.artifact.is_none()
                        && task.review.is_none()
                        && task.attempt == task.history.len() as u64
                        && if task.attempt == 0 {
                            task.repair.is_none()
                        } else {
                            task.history.last().is_some_and(|history| {
                                history.repair.as_ref() == task.repair.as_ref()
                                    && task.repair.is_some()
                                    || (history.source_refresh.is_some() && task.repair.is_none())
                            })
                        },
                    "planned attempt cannot reset or replay a prior effect without a repair round",
                )?,
                TaskState::AwaitingVerification | TaskState::Verifying => require(
                    task.artifact.is_some(),
                    "verification requires a pinned executor artifact",
                )?,
                TaskState::Verified => {
                    require(
                        task.artifact.is_some() && task.review.as_ref().is_some_and(|r| r.accepted),
                        "verified task requires artifact and accepted review",
                    )?;
                    self.validate_review(task, task.review.as_ref().unwrap())?;
                }
                TaskState::ChangesRequested => {
                    require(
                        task.review
                            .as_ref()
                            .is_some_and(|r| !r.accepted && !r.finding_ids.is_empty()),
                        "repair state requires unresolved findings",
                    )?;
                    self.validate_review(task, task.review.as_ref().unwrap())?;
                }
                _ => {}
            }
        }
        require(
            requirements == required_requirements
                && criteria == required_criteria
                && plans == required_plans,
            &format!(
                "deliverable requirement, criterion or original-plan coverage incomplete; missing requirements {:?}; missing criteria {:?}; missing plan items {:?}",
                required_requirements
                    .difference(&requirements)
                    .collect::<Vec<_>>(),
                required_criteria.difference(&criteria).collect::<Vec<_>>(),
                required_plans.difference(&plans).collect::<Vec<_>>()
            ),
        )?;
        require(
            owners.len()
                == self
                    .agents
                    .iter()
                    .filter(|a| a.kind != RoleKind::Verifier)
                    .count(),
            "every executor agent requires an assigned task",
        )?;
        let dependencies = self
            .tasks
            .iter()
            .map(|t| (t.id.clone(), t.dependencies.clone()))
            .collect();
        dag(&dependencies, "task dependency cycle")?;
        for task in self
            .tasks
            .iter()
            .filter(|task| task.deliverable && !task.plan_item_ids.is_empty())
        {
            let mapped = spec
                .plan_items
                .iter()
                .filter(|item| task.plan_item_ids.contains(&item.id))
                .flat_map(|item| item.requirement_ids.iter().cloned())
                .collect::<BTreeSet<_>>();
            require(
                task.requirement_ids.is_subset(&mapped),
                "task changed the original plan requirement assignment",
            )?;
        }
        for item in &spec.plan_items {
            let mapped_tasks = self
                .tasks
                .iter()
                .filter(|task| task.deliverable && task.plan_item_ids.contains(&item.id))
                .collect::<Vec<_>>();
            let mapped_requirements = mapped_tasks
                .iter()
                .flat_map(|task| task.requirement_ids.iter().cloned())
                .collect::<BTreeSet<_>>();
            let mapped_criteria = mapped_tasks
                .iter()
                .flat_map(|task| task.criterion_ids.iter().cloned())
                .collect::<BTreeSet<_>>();
            let expected_criteria = spec
                .requirements
                .iter()
                .filter(|requirement| item.requirement_ids.contains(&requirement.id))
                .flat_map(|requirement| {
                    requirement
                        .criteria
                        .keys()
                        .map(|criterion| format!("REQ:{}:{criterion}", requirement.id))
                })
                .collect::<BTreeSet<_>>();
            require(
                item.requirement_ids.is_subset(&mapped_requirements)
                    && expected_criteria.is_subset(&mapped_criteria),
                &format!(
                    "original plan item {} requirement or criterion coverage incomplete; missing requirements {:?}; missing criteria {:?}",
                    item.id,
                    item.requirement_ids
                        .difference(&mapped_requirements)
                        .collect::<Vec<_>>(),
                    expected_criteria
                        .difference(&mapped_criteria)
                        .collect::<Vec<_>>()
                ),
            )?;
        }
        // Original user-plan dependencies must survive expansion, even when
        // supporting analysis tasks also reference the same requirements.
        for item in &spec.plan_items {
            for dependency in &item.dependencies {
                let preceding = self
                    .tasks
                    .iter()
                    .filter(|t| t.deliverable && t.plan_item_ids.contains(dependency))
                    .map(|t| t.id.clone())
                    .collect::<BTreeSet<_>>();
                require(!preceding.is_empty(), "original plan dependency omitted")?;
                for task in self
                    .tasks
                    .iter()
                    .filter(|t| t.deliverable && t.plan_item_ids.contains(&item.id))
                {
                    require(
                        preceding.is_subset(&task.dependencies),
                        "original plan dependency was silently weakened",
                    )?;
                }
            }
        }
        Ok(())
    }
    pub fn validate_all(&mut self, spec: &RunSpec, config: &Config) -> Result<()> {
        self.validate(spec)?;
        if self.tasks.iter().any(|task| !task.check_plan.is_empty()) {
            config.policy.authorize_project_check(true)?;
        }
        let mut candidate = self.clone();
        for agent in &mut candidate.agents {
            agent.profile = Some(config.resolve(agent.role.work_kind(), agent.kind)?);
        }
        candidate.validate(spec)?;
        *self = candidate;
        Ok(())
    }
    fn validate_history(&self, task: &TaskRecord) -> Result<()> {
        let mut replacement_ids = BTreeSet::new();
        for replacement in &task.native_replacements {
            require(
                text(&replacement.id)
                    && replacement_ids.insert(&replacement.id)
                    && replacement.attempt > 0
                    && replacement.attempt <= task.attempt
                    && replacement.launch_attempt > 0
                    && text(&replacement.new_session_id)
                    && replacement.previous_session.as_ref() != Some(&replacement.new_session_id)
                    && replacement.handoff.version
                        == crate::runtime::team::replacement::HANDOFF_VERSION
                    && replacement.handoff.task_id == task.id
                    && replacement.handoff.task_attempt == replacement.attempt
                    && replacement.handoff.hash()? == replacement.handoff_hash
                    && replacement.approval_scope.len() == 64
                    && replacement
                        .approval_scope
                        .bytes()
                        .all(|b| b.is_ascii_hexdigit()),
                "invalid replacement session/handoff history",
            )?;
        }
        let mut resume_ids = BTreeSet::new();
        for resume in &task.native_resumes {
            require(
                text(&resume.id)
                    && resume_ids.insert(&resume.id)
                    && resume.attempt > 0
                    && resume.attempt <= task.attempt
                    && resume.launch_attempt > 0
                    && text(&resume.session_id)
                    && text(&resume.previous_turn_id)
                    && uuid::Uuid::parse_str(&resume.native_session_id).is_ok()
                    && resume.approval_scope.len() == 64
                    && resume.approval_scope.bytes().all(|b| b.is_ascii_hexdigit()),
                "invalid native continuation history",
            )?;
        }
        let mut retry_ids = BTreeSet::new();
        let mut last_attempt = 0;
        for retry in &task.verification_retries {
            let artifact = task
                .artifact
                .as_ref()
                .filter(|a| a.attempt == retry.attempt)
                .or_else(|| {
                    task.history
                        .iter()
                        .find(|h| h.attempt == retry.attempt)
                        .and_then(|h| h.artifact.as_ref())
                });
            require(
                text(&retry.id)
                    && retry_ids.insert(&retry.id)
                    && retry.attempt >= last_attempt
                    && artifact.is_some_and(|a| a.content_hash == retry.artifact_hash)
                    && retry.approval_scope.len() == 64
                    && retry.approval_scope.bytes().all(|b| b.is_ascii_hexdigit())
                    && retry.previous_session.as_ref().is_none_or(|session| {
                        self.agents
                            .iter()
                            .find(|a| a.id == task.verifier_id)
                            .is_some_and(|a| {
                                a.session_id.as_ref() == Some(session)
                                    || a.session_history.contains(session)
                            })
                    }),
                "invalid verification replacement history",
            )?;
            last_attempt = retry.attempt;
        }
        require(
            task.attempt >= task.history.len() as u64
                && task.attempt <= task.history.len() as u64 + 1,
            "attempt history continuity invalid",
        )?;
        for (index, historical) in task.history.iter().enumerate() {
            require(
                historical.attempt == index as u64 + 1,
                "attempt history continuity invalid",
            )?;
            let artifact = historical
                .artifact
                .as_ref()
                .ok_or_else(|| Error::Invalid("repair history lacks its pinned artifact".into()))?;
            if let Some(conflict) = &historical.integration_conflict {
                require(
                    historical.source_refresh.is_none()
                        && historical.review.as_ref().is_some_and(|r| r.accepted)
                        && !conflict.paths.is_empty()
                        && historical.repair.as_ref().is_some_and(|r| {
                            r.finding_ids == conflict.finding_ids() && text(&r.plan)
                        }),
                    "integration repair must preserve an accepted result and name its conflicts",
                )?;
            } else if let Some(refresh) = &historical.source_refresh {
                require(
                    historical.repair.is_none()
                        && historical.review.as_ref().is_none_or(|r| r.accepted)
                        && refresh.approval_scope.len() == 64
                        && refresh
                            .approval_scope
                            .bytes()
                            .all(|b| b.is_ascii_hexdigit())
                        && (refresh.target_sources != artifact.source_hashes
                            || refresh.target_dependencies != artifact.dependency_hashes),
                    "source refresh must preserve a nonrejected artifact and bind changed inputs/dependencies",
                )?;
            } else {
                let review = historical.review.as_ref().ok_or_else(|| {
                    Error::Invalid("repair history lacks independent rejection".into())
                })?;
                let repair = historical
                    .repair
                    .as_ref()
                    .ok_or_else(|| Error::Invalid("repair history lacks a repair plan".into()))?;
                require(
                    !review.accepted
                        && !review.finding_ids.is_empty()
                        && review.finding_ids == repair.finding_ids
                        && text(&repair.plan),
                    "repair must derive from all findings of a durable rejected review",
                )?;
            }
            let mut prior = task.clone();
            prior.attempt = historical.attempt;
            prior.artifact = Some(artifact.clone());
            self.validate_artifact(&prior, artifact)?;
            if let Some(review) = &historical.review {
                self.validate_review(&prior, review)?;
            }
        }
        if matches!(
            task.state,
            TaskState::Running
                | TaskState::Resuming
                | TaskState::AwaitingVerification
                | TaskState::Verifying
                | TaskState::Verified
                | TaskState::ChangesRequested
        ) {
            require(
                task.attempt == task.history.len() as u64 + 1,
                "active attempt must follow its durable history",
            )?;
        }
        Ok(())
    }
    fn verified_current(
        &self,
        id: &str,
        context: &GateContext,
        visiting: &mut BTreeSet<String>,
        cache: &mut BTreeMap<String, bool>,
    ) -> bool {
        if let Some(valid) = cache.get(id) {
            return *valid;
        }
        if !visiting.insert(id.to_owned()) {
            return false;
        }
        let valid = self
            .tasks
            .iter()
            .find(|task| task.id == id)
            .is_some_and(|task| {
                task.state == TaskState::Verified
                    && task.artifact.as_ref().is_some_and(|artifact| {
                        artifact.spec_revision == context.spec_revision
                            && artifact.source_hashes == context.source_hashes
                            && self
                                .dependency_bindings(task)
                                .is_ok_and(|bindings| bindings == artifact.dependency_hashes)
                    })
                    && task.dependencies.iter().all(|dependency| {
                        self.verified_current(dependency, context, visiting, cache)
                    })
            });
        visiting.remove(id);
        cache.insert(id.to_owned(), valid);
        valid
    }
    /// A model proposal may describe a plan, never manufacture runtime history.
    pub fn validate_fresh_proposal(&self, spec: &RunSpec) -> Result<()> {
        self.validate(spec)?;
        require(
            !self.checkpoint_uncertain
                && self.agents.iter().all(|agent| {
                    agent.session_id.is_none()
                        && agent.session_history.is_empty()
                        && agent.profile.is_none()
                })
                && self.tasks.iter().all(|task| {
                    task.state == TaskState::Planned
                        && task.attempt == 0
                        && task.history.is_empty()
                        && task.verification_retries.is_empty()
                        && task.native_resumes.is_empty()
                        && task.native_replacements.is_empty()
                        && task.artifact.is_none()
                        && task.review.is_none()
                        && task.repair.is_none()
                }),
            "a team proposal cannot assert sessions, attempts, artifacts or reviews",
        )
    }
    fn validate_artifact(&self, task: &TaskRecord, artifact: &ArtifactRef) -> Result<()> {
        require(
            text(&artifact.id)
                && artifact.task_id == task.id
                && artifact.created_by == task.owner_id
                && artifact.attempt == task.attempt
                && artifact.spec_revision == self.spec_revision
                && artifact.content_hash.len() == 64
                && artifact.content_hash.bytes().all(|b| b.is_ascii_hexdigit())
                && self
                    .agents
                    .iter()
                    .find(|a| a.id == task.owner_id)
                    .is_some_and(|a| {
                        a.session_id.as_ref() == Some(&artifact.session_id)
                            || a.session_history.contains(&artifact.session_id)
                    })
                && !artifact.evidence_refs.is_empty()
                && artifact.evidence_refs.iter().all(|id| text(id)),
            "artifact identity, attempt, revision or evidence mismatch",
        )
    }
    fn validate_review(&self, task: &TaskRecord, review: &ReviewDecision) -> Result<()> {
        let artifact = task
            .artifact
            .as_ref()
            .ok_or_else(|| Error::Invalid("review has no pinned executor artifact".into()))?;
        require(
            review.verifier_id == task.verifier_id
                && review.session_id != artifact.session_id
                && self
                    .agents
                    .iter()
                    .find(|a| a.id == task.verifier_id)
                    .is_some_and(|a| {
                        a.session_id.as_ref() == Some(&review.session_id)
                            || a.session_history.contains(&review.session_id)
                    })
                && review.artifact_hash == artifact.content_hash
                && review.spec_revision == self.spec_revision
                && !review.evidence_refs.is_empty()
                && review.evidence_refs.iter().all(|id| text(id)),
            "review author, artifact revision or evidence mismatch",
        )?;
        require(
            review.criteria.keys().cloned().collect::<BTreeSet<_>>() == task.criterion_ids,
            "review criterion coverage incomplete",
        )?;
        require(
            !review.accepted
                || (review.criteria.values().all(|pass| *pass) && review.finding_ids.is_empty()),
            "accepted review contains a failed criterion or unresolved finding",
        )?;
        require(
            review.accepted || !review.finding_ids.is_empty(),
            "rejected review requires actionable findings",
        )
    }
    pub fn ready_wave(&self, spec: &RunSpec, context: &GateContext) -> Result<Vec<TaskInvocation>> {
        self.ready_wave_for(spec, context, None)
    }
    pub fn dependency_bindings(&self, task: &TaskRecord) -> Result<BTreeMap<String, String>> {
        task.dependencies
            .iter()
            .map(|id| {
                let dependency = self
                    .tasks
                    .iter()
                    .find(|t| &t.id == id)
                    .ok_or_else(|| Error::Invalid("unknown dependency".into()))?;
                let artifact = dependency
                    .artifact
                    .as_ref()
                    .ok_or_else(|| Error::Invalid("dependency artifact missing".into()))?;
                let review = dependency
                    .review
                    .as_ref()
                    .filter(|r| r.accepted)
                    .ok_or_else(|| {
                        Error::Invalid("dependency independent acceptance missing".into())
                    })?;
                Ok((id.clone(), hash(serde_json::to_vec(&(artifact, review))?)))
            })
            .collect()
    }
    pub fn invalidate_sources(&mut self, context: &GateContext) -> Vec<String> {
        let mut stale = vec![];
        let mut cache = BTreeMap::new();
        for task in &self.tasks {
            let invalid = match task.state {
                TaskState::Verified => {
                    !self.verified_current(&task.id, context, &mut BTreeSet::new(), &mut cache)
                }
                TaskState::AwaitingVerification => task.artifact.as_ref().is_some_and(|a| {
                    a.source_hashes != context.source_hashes
                        || !self
                            .dependency_bindings(task)
                            .is_ok_and(|b| b == a.dependency_hashes)
                }),
                _ => false,
            };
            if invalid {
                stale.push(task.id.clone());
            }
        }
        for task in self.tasks.iter_mut().filter(|t| stale.contains(&t.id)) {
            task.state = TaskState::Stale;
            task.diagnostic = Some("Sources or reviewed dependencies changed; explicit read-only source refresh required".into());
        }
        stale
    }
    pub fn prepare_source_refresh(&mut self, task_id: &str, refresh: SourceRefresh) -> Result<()> {
        let prior = self
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .ok_or_else(|| Error::Invalid("unknown task".into()))?;
        require(
            prior.state == TaskState::Stale
                && prior.artifact.is_some()
                && prior.review.as_ref().is_none_or(|r| r.accepted),
            "source refresh needs a stale nonrejected result",
        )?;
        require(
            refresh.target_dependencies == self.dependency_bindings(prior)?,
            "source refresh dependency binding mismatch",
        )?;
        let task = self.tasks.iter_mut().find(|t| t.id == task_id).unwrap();
        task.history.push(AttemptRecord {
            attempt: task.attempt,
            artifact: task.artifact.take(),
            review: task.review.take(),
            repair: None,
            source_refresh: Some(refresh),
            integration_conflict: None,
        });
        task.repair = None;
        task.state = TaskState::Planned;
        task.diagnostic = None;
        self.validate_history(self.tasks.iter().find(|t| t.id == task_id).unwrap())
    }
    pub(crate) fn ready_wave_for(
        &self,
        spec: &RunSpec,
        context: &GateContext,
        selected_task: Option<&str>,
    ) -> Result<Vec<TaskInvocation>> {
        self.validate(spec)?;
        require(
            !self.checkpoint_uncertain,
            "scheduler checkpoint outcome requires reconciliation",
        )?;
        require(
            context.spec_revision == self.spec_revision,
            "scheduler context revision mismatch",
        )?;
        require(
            self.tasks
                .iter()
                .flat_map(|task| &task.source_dependencies)
                .all(|source| context.source_hashes.contains_key(source)),
            "scheduler context omits a pinned task source",
        )?;
        let tasks = self
            .tasks
            .iter()
            .map(|t| (t.id.as_str(), t))
            .collect::<BTreeMap<_, _>>();
        let mut freshness = BTreeMap::new();
        require(
            self.tasks
                .iter()
                .filter(|task| task.state == TaskState::Verified)
                .all(|task| {
                    self.verified_current(&task.id, context, &mut BTreeSet::new(), &mut freshness)
                }),
            "verified prerequisite has stale sources or dependencies; revalidate before scheduling",
        )?;
        let mut selected = vec![];
        let mut scopes = vec![];
        let mut busy = BTreeSet::new();
        for task in self.tasks.iter().filter(|t| {
            matches!(
                t.state,
                TaskState::Running | TaskState::Verifying | TaskState::Blocked
            )
        }) {
            busy.insert(if task.state == TaskState::Verifying {
                task.verifier_id.clone()
            } else {
                task.owner_id.clone()
            });
            if task.state == TaskState::Blocked {
                busy.insert(task.verifier_id.clone());
            }
            if matches!(task.state, TaskState::Running | TaskState::Blocked) {
                scopes.push(task.write_scopes.clone());
            }
        }
        for task in self
            .tasks
            .iter()
            .filter(|t| selected_task.is_none_or(|id| t.id == id))
        {
            let operation = match task.state {
                TaskState::Planned | TaskState::Resuming => TaskOperation::Execute,
                TaskState::AwaitingVerification => TaskOperation::Verify,
                _ => continue,
            };
            if !task.dependencies.iter().all(|id| {
                tasks.get(id.as_str()).is_some_and(|task| {
                    self.verified_current(&task.id, context, &mut BTreeSet::new(), &mut freshness)
                })
            }) {
                continue;
            }
            if let Some(accepted) = &context.accepted_results {
                let approved_dependency = |id: &String| {
                    tasks
                        .get(id.as_str())
                        .is_some_and(|parent| parent.stage == task.stage || accepted.contains(id))
                };
                if !task.dependencies.iter().all(approved_dependency)
                    || (operation == TaskOperation::Execute
                        && task.required_gates.iter().any(|gate| {
                            context
                                .gate_tasks
                                .get(gate)
                                .is_some_and(|id| !approved_dependency(id))
                        }))
                {
                    continue;
                }
            }
            if operation == TaskOperation::Execute
                && !task.required_gates.iter().all(|id| {
                    context.gates.get(id).is_some_and(|gate| {
                        gate.passed
                            && gate.spec_revision == context.spec_revision
                            && gate.source_hashes == context.source_hashes
                            && !gate.evidence_refs.is_empty()
                    })
                })
            {
                continue;
            }
            if operation == TaskOperation::Verify
                && task
                    .artifact
                    .as_ref()
                    .is_none_or(|artifact| artifact.source_hashes != context.source_hashes)
            {
                continue;
            }
            let agent_id = if operation == TaskOperation::Verify {
                &task.verifier_id
            } else {
                &task.owner_id
            };
            if busy.contains(agent_id) {
                continue;
            }
            let writes = if operation == TaskOperation::Execute {
                task.write_scopes.clone()
            } else {
                BTreeSet::new()
            };
            if scopes
                .iter()
                .any(|active| scopes_overlap(&writes, active).unwrap_or(true))
            {
                continue;
            }
            let agent = self
                .agents
                .iter()
                .find(|a| &a.id == agent_id)
                .unwrap()
                .clone();
            busy.insert(agent_id.clone());
            scopes.push(writes);
            selected.push(TaskInvocation {
                task: task.clone(),
                agent,
                operation,
                dependency_hashes: self.dependency_bindings(task)?,
                cancelled: Arc::new(AtomicBool::new(false)),
            });
        }
        Ok(selected)
    }
    pub fn request_repair(&mut self, task_id: &str, repair: RepairRound) -> Result<()> {
        require(
            !repair.finding_ids.is_empty() && text(&repair.plan),
            "repair round needs findings and a concrete plan",
        )?;
        let prior = self
            .tasks
            .iter()
            .find(|task| task.id == task_id)
            .ok_or_else(|| Error::Invalid("unknown task".into()))?;
        self.validate_history(prior)?;
        self.validate_review(
            prior,
            prior
                .review
                .as_ref()
                .ok_or_else(|| Error::Invalid("repair has no independent rejection".into()))?,
        )?;
        let task = self
            .tasks
            .iter_mut()
            .find(|t| t.id == task_id)
            .ok_or_else(|| Error::Invalid("unknown task".into()))?;
        require(
            task.state == TaskState::ChangesRequested,
            "repair requires changes_requested state",
        )?;
        require(
            task.review
                .as_ref()
                .is_some_and(|review| review.finding_ids == repair.finding_ids),
            "repair does not account for all unresolved findings",
        )?;
        task.history.push(AttemptRecord {
            attempt: task.attempt,
            artifact: task.artifact.take(),
            review: task.review.take(),
            repair: Some(repair.clone()),
            source_refresh: None,
            integration_conflict: None,
        });
        task.repair = Some(repair);
        task.state = TaskState::Planned;
        task.diagnostic = None;
        Ok(())
    }
    /// Send a verified result whose integration conflicted back to its owner.
    /// The accepted review stays in history; the next attempt resolves the
    /// conflict markers and is independently verified again.
    pub fn request_integration_repair(
        &mut self,
        task_id: &str,
        conflict: IntegrationConflict,
    ) -> Result<()> {
        let task = self
            .tasks
            .iter_mut()
            .find(|t| t.id == task_id)
            .ok_or_else(|| Error::Invalid("unknown task".into()))?;
        require(
            task.state == TaskState::Verified
                && task.artifact.is_some()
                && task.review.as_ref().is_some_and(|r| r.accepted)
                && !conflict.paths.is_empty(),
            "integration repair needs a verified result and its conflicts",
        )?;
        let repair = RepairRound {
            finding_ids: conflict.finding_ids(),
            plan: format!(
                "Integrating the verified result into the project conflicted in {:?}. Your task worktree now holds the current project state with three-way conflict markers (<<<<<<< project / ||||||| base / >>>>>>> task) in those files. Resolve every marker so the files keep the project's newer changes and your task's intent, re-propose complete files and keep all assigned criteria satisfied.",
                conflict.paths
            ),
        };
        task.history.push(AttemptRecord {
            attempt: task.attempt,
            artifact: task.artifact.take(),
            review: task.review.take(),
            repair: Some(repair.clone()),
            source_refresh: None,
            integration_conflict: Some(conflict),
        });
        task.repair = Some(repair);
        task.state = TaskState::Planned;
        task.diagnostic = None;
        let task = task.clone();
        self.validate_history(&task)
    }
    /// Preserve unknown effects after a crash; caller reconciles before deciding
    /// whether a replacement attempt is authorized. No implicit replay/reset.
    pub fn recover_interrupted(&mut self) -> Vec<String> {
        let mut interrupted = vec![];
        for task in &mut self.tasks {
            if matches!(
                task.state,
                TaskState::Running | TaskState::Verifying | TaskState::Resuming
            ) {
                task.state = TaskState::Blocked;
                task.diagnostic = Some(
                    "Interrupted attempt requires session/workspace reconciliation; not replayed"
                        .into(),
                );
                interrupted.push(task.id.clone());
            }
        }
        interrupted
    }
    /// Restore a runtime-authenticated completed executor outcome. This method
    /// grants no quality pass and leaves the dedicated verifier pending.
    pub(crate) fn restore_executor_result(
        &mut self,
        spec: &RunSpec,
        task_id: &str,
        artifact: ArtifactRef,
    ) -> Result<()> {
        let task = self
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .ok_or_else(|| Error::Invalid("unknown task".into()))?;
        require(
            task.state == TaskState::Blocked && task.artifact.is_none() && task.review.is_none(),
            "executor restoration cannot replace an existing result or rejection",
        )?;
        self.validate_artifact(task, &artifact)?;
        let task = self.tasks.iter_mut().find(|t| t.id == task_id).unwrap();
        task.artifact = Some(artifact);
        task.state = TaskState::AwaitingVerification;
        task.diagnostic = None;
        self.validate(spec)
    }
    pub fn prepare_verification_retry(
        &mut self,
        task_id: &str,
        retry: VerificationRetry,
    ) -> Result<()> {
        let task = self
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .ok_or_else(|| Error::Invalid("unknown task".into()))?;
        require(
            task.state == TaskState::Blocked && task.review.is_none(),
            "verification replacement requires blocked task without a substantive review",
        )?;
        let artifact = task
            .artifact
            .as_ref()
            .ok_or_else(|| Error::Invalid("verification replacement has no artifact".into()))?;
        self.validate_artifact(task, artifact)?;
        require(
            retry.attempt == task.attempt && retry.artifact_hash == artifact.content_hash,
            "verification replacement changed executor attempt or artifact",
        )?;
        if let Some(session) = &retry.previous_session {
            self.bind_session(task_id, TaskOperation::Verify, session.clone())?;
        }
        let task = self.tasks.iter_mut().find(|t| t.id == task_id).unwrap();
        task.verification_retries.push(retry);
        task.state = TaskState::AwaitingVerification;
        task.diagnostic = None;
        self.validate_history(self.tasks.iter().find(|t| t.id == task_id).unwrap())
    }
    fn bind_session(
        &mut self,
        task_id: &str,
        operation: TaskOperation,
        session: String,
    ) -> Result<String> {
        require(text(&session), "session identity required")?;
        let task = self.tasks.iter().find(|t| t.id == task_id).unwrap();
        let agent_id = if operation == TaskOperation::Execute {
            task.owner_id.clone()
        } else {
            task.verifier_id.clone()
        };
        require(
            !self.agents.iter().any(|a| {
                a.id != agent_id
                    && (a.session_id.as_ref() == Some(&session)
                        || a.session_history.contains(&session))
            }),
            "executor and verifier sessions must be independent",
        )?;
        let agent = self.agents.iter_mut().find(|a| a.id == agent_id).unwrap();
        if agent.session_id.as_ref() != Some(&session) {
            if let Some(prior) = agent.session_id.take() {
                agent.session_history.insert(prior);
            }
            agent.session_history.remove(&session);
        }
        agent.session_id = Some(session);
        Ok(agent_id)
    }
    fn finish(
        &mut self,
        task_id: &str,
        operation: TaskOperation,
        outcome: Result<TaskOutcome>,
        context: &GateContext,
    ) -> Result<()> {
        let index = self.tasks.iter().position(|t| t.id == task_id).unwrap();
        let task = &self.tasks[index];
        let agent_id = if operation == TaskOperation::Execute {
            &task.owner_id
        } else {
            &task.verifier_id
        };
        require(
            self.agents
                .iter()
                .find(|a| &a.id == agent_id)
                .is_some_and(|a| a.session_id.is_some()),
            "worker session was not durably recorded",
        )?;
        match (operation, outcome?) {
            (TaskOperation::Execute, TaskOutcome::Executed(artifact)) => {
                self.validate_artifact(task, &artifact)?;
                require(
                    self.agents
                        .iter()
                        .find(|a| a.id == task.owner_id)
                        .and_then(|a| a.session_id.as_ref())
                        == Some(&artifact.session_id),
                    "executor outcome uses an unbound current session",
                )?;
                require(
                    artifact.source_hashes == context.source_hashes
                        && artifact.dependency_hashes == self.dependency_bindings(task)?,
                    "executor artifact source snapshot or dependency mismatch",
                )?;
                self.tasks[index].artifact = Some(artifact);
                self.tasks[index].state = TaskState::AwaitingVerification;
            }
            (TaskOperation::Verify, TaskOutcome::Reviewed(review)) => {
                self.validate_review(task, &review)?;
                require(
                    self.agents
                        .iter()
                        .find(|a| a.id == task.verifier_id)
                        .and_then(|a| a.session_id.as_ref())
                        == Some(&review.session_id),
                    "review outcome uses an unbound current session",
                )?;
                self.tasks[index].state = if review.accepted {
                    TaskState::Verified
                } else {
                    TaskState::ChangesRequested
                };
                self.tasks[index].review = Some(review);
            }
            _ => {
                return Err(Error::Invalid(
                    "worker outcome does not match its assigned operation".into(),
                ));
            }
        }
        Ok(())
    }
    pub fn execute_ready<F, C>(
        &mut self,
        spec: &RunSpec,
        context: &GateContext,
        runner: F,
        checkpoint: C,
    ) -> Result<Vec<CompletedTask>>
    where
        F: Fn(&TaskInvocation, &dyn Fn(WorkerProgress) -> Result<()>) -> Result<TaskOutcome> + Sync,
        C: FnMut(&TeamPlan, &SchedulerEvent) -> Result<()>,
    {
        let invocations = self.ready_wave(spec, context)?;
        self.execute_wave(spec, context, invocations, runner, checkpoint)
    }
    pub fn execute_verification<F, C>(
        &mut self,
        spec: &RunSpec,
        context: &GateContext,
        task_id: &str,
        runner: F,
        checkpoint: C,
    ) -> Result<Vec<CompletedTask>>
    where
        F: Fn(&TaskInvocation, &dyn Fn(WorkerProgress) -> Result<()>) -> Result<TaskOutcome> + Sync,
        C: FnMut(&TeamPlan, &SchedulerEvent) -> Result<()>,
    {
        let invocations = self.ready_wave_for(spec, context, Some(task_id))?;
        require(
            invocations.len() == 1 && invocations[0].operation == TaskOperation::Verify,
            "selected task verifier is not ready",
        )?;
        self.execute_wave(spec, context, invocations, runner, checkpoint)
    }
    pub fn execute_selected<F, C>(
        &mut self,
        spec: &RunSpec,
        context: &GateContext,
        task_id: &str,
        runner: F,
        checkpoint: C,
    ) -> Result<Vec<CompletedTask>>
    where
        F: Fn(&TaskInvocation, &dyn Fn(WorkerProgress) -> Result<()>) -> Result<TaskOutcome> + Sync,
        C: FnMut(&TeamPlan, &SchedulerEvent) -> Result<()>,
    {
        let invocations = self.ready_wave_for(spec, context, Some(task_id))?;
        require(
            invocations.len() == 1,
            "selected source refresh task is not ready",
        )?;
        self.execute_wave(spec, context, invocations, runner, checkpoint)
    }
    pub fn verification_ready(
        &self,
        spec: &RunSpec,
        context: &GateContext,
        task_id: &str,
    ) -> Result<()> {
        let invocations = self.ready_wave_for(spec, context, Some(task_id))?;
        require(
            invocations.len() == 1 && invocations[0].operation == TaskOperation::Verify,
            "selected task verifier is not ready",
        )
    }
    fn execute_wave<F, C>(
        &mut self,
        spec: &RunSpec,
        context: &GateContext,
        mut invocations: Vec<TaskInvocation>,
        runner: F,
        mut checkpoint: C,
    ) -> Result<Vec<CompletedTask>>
    where
        F: Fn(&TaskInvocation, &dyn Fn(WorkerProgress) -> Result<()>) -> Result<TaskOutcome> + Sync,
        C: FnMut(&TeamPlan, &SchedulerEvent) -> Result<()>,
    {
        if invocations.is_empty() {
            return Ok(vec![]);
        }
        for invocation in &mut invocations {
            let task = self
                .tasks
                .iter_mut()
                .find(|t| t.id == invocation.task.id)
                .unwrap();
            let continuing = task.state == TaskState::Resuming;
            task.state = if invocation.operation == TaskOperation::Execute {
                TaskState::Running
            } else {
                TaskState::Verifying
            };
            if invocation.operation == TaskOperation::Execute && !continuing {
                task.attempt = task
                    .attempt
                    .checked_add(1)
                    .ok_or_else(|| Error::Invalid("attempt counter exhausted".into()))?;
            }
            invocation.task = task.clone();
        }
        let ids = invocations
            .iter()
            .map(|i| i.task.id.clone())
            .collect::<Vec<_>>();
        // Unknown commit status is deliberately left Running on callback error.
        if let Err(error) = checkpoint_call(
            &mut checkpoint,
            self,
            &SchedulerEvent::WaveStarted {
                task_ids: ids.clone(),
            },
        ) {
            self.checkpoint_uncertain = true;
            return Err(error);
        }
        enum Message {
            Progress(
                String,
                TaskOperation,
                WorkerProgress,
                mpsc::SyncSender<std::result::Result<(), String>>,
            ),
            Done(String, TaskOperation, Result<TaskOutcome>),
        }
        let (sender, receiver) = mpsc::channel();
        let mut completed = vec![];
        let mut persistence_error = None;
        let mut recorded_sessions = BTreeSet::new();
        thread::scope(|scope| {
            for invocation in &invocations {
                let sender = sender.clone();
                let runner = &runner;
                let id = invocation.task.id.clone();
                let operation = invocation.operation;
                let failure_sender = sender.clone();
                let failure_id = id.clone();
                let spawned = thread::Builder::new()
                    .name(format!("architect-task-{}", &hash(&id)[..12]))
                    .spawn_scoped(scope, move || {
                        let progress = |event| {
                            let (reply, ack) = mpsc::sync_channel(0);
                            sender
                                .send(Message::Progress(id.clone(), operation, event, reply))
                                .map_err(|_| {
                                    Error::Invalid("scheduler coordinator unavailable".into())
                                })?;
                            ack.recv()
                                .map_err(|_| {
                                    Error::Invalid("checkpoint acknowledgement unavailable".into())
                                })?
                                .map_err(Error::Invalid)
                        };
                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            runner(invocation, &progress)
                        }))
                        .unwrap_or_else(|_| {
                            Err(Error::Invalid(
                                "task worker panicked; effects require reconciliation".into(),
                            ))
                        });
                        let _ = sender.send(Message::Done(id, operation, result));
                    });
                if let Err(error) = spawned {
                    let _ = failure_sender.send(Message::Done(
                        failure_id,
                        operation,
                        Err(error.into()),
                    ));
                }
            }
            drop(sender);
            while completed.len() < invocations.len() {
                let Ok(message) = receiver.recv() else { break };
                match message {
                    Message::Progress(
                        id,
                        operation,
                        WorkerProgress::SessionStarted { session_id },
                        reply,
                    ) => {
                        let result = if persistence_error.is_some() {
                            Err(Error::Invalid("checkpoint failed; wave cancelled".into()))
                        } else {
                            self.bind_session(&id, operation, session_id)
                                .and_then(|agent_id| {
                                    checkpoint_call(
                                        &mut checkpoint,
                                        self,
                                        &SchedulerEvent::SessionRecorded {
                                            task_id: id.clone(),
                                            agent_id,
                                        },
                                    )
                                })
                        };
                        if let Err(error) = &result {
                            persistence_error = Some(error.to_string());
                            for invocation in &invocations {
                                invocation.cancelled.store(true, Ordering::Release);
                            }
                        }
                        if result.is_ok() {
                            recorded_sessions.insert(id.clone());
                        }
                        let _ = reply.send(result.map_err(|error| error.to_string()));
                    }
                    Message::Done(id, operation, outcome) => {
                        let result = if persistence_error.is_some() {
                            Err(Error::Invalid(
                                "checkpoint failed; task effects require reconciliation".into(),
                            ))
                        } else {
                            match outcome {
                                Err(error) => Err(error),
                                Ok(outcome) if recorded_sessions.contains(&id) => {
                                    self.finish(&id, operation, Ok(outcome), context)
                                }
                                Ok(_) => Err(Error::Invalid(
                                    "worker omitted durable session receipt for this attempt"
                                        .into(),
                                )),
                            }
                        };
                        if let Err(error) = result {
                            let task = self.tasks.iter_mut().find(|t| t.id == id).unwrap();
                            task.state = TaskState::Blocked;
                            task.diagnostic = Some(error.to_string());
                        }
                        if persistence_error.is_none()
                            && let Err(error) = checkpoint_call(
                                &mut checkpoint,
                                self,
                                &SchedulerEvent::TaskFinished {
                                    task_id: id.clone(),
                                },
                            )
                        {
                            persistence_error = Some(error.to_string());
                            for invocation in &invocations {
                                invocation.cancelled.store(true, Ordering::Release);
                            }
                        }
                        completed.push(CompletedTask {
                            task_id: id.clone(),
                            state: self.tasks.iter().find(|t| t.id == id).unwrap().state,
                        });
                    }
                }
            }
        });
        if let Some(error) = persistence_error {
            self.checkpoint_uncertain = true;
            for task in self.tasks.iter_mut().filter(|task| ids.contains(&task.id)) {
                task.state = TaskState::Blocked;
                task.diagnostic = Some(
                    "Checkpoint outcome uncertain; reconcile durable state and effects".into(),
                );
            }
            return Err(Error::Invalid(format!(
                "scheduler persistence failed: {error}"
            )));
        }
        require(
            completed.len() == invocations.len(),
            "worker disappeared; reconcile incomplete wave",
        )?;
        self.validate(spec)?;
        Ok(completed)
    }
}
impl TaskRecord {
    pub fn new(
        id: String,
        owner_id: String,
        verifier_id: String,
        description: String,
        stage: TaskStage,
        requirement_ids: BTreeSet<String>,
        criterion_ids: BTreeSet<String>,
    ) -> Self {
        Self {
            id,
            owner_id,
            verifier_id,
            description,
            stage,
            source_dependencies: std::iter::once("objective".to_owned())
                .chain(requirement_ids.iter().map(|id| format!("requirement:{id}")))
                .collect(),
            requirement_ids,
            criterion_ids,
            plan_item_ids: BTreeSet::new(),
            deliverable: false,
            dependencies: BTreeSet::new(),
            required_gates: BTreeSet::new(),
            write_scopes: BTreeSet::new(),
            check_plan: vec![],
            change_contract: None,
            integration: IntegrationStrategy::ArtifactOnly,
            escalation: EscalationPolicy {
                on_blocked: "Escalate missing dependency with evidence".into(),
                on_conflict: "Pause colliding writes and request integration".into(),
                on_failure: "Reconcile effects before retry or replacement".into(),
                completion: "Current artifact independently satisfies every assigned criterion"
                    .into(),
            },
            state: TaskState::Planned,
            attempt: 0,
            history: vec![],
            verification_retries: vec![],
            native_resumes: vec![],
            native_replacements: vec![],
            artifact: None,
            review: None,
            repair: None,
            diagnostic: None,
        }
    }
}
