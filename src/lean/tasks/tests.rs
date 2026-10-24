use super::*;
use serde_json::json;

fn verdict_with(coverage: serde_json::Map<String, serde_json::Value>) -> Verdict {
    Verdict {
        approved: false,
        findings: vec![],
        coverage,
    }
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
