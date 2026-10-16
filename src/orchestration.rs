//! Machine-owned facts about the supported fixed-stage workflow.
use crate::{Result, domain::*, hash, id, quality};
use serde_json::{Value, json};
pub mod planning;

pub fn context(run: &Run) -> Result<Value> {
    let catalog = quality::catalog();
    let mut stages = Vec::new();
    let mut previous: Option<Stage> = None;
    for stage in [
        Stage::Discovery,
        Stage::Planning,
        Stage::Execution,
        Stage::Validation,
        Stage::Integration,
        Stage::Delivery,
    ] {
        let competencies: std::collections::BTreeSet<_> = catalog
            .iter()
            .filter(|check| {
                check.stages.contains(&stage)
                    && check
                        .family
                        .is_none_or(|family| family == run.spec.task_family)
            })
            .map(|check| check.role.clone())
            .collect();
        stages.push(json!({"stage":stage,"executor_role":format!("{stage:?}:executor"),
            "verifier_role":format!("{stage:?}:verifier"),"competencies":competencies,
            "depends_on_stage":previous,"allocation":"fixed runtime routing; future native sessions are not yet allocated"}));
        previous = Some(stage);
    }
    let sessions:Vec<_>=run.session_ledger.sessions.values().map(|session|json!({
        "id":session.id,"scope":session.scope,"native_session_id":session.native_session_id,
        "native_observed":session.native_observed,"state":session.state,"current_turn_id":session.current_turn_id
    })).collect();
    let turns:Vec<_>=run.session_ledger.turns.values().map(|turn|json!({
        "id":turn.id,"session_id":turn.session_id,"state":turn.state,"provider_completed":turn.provider_completed,
        "prompt_hash":turn.prompt_hash,"schema_hash":turn.schema_hash,"output_ref":turn.output_ref
    })).collect();
    let risk_catalog: Value = serde_json::from_str(include_str!("../fixtures/risks.json"))?;
    let risks: Vec<_> = run
        .risk_coverage
        .values()
        .map(|risk| {
            json!({
                "risk_id":risk.risk_id,"stage":risk.stage,"executor_role":risk.executor_role,
                "verifier_role":risk.verifier_role,"check_id":risk.check_id,"status":risk.status
            })
        })
        .collect();
    let mut context = json!({"schema_version":1,"workflow":"fixed six-stage supervised pipeline",
        "run_id":run.id,"spec_revision":run.spec_revision,"invocation_stage":run.stage,"attempt":run.attempt,
        "project_instructions":run.project_instructions()?,
        "checklist_version":run.checklist_version,"risk_catalog_version":risk_catalog["version"],
        "stage_assignments":stages,"risk_coverage":risks,"approvals":run.approvals,
        "prior_risk_binding_status":crate::verification::prior_risk_binding_summary(run),
        "prior_risk_binding_status_scope":"Exact backend authentication of earlier-stage risk evidence bindings at the supplied scope and inputs only. This is not quality acceptance, independent semantic review, result acceptance, or a waiver of expected checks/gates. Current and future stage risks are excluded; zero checked risks means there are no earlier-stage obligations in this observation.",
        "sessions":sessions,"turns":turns,
        "diff_comments":run.diff_comments,
        "diff_reviews":run.diff_reviews.iter().map(|r| serde_json::json!({
            "id":r.id,"state":r.state,"spec_revision":r.spec_revision,
            "diff_hash":r.diff_hash,"input_hash":r.input_hash,"profile":r.profile,
            "session_id":r.session_id,"response":r.response,"diagnostic":r.diagnostic,
            "scope":"Independent candidate advice; no task or stage acceptance"
        })).collect::<Vec<_>>(),
        "unresolved_diff_comments":run.diff_comments.iter().filter(|c| !c.resolved).collect::<Vec<_>>(),
        "policy":{"configuration":run.spec.config,"executor_edits_only_in":"execution",
            "editable_files":run.spec.editable_files,"check_command":run.spec.check_command,
            "verifier_edits_allowed":false,"native_delegation_allowed":false,
            "start_and_repair":"explicit current-scope approval before Advance; attempt increments after approval",
            "result_acceptance":"independent validated review and explicit current-scope acceptance",
            "source_refresh":"After an authorized source edit, fresh context/model executor-verifier pairs are subordinate work within the approved implementation attempt, cannot edit files, and do not claim separate stage approvals. Their invocation stage differs from the primary runtime stage in the orchestration receipt.",
            "external_actions":"not granted by stage or repair approval"},
        "independence_contract":"Every invocation reserves a fresh logical session. The CLI native session is recorded on observation; canonical provider/native UUID reuse across logical sessions is rejected before output acceptance. The current verifier native ID is not yet known when its prompt is prepared; do not claim it was observed in advance.",
        "limitations":["This is the existing fixed-stage runtime, not execution of the standalone TeamPlan scheduler.","Future allocations and unassessed risks are pending, not completed work.","Native CLI support and OS containment limitations remain explicit; stage approvals do not authorize external project actions."] });
    if let Some(observation) = &run.input_observation {
        context["input_observation"] = serde_json::to_value(observation)?;
    }
    Ok(context)
}

pub fn observe(run: &Run, artifact_hash: &str) -> Result<Evidence> {
    let output = serde_json::to_string(&context(run)?)?;
    Ok(Evidence {
        id: id(),
        artifact_hash: artifact_hash.into(),
        spec_revision: run.spec_revision,
        source: "runtime:orchestration".into(),
        output_hash: hash(&output),
        output,
        success: true,
        input_hash: Some(hash(serde_json::to_vec(&run.input_snapshot)?)),
    })
}
