use super::*;
use serde_json::json;

fn verdict_with(coverage: serde_json::Map<String, serde_json::Value>) -> Verdict {
    Verdict {
        resolved: Vec::new(),
        approved: false,
        findings: vec![],
        coverage,
    }
}

/// A plan as `submit_plan` receives it, checked as the tool checks it.
fn submitted(plan: serde_json::Value) -> crate::Result<Vec<TaskContract>> {
    let tasks: Vec<PlannedTask> = serde_json::from_value(plan["tasks"].clone())?;
    let specs = planned_graph(tasks)?;
    validate(&specs)?;
    Ok(specs)
}

#[path = "tests/card_outcomes.rs"]
mod card_outcomes;
#[path = "tests/coverage.rs"]
mod coverage;
#[path = "tests/options_and_gates.rs"]
mod options_and_gates;
#[path = "tests/parsing_and_validation.rs"]
mod parsing_and_validation;
#[path = "tests/plans_and_messages.rs"]
mod plans_and_messages;
#[path = "tests/requirements_and_graph.rs"]
mod requirements_and_graph;
#[path = "tests/verdicts_and_bounds.rs"]
mod verdicts_and_bounds;
