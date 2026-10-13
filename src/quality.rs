//! Versioned role checklists and independently evidenced quality gates.
use crate::{Result, domain::*, hash, require};
use serde::{Deserialize, Serialize};
use schemars::JsonSchema;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskFamily { #[default] Software, Research, Document, Analysis }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GatePhase { Design, Delivery }

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct QualityArtifacts {
    pub family: TaskFamily,
    pub synthetic: bool,
    pub spec_revision: u64,
    pub source_hashes: Vec<SourceBinding>,
    pub context: Option<ContextAudit>,
    pub planning: Option<PlanningAudit>,
    pub software: Option<SoftwareQuality>,
    pub research: Option<ResearchReport>,
    pub analysis: Option<AnalysisReport>,
    pub editorial: Option<EditorialReport>,
    pub delivery: Option<DeliveryReport>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateAssessment {
    pub gate_id: String, pub phase: GatePhase, pub status: CheckStatus,
    pub spec_revision: u64, pub source_hashes: BTreeMap<String, String>,
    pub artifact_hash: String, pub review_hash: String, pub check_id: String,
    pub evidence_ids: Vec<String>, pub report_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextAudit {
    pub source_refs: BTreeSet<String>, pub consumers: Vec<String>, pub contracts: Vec<String>,
    pub configuration: Vec<String>, pub standards: Vec<String>,
    pub baseline: String, pub unresolved_material_unknowns: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PlanningAudit {
    pub option_ids: BTreeSet<String>, pub comparison_criteria: Vec<String>,
    pub rejected_options: Vec<OptionRejection>, pub chosen_option: String,
    pub sensitivity: String, pub invariants: Vec<String>, pub scenario_kinds: BTreeSet<String>,
    pub requirement_ids: BTreeSet<String>, pub verification_methods: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SoftwareQuality {
    pub contract: CodeQualityContract, pub architecture: ArchitectureReview,
    pub input_domain: InputDomainMatrix, pub dependencies: DependencyReport,
    pub traceability: Vec<TraceabilityEntry>, pub refactor: RefactorReport,
    pub static_analysis: Option<StaticAnalysisReport>, pub behavior: Option<BehaviorReport>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CodeQualityContract {
    pub languages: Vec<LanguageVersion>, pub project_rules: Vec<String>, pub idioms: Vec<String>,
    pub api_rules: Vec<String>, pub error_rules: Vec<String>, pub ownership_rules: Vec<String>,
    pub security_rules: Vec<String>, pub concurrency_rules: Vec<String>,
    pub analyzers: Vec<AnalyzerContract>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalyzerContract {
    pub id: String, pub tool: String, pub version: String, pub command: Vec<String>,
    pub rules: Vec<String>, pub targets: BTreeSet<String>, pub configurations: BTreeSet<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArchitectureReview {
    pub boundaries: Vec<String>, pub responsibilities: Vec<String>, pub dependency_directions: Vec<String>,
    pub producer_consumer_contracts: Vec<String>, pub failure_scenarios: Vec<String>,
    pub alternatives: Vec<String>, pub pattern_decisions: Vec<String>, pub change_risks: Vec<ChangeRisk>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeRisk {
    pub scenario: String, pub requirement_ids: BTreeSet<String>, pub impact: String,
    pub likelihood_rationale: String, pub detection: String, pub reversibility: String,
    pub mitigation: String, pub residual_owner: String, pub acceptance_criterion: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputDomainMatrix {
    /// Independently enumerated interface/field/config/environment/state inputs.
    pub inventory: BTreeSet<String>, pub inputs: Vec<InputContract>, pub interactions: Vec<InputInteraction>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputContract {
    pub id: String, pub source_ref: String, pub value_type: String, pub unit: String,
    pub domain: String, pub default: String, pub normalization: String,
    pub invalid_behavior: String, pub categories: Vec<DomainCase>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DomainCase {
    pub category: String, pub applicable: bool, pub rationale: String, pub expected_result: String, pub scenario_ids: BTreeSet<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InputInteraction {
    pub input_ids: BTreeSet<String>, pub rule: String, pub required_order: usize,
    pub tested_order: usize, pub expected_result: String, pub scenario_ids: BTreeSet<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DependencyReport {
    pub reuse_searches: Vec<ReuseDecision>, pub inventory: BTreeSet<String>,
    pub licenses: Vec<DependencyLicense>, pub project_license: String, pub distribution: String,
    pub vulnerability_database_date: String, pub security_assessment: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReuseDecision {
    pub contract: String, pub dated_sources: Vec<String>, pub candidates: Vec<String>,
    pub chosen: String, pub rejection_reasons: Vec<OptionRejection>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DependencyLicense {
    pub package: String, pub version: String, pub features: BTreeSet<String>, pub provenance: String,
    pub license_expression: String, pub interpretation: String, pub compatible: bool,
    pub obligations: Vec<LicenseObligation>, pub unresolved: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceabilityEntry {
    pub element: String, pub classification: String, pub requirement_ids: BTreeSet<String>,
    pub rationale: String, pub removal_analysis: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RefactorReport {
    pub smell_assessments: Vec<SmellAssessment>, pub transformations: Vec<String>,
    pub boy_scout_decisions: Vec<String>, pub behavior_contract: String,
    pub behavior_evidence_ids: Vec<String>, pub unresolved_necessary_refactors: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StaticAnalysisReport {
    pub analyses: Vec<AnalyzerResult>, pub suppressions: Vec<SuppressionReview>,
    pub unresolved_findings: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalyzerResult {
    pub analyzer_id: String, pub version: String, pub command: Vec<String>,
    pub targets: BTreeSet<String>, pub configurations: BTreeSet<String>,
    pub rules: Vec<String>, pub raw_evidence_ids: Vec<String>, pub diagnostics: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SuppressionReview {
    pub rule: String, pub original_finding: String, pub justification: String,
    pub evidence_ids: Vec<String>, pub independently_confirmed_false_positive: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BehaviorReport {
    pub requirement_scenarios: Vec<ScenarioCoverage>,
    pub invariant_scenarios: Vec<ScenarioCoverage>, pub oracle: String,
    pub scenario_evidence: Vec<ScenarioEvidence>, pub mutation_or_counterexample: String,
    pub unchanged_test_contracts: String, pub unresolved_findings: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResearchReport {
    pub claims: Vec<SourceClaim>, pub coverage: String, pub contradictory_sources: Vec<String>,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceClaim {
    pub claim: String, pub source_ref: String, pub source_hash: String, pub primary: bool,
    pub date: String, pub support: String, pub independent_read_evidence_ids: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AnalysisReport {
    pub dataset_sources: BTreeSet<String>, pub units: Vec<UnitDefinition>, pub assumptions: Vec<String>,
    pub method: String, pub independent_calculation_evidence_ids: Vec<String>,
    pub uncertainty: String, pub sensitivity: String, pub conclusions: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditorialReport {
    pub audience: String, pub brief_coverage: BTreeSet<String>, pub claims: Vec<SourceClaim>,
    pub structure: String, pub repetition_review: String, pub unsupported_claims: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeliveryReport {
    pub requirement_ids: BTreeSet<String>, pub instructions: Vec<String>, pub demonstrated_results: Vec<String>,
    pub limitations: Vec<String>, pub unsupported_claims: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceBinding { pub source_ref: String, pub source_hash: String }
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OptionRejection { pub option_id: String, pub reason: String }
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LanguageVersion { pub language: String, pub version: String }
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LicenseObligation { pub obligation: String, pub fulfillment: String }
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SmellAssessment { pub smell_id: String, pub assessment: String }
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScenarioCoverage { pub criterion_id: String, pub scenario_ids: BTreeSet<String> }
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScenarioEvidence { pub scenario_id: String, pub evidence_ids: Vec<String> }
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnitDefinition { pub field: String, pub unit: String }

pub const INPUT_CATEGORIES: &[&str] = &[
    "missing", "null", "empty", "zero", "negative", "minimum", "maximum", "boundary_neighbors",
    "out_of_range", "oversized", "invalid_encoding", "unicode", "duplicate", "contradictory",
    "wrong_order", "partial", "untrusted", "non_finite", "nan", "timezones", "time_order",
    "path_normalization", "access_boundaries", "fragmentation", "eof", "stream_error",
];

#[derive(Debug, Clone, Serialize)]
pub struct RoleCheckDefinition {
    pub check_id: String, pub role: String, pub stages: Vec<Stage>, pub family: Option<TaskFamily>,
    pub risk_ids: BTreeSet<String>, pub question: String, pub method: String,
    pub evidence_kind: String, pub rejection_condition: String, pub not_applicable_condition: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GateDefinition {
    pub gate_id: String, pub phase: GatePhase, pub stages: Vec<Stage>, pub software_only: bool,
    pub runner_required: bool,
}

fn role_definitions() -> Vec<(&'static str, Vec<Stage>, Option<TaskFamily>, Vec<&'static str>, Vec<u16>)> {
    use Stage::*;
    use TaskFamily::*;
    vec![
        ("orchestration", vec![Discovery], None, vec!["competencies", "dedicated_pairs", "dependencies", "permissions", "round_approvals"], (11..=20).collect()),
        ("context", vec![Discovery], None, vec!["fact_sources", "consumers", "contracts", "configuration", "baseline", "standards", "unknowns"], (101..=120).collect()),
        ("requirements", vec![Discovery], None, vec!["original_source", "atomicity", "conditions", "prohibitions", "conflicts", "measurable_criteria"], (191..=200).collect()),
        ("plan", vec![Planning], None, vec!["model", "inputs", "invariants", "states", "failures", "dependencies", "simulations", "evidence"], (121..=140).collect()),
        ("options", vec![Planning], None, vec!["real_alternatives", "criteria_first", "common_conditions", "mandatory_filters", "evidence", "sensitivity", "omitted_alternatives"], (231..=240).collect()),
        ("architecture", vec![Planning, Integration], Some(Software), vec!["boundaries", "responsibilities", "dependency_directions", "patterns", "data_consistency", "change_risks", "alternatives"], (201..=210).collect()),
        ("dependency", vec![Planning, Integration], Some(Software), vec!["reuse_search", "comparison", "provenance", "direct_licenses", "transitive_licenses", "distribution_obligations"], (221..=230).collect()),
        ("code", vec![Execution, Integration], Some(Software), vec!["model_conformance", "standards", "necessary_code", "input_contracts", "errors", "ownership", "duplication"], (141..=155).collect()),
        ("refactor", vec![Execution, Integration], Some(Software), vec!["necessary_transformations", "boy_scout", "behavior_preserved", "scope"], (156..=165).collect()),
        ("static", vec![Validation, Integration], Some(Software), vec!["rules", "versions", "target_scope", "feature_matrix", "raw_results", "suppressions", "existing_findings"], (166..=170).collect()),
        ("test_design", vec![Planning, Validation], Some(Software), vec!["every_criterion", "input_classes", "boundaries", "errors", "invariants", "integration", "oracle"], (171..=180).collect()),
        ("test", vec![Validation, Integration], Some(Software), vec!["real_path", "assertions", "mocks", "isolation", "determinism", "detects_wrong_implementation"], (181..=190).collect()),
        ("security", vec![Execution, Validation], Some(Software), vec!["trust_boundaries", "authentication", "authorization", "data", "dependencies", "injection", "secrets", "abuse"], (51..=60).collect()),
        ("performance", vec![Execution, Validation], Some(Software), vec!["representative_input", "time", "memory", "baseline", "tails", "semantics"], (71..=80).collect()),
        ("integration", vec![Integration], None, vec!["contract_composition", "migrations", "configuration", "concurrency", "whole_result_regressions"], (41..=50).collect()),
        ("compliance", vec![Delivery], None, vec!["original_vs_ledger", "all_children", "all_plan_items", "evidence_quality", "freshness"], (191..=200).collect()),
        ("research", vec![Execution, Validation, Integration], Some(Research), vec!["existence", "primary_sources", "freshness", "claim_support", "contradictory_sources"], (81..=90).collect()),
        ("analysis", vec![Execution, Validation, Integration], Some(Analysis), vec!["provenance", "data_completeness", "units", "method", "uncertainty", "reproduction"], (81..=90).collect()),
        ("editorial", vec![Execution, Validation, Integration], Some(Document), vec!["brief", "audience", "every_claim", "structure", "repetition", "citations"], (81..=90).collect()),
        ("delivery", vec![Delivery], None, vec!["report_matches_result", "instructions", "results", "limitations", "requirements"], (91..=100).collect()),
    ]
}

pub fn catalog() -> Vec<RoleCheckDefinition> {
    let mut checks = Vec::new();
    for (role, stages, family, facets, risks) in role_definitions() {
        for facet in facets {
            checks.push(RoleCheckDefinition {
                check_id: format!("ROLE:{role}:{facet}"), role: role.into(), stages: stages.clone(), family,
                risk_ids: risks.iter().map(|number| format!("R{number:03}")).collect(),
                question: format!("Independently verify {facet} for the current {role} artifact and its consumers."),
                method: format!("Inspect the structured {role} report, source contracts and counterexamples for {facet}; reproduce material claims using current receipts."),
                evidence_kind: "Current source/model read, traceable scenario, or actual runner result appropriate to stage".into(),
                rejection_condition: format!("Missing, stale, unsupported or contradicted {facet}; any unresolved mandatory obligation"),
                not_applicable_condition: "Only an explicitly optional domain property with scope-specific rationale and inspected source evidence; mandatory role coverage is never N/A".into(),
            });
        }
    }
    let smells = ["Long Method", "Large Class", "Primitive Obsession", "Long Parameter List", "Data Clumps", "Switch Statements", "Temporary Field", "Refused Bequest", "Alternative Classes with Different Interfaces", "Divergent Change", "Shotgun Surgery", "Parallel Inheritance Hierarchies", "Comments", "Duplicate Code", "Lazy Class", "Data Class", "Dead Code", "Speculative Generality", "Feature Envy", "Inappropriate Intimacy", "Message Chains", "Middle Man", "Incomplete Library Class"];
    for (index, name) in smells.into_iter().enumerate() {
        checks.push(RoleCheckDefinition {
            check_id: format!("SM{:02}", index + 1), role: "refactor".into(),
            stages: vec![Stage::Execution, Stage::Integration], family: Some(TaskFamily::Software),
            risk_ids: (156..=165).map(|number| format!("R{number:03}")).collect(),
            question: format!("Does {name} obstruct this solution or a dependency necessary for its correctness?"),
            method: "Inspect actual responsibilities, callers and before/after behavior; decide using the project language and contracts.".into(),
            evidence_kind: "Current code and caller reads; behavior receipts for required transformations".into(),
            rejection_condition: "A necessary structural change is deferred, or a transformation changes a preserved contract".into(),
            not_applicable_condition: "A concrete source inspection demonstrates why the symptom is inapplicable to this scope".into(),
        });
    }
    checks
}

pub fn checklist(run: &Run) -> Vec<RoleCheckDefinition> {
    catalog().into_iter().filter(|definition| definition.stages.contains(&run.stage)
        && definition.family.is_none_or(|family| family == run.spec.task_family)).collect()
}

pub fn gate_definitions() -> Vec<GateDefinition> {
    use Stage::*;
    let mut gates = vec![
        ("context_ready", GatePhase::Design, vec![Discovery], false, false),
        ("solution_selected", GatePhase::Design, vec![Planning], false, false),
        ("plan_ready", GatePhase::Design, vec![Planning], false, false),
        ("requirements_complete", GatePhase::Delivery, vec![Delivery], false, false),
        ("review_complete", GatePhase::Delivery, vec![Delivery], false, false),
    ];
    for gate in ["architecture_verified", "input_domain_verified", "dependencies_verified"] {
        gates.push((gate, GatePhase::Design, vec![Planning], true, false));
        gates.push((gate, GatePhase::Delivery, vec![Integration], true, true));
    }
    for gate in ["scope_clean", "standards_clean", "refactor_complete"] {
        gates.push((gate, GatePhase::Delivery, vec![Execution, Integration], true, true));
    }
    for gate in ["static_clean", "behavior_verified"] {
        gates.push((gate, GatePhase::Delivery, vec![Validation, Integration], true, true));
    }
    gates.push(("integration_verified", GatePhase::Delivery, vec![Integration], false, true));
    gates.into_iter().map(|(id, phase, stages, software_only, runner_required)| GateDefinition {
        gate_id: id.into(), phase, stages, software_only, runner_required,
    }).collect()
}

fn phase_name(phase: GatePhase) -> &'static str { match phase { GatePhase::Design => "design", GatePhase::Delivery => "delivery" } }
pub fn gate_check_id(gate_id: &str, phase: GatePhase) -> String { format!("GATE:{gate_id}:{}", phase_name(phase)) }
pub fn catalog_hash() -> String { hash(serde_json::to_vec(&(catalog(), gate_definitions(), INPUT_CATEGORIES, "typed-reports-v1")).expect("quality catalog serialization")) }

pub fn schema(_run: &Run) -> serde_json::Value {
    let schema = serde_json::to_value(schemars::schema_for!(QualityArtifacts)).expect("quality schema serialization");
    fn inline(value: &serde_json::Value, definitions: &serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(object) => {
                if let Some(reference) = object.get("$ref").and_then(|value| value.as_str()) {
                    let name = reference.strip_prefix("#/$defs/").expect("local quality schema reference");
                    return inline(&definitions[name], definitions);
                }
                let mut result = serde_json::Map::new();
                for (key, value) in object {
                    if !matches!(key.as_str(), "$schema" | "$defs" | "uniqueItems") {
                        result.insert(key.clone(), inline(value, definitions));
                    }
                }
                if let Some(properties) = result.get("properties").and_then(|value| value.as_object()) {
                    let required: Vec<_> = properties.keys().cloned().collect();
                    result.insert("required".into(), serde_json::json!(required));
                    result.insert("additionalProperties".into(), serde_json::json!(false));
                }
                serde_json::Value::Object(result)
            },
            serde_json::Value::Array(values) => serde_json::Value::Array(values.iter().map(|value| inline(value, definitions)).collect()),
            other => other.clone(),
        }
    }
    inline(&schema, &schema["$defs"])
}

fn nonempty(value: &str) -> bool { !value.trim().is_empty() }
fn strings(values: &[String]) -> bool { !values.is_empty() && values.iter().all(|value| nonempty(value)) }
fn requirements(run: &Run) -> BTreeSet<String> { run.spec.requirements.iter().map(|requirement| requirement.id.clone()).collect() }
fn criteria(run: &Run) -> BTreeSet<String> { run.spec.requirements.iter().flat_map(|requirement| requirement.criteria.keys().map(|id| format!("REQ:{}:{id}",requirement.id))).collect() }
fn source_bindings(report: &QualityArtifacts) -> Result<BTreeMap<String, String>> {
    let sources: BTreeMap<_, _> = report.source_hashes.iter().map(|source| (source.source_ref.clone(), source.source_hash.clone())).collect();
    require(sources.len() == report.source_hashes.len(), "duplicate quality source binding")?;
    Ok(sources)
}
fn validate_envelope(run: &Run, report: &QualityArtifacts) -> Result<()> {
    require(report.family == run.spec.task_family && report.spec_revision == run.spec_revision,
        "wrong task family or stale quality report revision")?;
    require(report.synthetic == (run.spec.provider == "fake"), "synthetic quality reports are restricted to the fake provider")?;
    require(source_bindings(report)? == run.source_hashes(), "quality report source bindings are incomplete or stale")
}

pub fn validate_input_domain(run: &Run, matrix: &InputDomainMatrix) -> Result<()> {
    let actual: BTreeSet<_> = matrix.inputs.iter().map(|input| input.id.clone()).collect();
    require(!actual.is_empty() && actual == matrix.inventory && actual.len() == matrix.inputs.len(), "input inventory is empty, duplicate or incomplete")?;
    let expected: BTreeSet<_> = INPUT_CATEGORIES.iter().map(|category| category.to_string()).collect();
    for input in &matrix.inputs {
        require(run.source_hashes().contains_key(&input.source_ref)
            && [&input.id,&input.value_type,&input.unit,&input.domain,&input.default,&input.normalization,&input.invalid_behavior].into_iter().all(|field| nonempty(field)), "input contract lacks source, domain, units or error behavior")?;
        let categories: BTreeSet<_> = input.categories.iter().map(|case| case.category.clone()).collect();
        require(categories == expected && categories.len() == input.categories.len(), "input category omitted, duplicated or unknown")?;
        for case in &input.categories {
            require(nonempty(&case.rationale) && (!case.applicable || (nonempty(&case.expected_result) && !case.scenario_ids.is_empty())), "input class has no justified applicability, expected result or scenario")?;
        }
    }
    let mut participants = BTreeSet::new();
    for interaction in &matrix.interactions {
        require(interaction.input_ids.len() >= 2 && interaction.input_ids.is_subset(&actual)
            && interaction.required_order >= interaction.input_ids.len()
            && interaction.tested_order >= interaction.required_order
            && nonempty(&interaction.rule) && nonempty(&interaction.expected_result) && !interaction.scenario_ids.is_empty(), "input interaction is unmodeled or pairwise coverage cannot prove its required order")?;
        participants.extend(interaction.input_ids.iter().cloned());
    }
    require(actual.len() < 2 || participants == actual, "inputs lack an explicit interaction or independence assessment")
}

fn validate_contract(contract: &CodeQualityContract) -> Result<()> {
    require(!contract.languages.is_empty() && contract.languages.iter().all(|language| nonempty(&language.language) && nonempty(&language.version))
        && [&contract.project_rules,&contract.idioms,&contract.api_rules,&contract.error_rules,&contract.ownership_rules,&contract.security_rules,&contract.concurrency_rules].into_iter().all(|rules| strings(rules)), "code quality contract omits language versions or mandatory rules")?;
    let ids: BTreeSet<_> = contract.analyzers.iter().map(|analyzer| &analyzer.id).collect();
    require(!ids.is_empty() && ids.len() == contract.analyzers.len(), "analysis contract missing or duplicate")?;
    for analyzer in &contract.analyzers {
        require(nonempty(&analyzer.id) && nonempty(&analyzer.tool) && nonempty(&analyzer.version) && strings(&analyzer.command)
            && strings(&analyzer.rules) && !analyzer.targets.is_empty() && !analyzer.configurations.is_empty(), "analysis contract has no version, rules, command or target/configuration matrix")?;
    }
    Ok(())
}

fn validate_architecture(run: &Run, architecture: &ArchitectureReview) -> Result<()> {
    require([&architecture.boundaries,&architecture.responsibilities,&architecture.dependency_directions,&architecture.producer_consumer_contracts,&architecture.failure_scenarios,&architecture.alternatives,&architecture.pattern_decisions].into_iter().all(|items| strings(items)) && !architecture.change_risks.is_empty(), "architecture report omits boundaries, contracts, alternatives or change-risk scenarios")?;
    let expected = requirements(run);
    for risk in &architecture.change_risks {
        require(!risk.requirement_ids.is_empty() && risk.requirement_ids.is_subset(&expected)
            && [&risk.scenario,&risk.impact,&risk.likelihood_rationale,&risk.detection,&risk.reversibility,&risk.mitigation,&risk.residual_owner,&risk.acceptance_criterion].into_iter().all(|value| nonempty(value)), "change risk lacks affected requirements, scenario, mitigation or residual acceptance")?;
    }
    Ok(())
}

pub fn validate_dependencies(report: &DependencyReport) -> Result<()> {
    require(!report.reuse_searches.is_empty() && nonempty(&report.project_license) && nonempty(&report.distribution)
        && nonempty(&report.vulnerability_database_date) && nonempty(&report.security_assessment), "dependency report omits reuse search, licensing context or dated security assessment")?;
    for search in &report.reuse_searches {
        let candidates: BTreeSet<_> = search.candidates.iter().cloned().collect();
        let rejected: BTreeSet<_> = search.rejection_reasons.iter().map(|reason| reason.option_id.clone()).collect();
        require(nonempty(&search.contract) && strings(&search.dated_sources) && candidates.len() >= 2 && candidates.len() == search.candidates.len()
            && candidates.contains(&search.chosen) && rejected == candidates.iter().filter(|id| *id != &search.chosen).cloned().collect()
            && rejected.len() == search.rejection_reasons.len() && search.rejection_reasons.iter().all(|reason| nonempty(&reason.reason)), "reuse/build decision lacks real alternatives, dated sources or rejection reasons")?;
    }
    let actual: BTreeSet<_> = report.licenses.iter().map(|license| license.package.clone()).collect();
    require(actual == report.inventory && actual.len() == report.licenses.len(), "direct/transitive dependency license inventory is incomplete or duplicate")?;
    for license in &report.licenses {
        require([&license.package,&license.version,&license.provenance,&license.license_expression,&license.interpretation].into_iter().all(|field| nonempty(field))
            && !matches!(license.license_expression.to_lowercase().as_str(), "unknown" | "unlicensed" | "none" | "noassertion")
            && license.compatible && license.unresolved.is_empty()
            && !license.obligations.is_empty() && license.obligations.iter().all(|obligation| nonempty(&obligation.obligation) && nonempty(&obligation.fulfillment)), "unknown/incompatible license or unfulfilled distribution obligation")?;
    }
    Ok(())
}

fn validate_traceability(run: &Run, entries: &[TraceabilityEntry]) -> Result<()> {
    let expected = requirements(run);
    let mut covered = BTreeSet::new();
    require(!entries.is_empty(), "missing bidirectional code traceability")?;
    for entry in entries {
        require(nonempty(&entry.element) && nonempty(&entry.rationale) && nonempty(&entry.removal_analysis)
            && matches!(entry.classification.as_str(), "explicit_requirement" | "necessary_mechanism" | "quality_improvement")
            && entry.requirement_ids.is_subset(&expected)
            && (entry.classification == "quality_improvement" || !entry.requirement_ids.is_empty()), "out-of-scope or unjustified implementation element")?;
        covered.extend(entry.requirement_ids.iter().cloned());
    }
    require(covered == expected, "traceability omits a mandatory requirement")
}

fn validate_refactor(report: &RefactorReport) -> Result<()> {
    let expected: BTreeSet<_> = (1..=23).map(|number| format!("SM{number:02}")).collect();
    let actual: BTreeSet<_> = report.smell_assessments.iter().map(|assessment| assessment.smell_id.clone()).collect();
    require(actual == expected && actual.len() == report.smell_assessments.len()
        && report.smell_assessments.iter().all(|assessment| nonempty(&assessment.assessment))
        && strings(&report.boy_scout_decisions) && nonempty(&report.behavior_contract)
        && report.unresolved_necessary_refactors.is_empty(), "refactor report omits SM01–SM23, behavior preservation or leaves necessary debt")
}

fn runner_receipt<'a>(run: &'a Run, artifact: &Artifact, evidence_id: &str) -> Result<&'a Evidence> {
    let evidence = run.evidence.iter().find(|evidence| evidence.id == evidence_id)
        .ok_or_else(|| crate::Error::Invalid("unknown quality runner evidence".into()))?;
    require(evidence.success && evidence.source.starts_with("runner:") && !evidence.source.starts_with("runner:baseline:")
        && evidence.output_hash == hash(&evidence.output)
        && crate::verification::evidence_is_current(run, artifact, evidence), "quality gate needs an actual successful current runner receipt; baseline/artifact-read is insufficient")?;
    Ok(evidence)
}

fn raw_has_diagnostic(output: &str) -> bool {
    let raw = serde_json::from_str::<serde_json::Value>(output).ok();
    let text = raw.as_ref().map(|raw| format!("{}\n{}", raw["stdout"].as_str().unwrap_or(""),raw["stderr"].as_str().unwrap_or(""))).unwrap_or_else(|| output.into());
    text.lines().any(|line| {
        let lower = line.trim().to_lowercase();
        lower.starts_with("warning") || lower.starts_with("error") || lower.contains(": warning") || lower.contains(": error") || lower.contains("\"level\":\"warning\"") || lower.contains("\"level\":\"error\"")
    })
}

pub fn validate_static_analysis(run: &Run, artifact: &Artifact, contract: &CodeQualityContract, report: &StaticAnalysisReport) -> Result<()> {
    require(report.unresolved_findings.is_empty(), "static analysis has unresolved findings (including existing baseline findings)")?;
    let expected: BTreeSet<_> = contract.analyzers.iter().map(|analyzer| &analyzer.id).collect();
    let actual: BTreeSet<_> = report.analyses.iter().map(|analysis| &analysis.analyzer_id).collect();
    require(actual == expected && actual.len() == report.analyses.len(), "static analysis omits or duplicates a contracted analyzer")?;
    for analysis in &report.analyses {
        let analyzer = contract.analyzers.iter().find(|analyzer| analyzer.id == analysis.analyzer_id).expect("known analyzer");
        require(analysis.version == analyzer.version && analysis.command == analyzer.command && analysis.rules == analyzer.rules
            && analysis.targets == analyzer.targets && analysis.configurations == analyzer.configurations
            && !analysis.raw_evidence_ids.is_empty() && analysis.diagnostics.is_empty(), "analysis changed version, rules, target scope, feature matrix or left diagnostics")?;
        let source = format!("runner:{}",serde_json::to_string(&analysis.command)?);
        for evidence_id in &analysis.raw_evidence_ids {
            let evidence = runner_receipt(run, artifact, evidence_id)?;
            require(evidence.source == source && !raw_has_diagnostic(&evidence.output), "raw analysis result has diagnostics or does not match contracted command")?;
        }
    }
    for suppression in &report.suppressions {
        require(nonempty(&suppression.rule) && nonempty(&suppression.original_finding) && nonempty(&suppression.justification)
            && suppression.independently_confirmed_false_positive && !suppression.evidence_ids.is_empty(), "suppression hides a finding without independently proved false-positive resolution")?;
        for evidence_id in &suppression.evidence_ids { runner_receipt(run, artifact, evidence_id)?; }
    }
    Ok(())
}
