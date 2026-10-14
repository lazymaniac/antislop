use crate::{Result, hash, require};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Discovery,
    Planning,
    Execution,
    Validation,
    Integration,
    Delivery,
}
impl Stage {
    pub fn next(self) -> Option<Self> {
        match self {
            Self::Discovery => Some(Self::Planning),
            Self::Planning => Some(Self::Execution),
            Self::Execution => Some(Self::Validation),
            Self::Validation => Some(Self::Integration),
            Self::Integration => Some(Self::Delivery),
            Self::Delivery => None,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    AwaitingStart,
    Ready,
    Running,
    AwaitingResult,
    ChangesRequested,
    Paused,
    Blocked,
    Completed,
    Cancelled,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requirement {
    pub id: String,
    pub original_text: String,
    pub source_ref: String,
    #[serde(deserialize_with = "unique_criteria")]
    pub criteria: BTreeMap<String, String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanItem {
    pub id: String,
    #[serde(default)]
    pub dependencies: BTreeSet<String>,
    pub requirement_ids: BTreeSet<String>,
    pub description: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunSpec {
    #[serde(default)]
    pub task_family: crate::quality::TaskFamily,
    #[serde(default)]
    pub config: Option<crate::config::PinnedConfig>,
    pub objective: String,
    pub workspace: String,
    pub provider: String,
    pub model: Option<String>,
    pub requirements: Vec<Requirement>,
    pub plan_items: Vec<PlanItem>,
    #[serde(default)]
    pub input_files: Vec<String>,
    #[serde(default)]
    pub editable_files: Vec<String>,
    #[serde(default)]
    pub check_command: Vec<String>,
}
impl RunSpec {
    pub fn validate(&self) -> Result<()> {
        self.validate_scope(false)
    }
    fn validate_scope(&self, intake: bool) -> Result<()> {
        if intake {
            require(
                self.config.is_some(),
                "intake requires an orchestrator configuration",
            )?;
            require(
                self.input_files.is_empty()
                    && self.editable_files.is_empty()
                    && self.check_command.is_empty(),
                "intake cannot declare task execution inputs, writes or checks",
            )?;
        }
        require(!self.objective.trim().is_empty(), "objective is required")?;
        if let Some(pinned) = &self.config {
            pinned.validate()?;
            let config = &pinned.config;
            if !self.check_command.is_empty() {
                config
                    .policy
                    .authorize_local(crate::config::LocalAction::Check, true)?;
                require(
                    config.policy.mode == crate::config::Mode::Supervised,
                    "autonomous project checks require a restricted runner; unsandboxed execution is not enabled",
                )?;
            }
            if !self.editable_files.is_empty() {
                config
                    .policy
                    .authorize_local(crate::config::LocalAction::Write, true)?;
            }
            let kinds = [
                crate::config::WorkKind::Context,
                crate::config::WorkKind::Planning,
                match self.task_family {
                    crate::quality::TaskFamily::Software => crate::config::WorkKind::Implementation,
                    crate::quality::TaskFamily::Research => crate::config::WorkKind::Research,
                    crate::quality::TaskFamily::Document => crate::config::WorkKind::Writing,
                    crate::quality::TaskFamily::Analysis => crate::config::WorkKind::Analysis,
                },
                crate::config::WorkKind::Testing,
                crate::config::WorkKind::Integration,
                crate::config::WorkKind::Delivery,
            ]
            .into_iter()
            .collect();
            config.validate_work_kinds(&if intake {
                std::collections::BTreeSet::from([crate::config::WorkKind::Orchestration])
            } else {
                kinds
            })?;
        }
        require(
            matches!(
                self.provider.as_str(),
                "fake" | "codex" | "claude" | "gemini" | "agy"
            ),
            "unsupported provider",
        )?;
        require(
            !self.requirements.is_empty(),
            "requirements must not be empty",
        )?;
        require(
            self.editable_files
                .iter()
                .all(|p| self.input_files.contains(p)),
            "editable files must be pinned inputs",
        )?;
        require(
            self.editable_files.is_empty() || !self.check_command.is_empty(),
            "code edits require a check command",
        )?;
        let mut rendered_checks = BTreeSet::new();
        let mut ids = BTreeSet::new();
        for r in &self.requirements {
            require(
                !r.id.trim().is_empty() && ids.insert(&r.id),
                "duplicate or empty requirement ID",
            )?;
            require(
                !r.original_text.trim().is_empty() && !r.source_ref.trim().is_empty(),
                "requirement source is required",
            )?;
            require(!r.criteria.is_empty(), "requirement has no criteria")?;
            for (id, description) in &r.criteria {
                require(
                    rendered_checks.insert(format!("REQ:{}:{id}", r.id)),
                    "ambiguous criterion IDs",
                )?;
                require(
                    !id.trim().is_empty() && !description.trim().is_empty(),
                    "empty criterion",
                )?;
            }
        }
        let mut items = BTreeSet::new();
        for item in &self.plan_items {
            require(
                !item.id.trim().is_empty() && items.insert(&item.id),
                "duplicate or empty plan item ID",
            )?;
            require(
                !item.description.trim().is_empty() && !item.requirement_ids.is_empty(),
                "incomplete plan item",
            )?;
            require(
                item.requirement_ids.iter().all(|id| ids.contains(id)),
                "unknown plan requirement",
            )?;
        }
        require(!items.is_empty(), "plan ledger is required")?;
        let graph: BTreeMap<_, _> = self
            .plan_items
            .iter()
            .map(|item| (item.id.as_str(), &item.dependencies))
            .collect();
        for item in &self.plan_items {
            require(
                item.dependencies
                    .iter()
                    .all(|dep| graph.contains_key(dep.as_str())),
                "unknown plan dependency",
            )?;
        }
        let mut resolved = BTreeSet::new();
        loop {
            let before = resolved.len();
            for item in &self.plan_items {
                if item.dependencies.iter().all(|dep| resolved.contains(dep)) {
                    resolved.insert(item.id.clone());
                }
            }
            if resolved.len() == self.plan_items.len() {
                break;
            }
            require(resolved.len() != before, "plan dependency cycle")?;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Run {
    pub id: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub intake: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub chat_origin: bool,
    pub spec: RunSpec,
    pub spec_revision: u64,
    pub stage: Stage,
    pub status: Status,
    pub paused_from: Option<Status>,
    pub attempt: u64,
    pub approvals: Vec<Approval>,
    pub artifacts: Vec<Artifact>,
    pub reviews: Vec<Review>,
    pub evidence: Vec<Evidence>,
    pub active_session: Option<String>,
    #[serde(default)]
    pub session_ledger: crate::sessions::SessionLedger,
    #[serde(default)]
    pub communication_ledger: crate::communication::CommunicationLedger,
    #[serde(default)]
    pub message_bodies: BTreeMap<String, String>,
    /// Exact immutable packets retained for prompt/delivery audits.
    #[serde(default)]
    pub message_packets: BTreeMap<String, crate::communication::ContextPacket>,
    #[serde(default)]
    pub agent_outboxes: BTreeMap<String, crate::communication::outbox::Receipt>,
    #[serde(default)]
    pub orchestration_rounds: Vec<crate::orchestration::planning::Round>,
    #[serde(default)]
    pub team: Option<crate::team::TeamPlan>,
    #[serde(default)]
    pub team_risk_owners: Vec<crate::orchestration::planning::RiskOwner>,
    #[serde(default)]
    pub task_executions: Vec<crate::runtime::team::effects::ExecutionBinding>,
    #[serde(default)]
    pub task_checks: Vec<crate::runtime::team::checks::Invocation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub task_check_outcomes: Vec<crate::runtime::team::check_recovery::Outcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_workspace: Option<crate::workspace::managed::Workspace>,
    #[serde(default)]
    pub task_result_restorations: Vec<crate::runtime::team::restore::Restoration>,
    #[serde(default)]
    pub team_result_approvals: Vec<crate::runtime::team::stage::ResultApproval>,
    pub diagnostic: Option<String>,
    pub input_snapshot: BTreeMap<String, String>,
    #[serde(default)]
    pub checklist_version: String,
    #[serde(default)]
    pub risk_coverage: BTreeMap<String, RiskCoverageEntry>,
    /// Persistent dependency graph. Historical revisions remain inspectable.
    #[serde(default)]
    pub provenance: BTreeMap<String, ProvenanceNode>,
    #[serde(default)]
    pub provenance_reviews: BTreeMap<String, Review>,
    #[serde(default)]
    pub quality_gates: BTreeMap<String, crate::quality::GateAssessment>,
    #[serde(default)]
    pub quality: Option<crate::quality::QualityArtifacts>,
    #[serde(default)]
    pub provider_failure: Option<crate::providers::ProviderFailure>,
    #[serde(default)]
    pub pending_edits: Vec<crate::recovery::EditReceipt>,
    #[serde(default)]
    pub reconciliation_required: bool,
    #[serde(default)]
    pub diff_comments: Vec<DiffComment>,
    #[serde(default)]
    pub element_comments: Vec<crate::comments::Comment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffComment {
    pub id: String,
    pub run_id: String,
    pub path: String,
    #[serde(default)]
    pub line_number: Option<usize>,
    pub text: String,
    pub author: String,
    pub created_at: String,
    #[serde(default)]
    pub resolved: bool,
}
impl Run {
    pub fn new(id: String, spec: RunSpec) -> Result<Self> {
        Self::new_scoped(id, spec, false)
    }
    pub(crate) fn new_intake(id: String, spec: RunSpec) -> Result<Self> {
        Self::new_scoped(id, spec, true)
    }
    fn new_scoped(id: String, spec: RunSpec, intake: bool) -> Result<Self> {
        spec.validate_scope(intake)?;
        let input_snapshot = crate::workspace::read_inputs(
            std::path::Path::new(&spec.workspace),
            &spec.input_files,
        )?;
        Ok(Self {
            id,
            intake,
            chat_origin: intake,
            spec,
            spec_revision: 1,
            stage: Stage::Discovery,
            status: Status::AwaitingStart,
            paused_from: None,
            attempt: 0,
            approvals: vec![],
            artifacts: vec![],
            reviews: vec![],
            evidence: vec![],
            active_session: None,
            session_ledger: crate::sessions::SessionLedger::default(),
            communication_ledger: crate::communication::CommunicationLedger::default(),
            message_bodies: BTreeMap::new(),
            message_packets: BTreeMap::new(),
            agent_outboxes: BTreeMap::new(),
            orchestration_rounds: vec![],
            team: None,
            team_risk_owners: vec![],
            task_executions: vec![],
            task_checks: vec![],
            task_check_outcomes: vec![],
            managed_workspace: None,
            task_result_restorations: vec![],
            team_result_approvals: vec![],
            diagnostic: None,
            input_snapshot,
            checklist_version: checklist_version(),
            risk_coverage: risk_coverage(),
            provenance: BTreeMap::new(),
            provenance_reviews: BTreeMap::new(),
            quality_gates: BTreeMap::new(),
            quality: None,
            provider_failure: None,
            pending_edits: vec![],
            reconciliation_required: false,
            diff_comments: vec![],
            element_comments: vec![],
        })
    }
    pub fn scope_hash(&self, action: &str) -> String {
        hash(
            serde_json::to_vec(&(
                &self.id,
                &self.spec,
                self.spec_revision,
                self.stage,
                self.attempt,
                &self.input_snapshot,
                &self.checklist_version,
                &self.pending_edits,
                self.reconciliation_required,
                action,
                self.artifacts.last().map(|a| &a.hash),
            ))
            .expect("serializable scope"),
        )
    }
    /// Supplemental checks have a separate approval scope from the immutable
    /// RunSpec check command; every argv byte is part of the approved action.
    pub fn check_action(argv: &[String]) -> Result<String> {
        require(
            argv.first().is_some_and(|binary| !binary.trim().is_empty()),
            "empty supplemental check command",
        )?;
        require(
            argv.iter().all(|argument| !argument.contains('\0')),
            "supplemental check argv contains NUL",
        )?;
        let bytes = serde_json::to_vec(argv)?;
        require(
            bytes.len() < crate::transport::MAX_FRAME,
            "check argv exceeds transport limit",
        )?;
        Ok(format!("check:{}", hash(bytes)))
    }
    pub fn check_scope_hash(&self, argv: &[String]) -> Result<String> {
        Ok(self.scope_hash(&Self::check_action(argv)?))
    }
    pub fn team_step_scope(&self) -> Result<String> {
        let team = self
            .team
            .as_ref()
            .ok_or_else(|| crate::Error::Invalid("no team installed".into()))?;
        Ok(hash(serde_json::to_vec(&(
            &self.id,
            self.spec_revision,
            self.attempt,
            &self.input_snapshot,
            &self.checklist_version,
            &self.pending_edits,
            self.reconciliation_required,
            "step_team",
            team.revision,
            team.checkpoint_uncertain,
            &team.tasks,
            &self.team_result_approvals,
            &self.spec.config,
            self.source_hashes(),
        ))?))
    }
    pub fn wave_effect_reconciliation_scope(&self, task_ids: &[String]) -> Result<String> {
        crate::runtime::team::adjudicate::wave_scope(self, task_ids)
    }
    pub fn task_effect_reconciliation_scope(&self, task_id: &str) -> Result<String> {
        crate::runtime::team::adjudicate::scope(self, task_id)
    }
    pub fn task_result_restore_scope(&self, task_id: &str) -> Result<String> {
        crate::runtime::team::restore::scope(self, task_id)
    }
    pub fn task_source_refresh_scope(&self, task_id: &str) -> Result<String> {
        crate::runtime::team::refresh::scope(self, task_id)
    }
    pub fn task_verification_retry_scope(&self, task_id: &str) -> Result<String> {
        crate::runtime::team::retry::scope(self, task_id)
    }
    pub fn team_repair_scope(&self, task_id: &str, plan: &str) -> Result<String> {
        let team = self
            .team
            .as_ref()
            .ok_or_else(|| crate::Error::Invalid("no team installed".into()))?;
        let task = team
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .ok_or_else(|| crate::Error::Invalid("unknown task".into()))?;
        Ok(hash(serde_json::to_vec(&(
            &self.id,
            self.spec_revision,
            self.attempt,
            &self.input_snapshot,
            &self.checklist_version,
            &self.pending_edits,
            self.reconciliation_required,
            "repair_task",
            task_id,
            plan,
            team.revision,
            &task.history,
            &task.review,
            self.source_hashes(),
        ))?))
    }
    pub fn diff_review_scope(&self) -> Result<String> {
        let diff = crate::workspace::compute_diff(
            std::path::Path::new(&self.spec.workspace),
            &self.input_snapshot,
        )?;
        Ok(hash(serde_json::to_vec(&(
            self.scope_hash("review-diff"),
            self.spec_revision,
            &diff,
            &self.diff_comments,
        ))?))
    }
    pub fn sources(&self) -> BTreeMap<String, String> {
        let mut sources: BTreeMap<_, _> = self
            .input_snapshot
            .iter()
            .map(|(path, text)| (format!("file:{path}"), text.clone()))
            .collect();
        sources.insert("objective".into(), self.spec.objective.clone());
        for r in &self.spec.requirements {
            sources.insert(
                format!("requirement:{}", r.id),
                serde_json::to_string(r).expect("requirement serialization"),
            );
        }
        for p in &self.spec.plan_items {
            sources.insert(
                format!("plan:{}", p.id),
                serde_json::to_string(p).expect("plan serialization"),
            );
        }
        sources
    }
    pub fn source_hashes(&self) -> BTreeMap<String, String> {
        self.source_hashes_for_inputs(&self.input_snapshot)
    }
    /// Canonical source hashes for retained historical inputs, without cloning the run ledger.
    pub(crate) fn source_hashes_for_inputs(
        &self,
        inputs: &BTreeMap<String, String>,
    ) -> BTreeMap<String, String> {
        let mut hashes: BTreeMap<_, _> = inputs
            .iter()
            .map(|(path, value)| (format!("file:{path}"), hash(value)))
            .collect();
        hashes.insert("objective".into(), hash(&self.spec.objective));
        for requirement in &self.spec.requirements {
            hashes.insert(
                format!("requirement:{}", requirement.id),
                hash(serde_json::to_string(requirement).expect("requirement serialization")),
            );
        }
        for item in &self.spec.plan_items {
            hashes.insert(
                format!("plan:{}", item.id),
                hash(serde_json::to_string(item).expect("plan serialization")),
            );
        }
        hashes
    }
    pub fn expected_checks(&self) -> BTreeSet<String> {
        let mut ids: BTreeSet<_> = (1..=8).map(|i| format!("V{i:02}")).collect();
        ids.extend(crate::quality::expected_checks(self));
        for r in &self.spec.requirements {
            for c in r.criteria.keys() {
                ids.insert(format!("REQ:{}:{c}", r.id));
            }
        }
        for p in &self.spec.plan_items {
            ids.insert(format!("PLAN:{}", p.id));
        }
        for entry in self
            .risk_coverage
            .values()
            .filter(|entry| entry.stage == self.stage)
        {
            ids.insert(entry.check_id.clone());
        }
        let artifact_hash = self.artifacts.last().map(|artifact| artifact.hash.as_str());
        let validity = crate::verification::provenance_validity(self);
        for (id, node) in &self.provenance {
            if node.spec_revision == self.spec_revision
                && node.superseded_by.is_none()
                && node.kind != ProvenanceKind::Source
                && (!validity.get(id).copied().unwrap_or(false)
                    || (node.refresh && node.revalidation.is_none())
                    || node.revalidation.as_ref().is_some_and(|binding| {
                        Some(binding.artifact_hash.as_str()) == artifact_hash
                    }))
            {
                ids.insert(format!("REVALIDATE:{id}"));
            }
        }
        ids
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Approval {
    pub action: String,
    pub scope_hash: String,
    pub spec_revision: u64,
    pub actor: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: String,
    pub stage: Stage,
    pub attempt: u64,
    pub spec_revision: u64,
    pub session_id: String,
    pub content: String,
    pub hash: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub id: String,
    pub artifact_hash: String,
    pub spec_revision: u64,
    pub source: String,
    pub output_hash: String,
    pub output: String,
    pub success: bool,
    /// Exact workspace snapshot at receipt creation. Missing legacy runner
    /// bindings are historical and cannot authorize current behavior claims.
    #[serde(default)]
    pub input_hash: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Fail,
    Blocked,
    NotApplicable,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckResult {
    pub check_id: String,
    pub status: CheckStatus,
    pub method: String,
    pub evidence_ids: Vec<String>,
    pub rationale: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Review {
    #[serde(default)]
    pub quality: Option<crate::quality::QualityArtifacts>,
    pub artifact_hash: String,
    pub spec_revision: u64,
    pub checklist_version: String,
    pub executor_session: String,
    pub verifier_session: String,
    pub checks: Vec<CheckResult>,
    pub findings: Vec<String>,
}
pub fn checklist_version() -> String {
    hash(
        [
            b"common-v2;stage-risk-ownership-v2;provenance-revalidation-v1;task-models-v1;task-source-refresh-v1;task-check-plan-v1;task-baseline-v1;task-change-proof-v1;team-stage-result-v1;"
                .as_slice(),
            include_bytes!("../fixtures/risks.json"),
            crate::quality::catalog_hash().as_bytes(),
        ]
        .concat(),
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileEdit {
    pub path: String,
    pub content: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionOutput {
    #[serde(default)]
    pub agent_messages: Vec<crate::communication::outbox::Message>,
    #[serde(default)]
    pub quality: Option<crate::quality::QualityArtifacts>,
    pub artifact: String,
    pub edits: Vec<FileEdit>,
    pub context: Option<ContextModel>,
    pub solution: Option<SolutionModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextFact {
    pub claim: String,
    pub source_ref: String,
    pub source_hash: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Unknown {
    pub question: String,
    pub material: bool,
    pub resolution: Option<String>,
    pub source_ref: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextModel {
    pub facts: Vec<ContextFact>,
    pub unknowns: Vec<Unknown>,
}
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SolutionOption {
    pub id: String,
    pub mechanism: String,
    pub requirement_ids: BTreeSet<String>,
    pub evidence: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SimulationCase {
    pub kind: String,
    pub initial_state: String,
    pub events: Vec<String>,
    pub expected_result: String,
    pub invariant_ids: BTreeSet<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SolutionModel {
    pub criteria: Vec<String>,
    pub options: Vec<SolutionOption>,
    pub chosen_option: String,
    pub rationale: String,
    pub invariants: Vec<Invariant>,
    pub simulations: Vec<SimulationCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Invariant {
    pub id: String,
    pub statement: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskCoverageEntry {
    pub risk_id: String,
    pub stage: Stage,
    pub executor_role: String,
    pub verifier_role: String,
    pub check_id: String,
    pub status: CheckStatus,
    pub rationale: String,
    #[serde(default)]
    pub assessment: Option<RiskAssessmentBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProvenanceKind {
    Source,
    ContextFact,
    ContextModel,
    SolutionModel,
    Simulation,
    Artifact,
    Review,
    RiskAssessment,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceNode {
    pub kind: ProvenanceKind,
    pub spec_revision: u64,
    pub content_hash: String,
    /// Structured claim/model/check available to the independent verifier.
    pub content: String,
    pub source_hashes: BTreeMap<String, String>,
    /// Dependency node ID -> the exact binding hash used at assessment time.
    pub dependencies: BTreeMap<String, String>,
    pub valid: bool,
    pub revalidation: Option<ProvenanceValidation>,
    #[serde(default)]
    pub superseded_by: Option<String>,
    #[serde(default)]
    pub refresh: bool,
}
impl ProvenanceNode {
    pub fn binding_hash(&self) -> String {
        hash(
            serde_json::to_vec(&(
                &self.kind,
                self.spec_revision,
                &self.content_hash,
                &self.source_hashes,
                &self.dependencies,
                &self.revalidation,
                &self.superseded_by,
                self.refresh,
            ))
            .expect("provenance serialization"),
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceValidation {
    pub review_hash: String,
    pub verifier_session: String,
    pub artifact_hash: String,
    pub evidence_ids: Vec<String>,
    #[serde(default)]
    pub source_hashes: BTreeMap<String, String>,
    #[serde(default)]
    pub dependencies: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskAssessmentBinding {
    pub node_id: String,
    pub review_hash: String,
    pub artifact_hash: String,
    pub check_id: String,
    pub evidence_ids: Vec<String>,
    pub source_hashes: BTreeMap<String, String>,
    pub spec_revision: u64,
}
pub fn risk_coverage() -> BTreeMap<String, RiskCoverageEntry> {
    let catalog: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/risks.json")).expect("embedded catalog");
    catalog["risks"]
        .as_array()
        .expect("risk list")
        .iter()
        .map(|risk| {
            let id = risk["id"].as_str().expect("risk ID").to_string();
            let number: u16 = id[1..].parse().expect("numeric risk ID");
            let stage = match number {
                1..=5 | 101..=120 | 191 | 196 => Stage::Discovery,
                6..=10 | 121..=140 | 201..=240 => Stage::Planning,
                21..=30 | 51..=60 | 71..=80 | 141..=170 => Stage::Execution,
                31..=40 | 81..=90 | 171..=190 => Stage::Validation,
                11..=20 | 41..=50 | 61..=70 | 91..=100 => Stage::Integration,
                _ => Stage::Delivery,
            };
            (
                id.clone(),
                RiskCoverageEntry {
                    risk_id: id.clone(),
                    stage,
                    executor_role: format!("{stage:?}:executor"),
                    verifier_role: format!("{stage:?}:verifier"),
                    check_id: format!("RISK:{id}"),
                    status: CheckStatus::Blocked,
                    rationale: "Pending independent assessment".into(),
                    assessment: None,
                },
            )
        })
        .collect()
}

fn unique_criteria<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<BTreeMap<String, String>, D::Error> {
    struct Criteria;
    impl<'de> serde::de::Visitor<'de> for Criteria {
        type Value = BTreeMap<String, String>;
        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a map of unique criterion IDs to descriptions")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> std::result::Result<Self::Value, M::Error> {
            let mut criteria = BTreeMap::new();
            while let Some((id, description)) = map.next_entry::<String, String>()? {
                if criteria.insert(id, description).is_some() {
                    return Err(serde::de::Error::custom("duplicate criterion ID"));
                }
            }
            Ok(criteria)
        }
    }
    deserializer.deserialize_map(Criteria)
}

fn is_false(value: &bool) -> bool {
    !*value
}
