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
        require(!self.objective.trim().is_empty(), "objective is required")?;
        if let Some(config) = &self.config { config.validate()?; }
        require(
            matches!(self.provider.as_str(), "fake" | "codex"),
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
    pub quality_gates: BTreeMap<String, crate::quality::GateAssessment>,
    #[serde(default)]
    pub pending_edits: Vec<crate::recovery::EditReceipt>,
    #[serde(default)]
    pub reconciliation_required: bool,
}
impl Run {
    pub fn new(id: String, spec: RunSpec) -> Result<Self> {
        spec.validate()?;
        let input_snapshot = crate::workspace::read_inputs(
            std::path::Path::new(&spec.workspace),
            &spec.input_files,
        )?;
        Ok(Self {
            id,
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
            diagnostic: None,
            input_snapshot,
            checklist_version: checklist_version(),
            risk_coverage: risk_coverage(),
            provenance: BTreeMap::new(),
            quality_gates: BTreeMap::new(),
            pending_edits: vec![],
            reconciliation_required: false,
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
        self.sources()
            .into_iter()
            .map(|(key, text)| (key, hash(text)))
            .collect()
    }
    pub fn expected_checks(&self) -> BTreeSet<String> {
        let mut ids: BTreeSet<_> = (1..=8).map(|i| format!("V{i:02}")).collect();
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
                && node.kind != ProvenanceKind::Source
                && (!validity.get(id).copied().unwrap_or(false)
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
            b"common-v2;stage-risk-ownership-v2;provenance-revalidation-v1;".as_slice(),
            include_bytes!("../fixtures/risks.json"),
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
    pub quality: Option<crate::quality::QualityArtifacts>,
    pub artifact: String,
    pub edits: Vec<FileEdit>,
    pub context: Option<ContextModel>,
    pub solution: Option<SolutionModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextFact {
    pub claim: String,
    pub source_ref: String,
    pub source_hash: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unknown {
    pub question: String,
    pub material: bool,
    pub resolution: Option<String>,
    pub source_ref: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextModel {
    pub facts: Vec<ContextFact>,
    pub unknowns: Vec<Unknown>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolutionOption {
    pub id: String,
    pub mechanism: String,
    pub requirement_ids: BTreeSet<String>,
    pub evidence: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SimulationCase {
    pub kind: String,
    pub initial_state: String,
    pub events: Vec<String>,
    pub expected_result: String,
    pub invariant_ids: BTreeSet<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolutionModel {
    pub criteria: Vec<String>,
    pub options: Vec<SolutionOption>,
    pub chosen_option: String,
    pub rationale: String,
    pub invariants: Vec<Invariant>,
    pub simulations: Vec<SimulationCase>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
