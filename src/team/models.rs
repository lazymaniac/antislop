//! Source-bound role artifacts and independent model inspections for native task turns.
use super::{CatalogRole, TaskRecord};
use crate::{Result, domain::*, hash, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const CONTRACT: &str = "task-models-v1";
pub const CONTEXT_SECTIONS: [&str; 8] = [
    "intent",
    "domain",
    "behavior",
    "dependencies",
    "contracts",
    "standards",
    "environment",
    "baseline",
];
pub const SCENARIO_KINDS: [&str; 9] = [
    "success",
    "boundary",
    "invalid",
    "failure",
    "repeated_or_delayed",
    "cancel_restart",
    "concurrency",
    "integration",
    "counterexample",
];

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextSections {
    pub intent: Vec<ContextFact>,
    pub domain: Vec<ContextFact>,
    pub behavior: Vec<ContextFact>,
    pub dependencies: Vec<ContextFact>,
    pub contracts: Vec<ContextFact>,
    pub standards: Vec<ContextFact>,
    pub environment: Vec<ContextFact>,
    pub baseline: Vec<ContextFact>,
}
impl ContextSections {
    fn facts(&self) -> [&Vec<ContextFact>; 8] {
        [
            &self.intent,
            &self.domain,
            &self.behavior,
            &self.dependencies,
            &self.contracts,
            &self.standards,
            &self.environment,
            &self.baseline,
        ]
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextArtifact {
    pub model: ContextModel,
    pub sections: ContextSections,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ScenarioOutcome {
    Pass,
    Fail,
    NotApplicable,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelScenario {
    pub scenario_id: String,
    pub model_revision: u64,
    pub criterion_ids: BTreeSet<String>,
    pub scenario: SimulationCase,
    pub outcome: ScenarioOutcome,
    pub rationale: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SolutionArtifact {
    pub model: SolutionModel,
    pub scenarios: Vec<ModelScenario>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub contract_version: String,
    pub artifact: String,
    pub edits: Vec<FileEdit>,
    pub agent_messages: Vec<crate::communication::outbox::Message>,
    #[serde(deserialize_with = "required_nullable")]
    pub context: Option<ContextArtifact>,
    #[serde(deserialize_with = "required_nullable")]
    pub solution: Option<SolutionArtifact>,
}
fn required_nullable<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ModelReview {
    #[serde(deserialize_with = "required_nullable")]
    pub context_hash: Option<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub solution_hash: Option<String>,
    pub inspected_context_sections: BTreeSet<String>,
    #[serde(deserialize_with = "required_nullable")]
    pub independent_context: Option<ContextModel>,
    #[serde(deserialize_with = "required_nullable")]
    pub context_counterexample: Option<String>,
    pub independent_simulations: Vec<ModelScenario>,
}
fn text(value: &str) -> bool {
    !value.trim().is_empty() && !value.contains('\0')
}

/// Expand local schema references without loosening the provider's strict object contract.
fn inline(value: &serde_json::Value, definitions: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(fields) => {
            if let Some(reference) = fields.get("$ref").and_then(|v| v.as_str()) {
                return inline(
                    &definitions[reference
                        .strip_prefix("#/$defs/")
                        .expect("local model schema")],
                    definitions,
                );
            }
            let mut fields: serde_json::Map<String, serde_json::Value> = fields
                .iter()
                .filter(|(key, _)| !matches!(key.as_str(), "$defs" | "$schema" | "uniqueItems"))
                .map(|(k, v)| (k.clone(), inline(v, definitions)))
                .collect();
            if let Some(properties) = fields.get("properties").and_then(|v| v.as_object()) {
                let required = properties.keys().cloned().collect::<Vec<_>>();
                fields.insert("required".into(), serde_json::json!(required));
                fields.insert("additionalProperties".into(), serde_json::json!(false));
            }
            serde_json::Value::Object(fields)
        }
        serde_json::Value::Array(values) => values.iter().map(|v| inline(v, definitions)).collect(),
        _ => value.clone(),
    }
}
pub fn schema<T: JsonSchema>() -> serde_json::Value {
    let schema = serde_json::to_value(schemars::schema_for!(T)).expect("model schema");
    inline(&schema, &schema["$defs"])
}
pub fn output_schema() -> serde_json::Value {
    serde_json::json!({"type":"object","properties":{
        "contract_version":{"type":"string","enum":[CONTRACT]},
        "artifact":{"type":"string"},"edits":{"type":"array","items":{
            "type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},
            "required":["path","content"],"additionalProperties":false}},
        "agent_messages":crate::communication::outbox::schema(),
        "context":{"anyOf":[schema::<ContextArtifact>(),{"type":"null"}]},
        "solution":{"anyOf":[schema::<SolutionArtifact>(),{"type":"null"}]}
    },"required":["contract_version","artifact","edits","agent_messages","context","solution"],"additionalProperties":false})
}

fn validate_scenarios(
    run: &Run,
    task: &TaskRecord,
    model: &SolutionModel,
    scenarios: &[ModelScenario],
) -> Result<()> {
    let mut ids = BTreeSet::new();
    let mut kinds = BTreeSet::new();
    let mut covered = BTreeSet::new();
    let invariants: BTreeSet<_> = model.invariants.iter().map(|i| i.id.clone()).collect();
    for assessment in scenarios {
        let scenario = &assessment.scenario;
        let mandatory = matches!(
            scenario.kind.as_str(),
            "success" | "failure" | "counterexample"
        );
        require(
            text(&assessment.scenario_id)
                && ids.insert(&assessment.scenario_id)
                && assessment.model_revision == run.spec_revision
                && SCENARIO_KINDS.contains(&scenario.kind.as_str())
                && text(&assessment.rationale)
                && assessment.outcome != ScenarioOutcome::Fail
                && (!mandatory || assessment.outcome == ScenarioOutcome::Pass),
            "model scenario is duplicated, stale, failed or missing mandatory simulation",
        )?;
        kinds.insert(scenario.kind.as_str());
        require(
            assessment.criterion_ids.is_subset(&task.criterion_ids),
            "model scenario invents a criterion",
        )?;
        if assessment.outcome == ScenarioOutcome::Pass {
            require(
                !assessment.criterion_ids.is_empty()
                    && text(&scenario.initial_state)
                    && !scenario.events.is_empty()
                    && scenario.events.iter().all(|e| text(e))
                    && text(&scenario.expected_result)
                    && scenario.invariant_ids == invariants,
                "model scenario lacks current criterion, state, events, result or invariant coverage",
            )?;
            covered.extend(assessment.criterion_ids.iter().cloned());
        }
    }
    require(
        kinds == SCENARIO_KINDS.into_iter().collect() && covered == task.criterion_ids,
        "model scenarios omit a design class or assigned criterion",
    )
}

pub fn validate_output(
    run: &Run,
    task: &TaskRecord,
    role: CatalogRole,
    output: &Output,
) -> Result<()> {
    require(
        output.contract_version == CONTRACT && text(&output.artifact),
        "task model contract missing or outdated",
    )?;
    require(
        !matches!(
            role,
            CatalogRole::ContextAnalyst
                | CatalogRole::Researcher
                | CatalogRole::DataAnalyst
                | CatalogRole::Writer
        ) || output.context.is_some(),
        "context role omitted its actual ContextModel",
    )?;
    require(
        !matches!(role, CatalogRole::Planner | CatalogRole::OptionsAnalyst)
            || output.solution.is_some(),
        "planning role omitted its actual SolutionModel",
    )?;
    if let Some(context) = &output.context {
        crate::verification::validate_context_model(run, &context.model)?;
        let facts: BTreeSet<_> = context
            .model
            .facts
            .iter()
            .map(|f| serde_json::to_string(f).unwrap())
            .collect();
        require(
            context.sections.facts().iter().all(|items| {
                !items.is_empty()
                    && items
                        .iter()
                        .all(|f| facts.contains(&serde_json::to_string(f).unwrap()))
            }),
            "ContextModel omits a context section or section facts are not in its sourced model",
        )?;
    }
    if let Some(solution) = &output.solution {
        let mut scoped = run.clone();
        scoped
            .spec
            .requirements
            .retain(|r| task.requirement_ids.contains(&r.id));
        crate::verification::validate_solution_model(&scoped, &solution.model)?;
        validate_scenarios(run, task, &solution.model, &solution.scenarios)?;
        let simulations: BTreeSet<_> = solution
            .model
            .simulations
            .iter()
            .map(|s| hash(serde_json::to_vec(s).unwrap()))
            .collect();
        require(
            solution
                .scenarios
                .iter()
                .filter(|s| s.outcome == ScenarioOutcome::Pass)
                .all(|s| simulations.contains(&hash(serde_json::to_vec(&s.scenario).unwrap()))),
            "executor scenario assessment does not inspect its retained model simulations",
        )?;
    }
    Ok(())
}

pub fn model_hashes(output: &Output) -> serde_json::Value {
    serde_json::json!({"context":output.context.as_ref().map(|v|hash(serde_json::to_vec(v).unwrap())),
        "solution":output.solution.as_ref().map(|v|hash(serde_json::to_vec(v).unwrap()))})
}

pub fn validate_review(
    run: &Run,
    artifact: &Artifact,
    quality: &crate::quality::QualityArtifacts,
) -> Result<Output> {
    let team = run
        .team
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("model review has no installed team".into()))?;
    let task = team
        .tasks
        .iter()
        .find(|t| {
            t.artifact
                .as_ref()
                .is_some_and(|a| a.id == artifact.id && a.session_id == artifact.session_id)
        })
        .ok_or_else(|| {
            crate::Error::Invalid("model review has no exact task artifact owner".into())
        })?;
    require(
        hash(&artifact.content) == artifact.hash
            && task.artifact.as_ref().is_some_and(|reference| {
                reference.content_hash == artifact.hash
                    && reference.spec_revision == artifact.spec_revision
                    && reference.attempt == artifact.attempt
            }),
        "model artifact bytes or identity differ from the task's immutable result",
    )?;
    let role = team
        .agents
        .iter()
        .find(|a| a.id == task.owner_id)
        .unwrap()
        .role;
    let output: Output = crate::transport::decode(artifact.content.as_bytes())?;
    validate_output(run, task, role, &output)?;
    if output.context.is_none() && output.solution.is_none() {
        require(
            quality.model_review.is_none(),
            "review claims inspection of nonexistent task models",
        )?;
        return Ok(output);
    }
    let review = quality
        .model_review
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("task models lack independent inspection".into()))?;
    let hashes = model_hashes(&output);
    require(
        review.context_hash.as_deref() == hashes["context"].as_str()
            && review.solution_hash.as_deref() == hashes["solution"].as_str(),
        "model review is bound to other executor models",
    )?;
    if output.context.is_some() {
        require(
            review.inspected_context_sections
                == CONTEXT_SECTIONS.into_iter().map(str::to_string).collect()
                && review
                    .context_counterexample
                    .as_ref()
                    .is_some_and(|s| text(s)),
            "context review omits sections or independent counterexample",
        )?;
        crate::verification::validate_context_model(
            run,
            review.independent_context.as_ref().ok_or_else(|| {
                crate::Error::Invalid(
                    "context review lacks independently sourced observations".into(),
                )
            })?,
        )?;
    } else {
        require(
            review.independent_context.is_none()
                && review.context_counterexample.is_none()
                && review.inspected_context_sections.is_empty(),
            "review invented context inspection",
        )?;
    }
    if let Some(solution) = &output.solution {
        crate::quality::validate_planning_binding(quality, Some(&solution.model))?;
        validate_scenarios(run, task, &solution.model, &review.independent_simulations)?;
        let simulations: BTreeSet<_> = solution
            .model
            .simulations
            .iter()
            .map(|s| hash(serde_json::to_vec(s).unwrap()))
            .collect();
        require(
            review
                .independent_simulations
                .iter()
                .filter(|s| s.outcome == ScenarioOutcome::Pass)
                .all(|s| !simulations.contains(&hash(serde_json::to_vec(&s.scenario).unwrap()))),
            "verifier copied executor simulations instead of independent counterexamples",
        )?;
    } else {
        require(
            review.independent_simulations.is_empty(),
            "review invented nonexistent solution simulations",
        )?;
    }
    Ok(output)
}
