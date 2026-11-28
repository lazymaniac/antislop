//! Small role-specific schemas keep process control out of task prompts.
use crate::lean::prompts;
use serde_json::{Value, json};

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}

/// `addressed`: the open comments an orchestrator's answer settles.
pub(super) fn addressed() -> Value {
    json!({"type":"array","items":object(
        json!({"id":{"type":"integer","description":prompts::ADDRESSED_ID},"how":{"type":"string","description":prompts::ADDRESSED_HOW}}),
        &["id", "how"],
    ),"description":prompts::ADDRESSED_COMMENTS_DESCRIPTION})
}

/// `ask_user`: a question with the choices the user picks from.
pub(super) fn question() -> Value {
    let choice = object(
        json!({"label":{"type":"string"},"description":{"type":"string","description":prompts::QUESTION_DESCRIPTION}}),
        &["label"],
    );
    json!({
        "context":{"type":"string","description":prompts::QUESTION_CONTEXT},
        "question":{"type":"string"},
        "options":{"type":"array","minItems":2,"maxItems":6,"items":choice}
    })
}

/// `select`: the options an options task weighed and the one it chose.
pub(super) fn selection() -> Value {
    let alternative = object(
        json!({"id":{"type":"string","description":prompts::SELECTION_ID},"approach":{"type":"string"},"tradeoffs":{"type":"string","description":prompts::SELECTION_TRADEOFFS}}),
        &["id", "approach", "tradeoffs"],
    );
    json!({
        "alternatives":{"type":"array","minItems":1,"maxItems":4,"items":alternative},
        "selected":{"type":"string","description":prompts::SELECTION_SELECTED},
        "ranking":{"type":"array","minItems":1,"maxItems":4,"uniqueItems":true,"items":{"type":"string"},"description":"Rank every alternative ID strongest first, including the only ID when one is viable."},
        "decisive_criterion":{"type":"string","description":"The task criterion or constraint that determined the selected option."},
        "ranking_override":{"type":"string","description":"Required when the chosen ID is below first place. Explain the concrete reason for overriding the ranking."},
        "rationale":{"type":"string","description":prompts::SELECTION_RATIONALE},
        "user_summary":{"type":"string","description":prompts::SELECTION_USER_SUMMARY}
    })
}

/// `review`: one entry per listed check and the problems found; the runtime
/// derives the verdict.
pub(super) fn review() -> Value {
    let check = object(
        json!({"status":{"enum":["ok","issue","blocked","n/a"]},"note":{"type":"string","description":prompts::REVIEW_NOTE},"validation":{"type":"string","description":"Passing validation receipt required for a criterion that inherited a reproduced finding through a plan revision."}}),
        &["status", "note"],
    );
    let scenario = object(
        json!({"scenario":{"type":"string"},"validation":{"type":"string"}}),
        &["scenario", "validation"],
    );
    let finding = object(
        json!({"reproduction":scenario.clone(),"check":{"type":"string","description":prompts::REVIEW_CHECK},"severity":{"enum":["critical","high","medium","low"],"description":prompts::REVIEW_SEVERITY},"issue":{"type":"string"},"where":{"type":"string","description":prompts::REVIEW_WHERE}}),
        &["check", "severity", "issue", "where"],
    );
    json!({
        "report":{"type":"string","description":prompts::REVIEW_REPORT},
        "checks":{"type":"object","additionalProperties":check,"description":prompts::REVIEW_CHECKS},
        "findings":{"type":"array","items":finding},
        "carry_findings":{"type":"array","items":object(json!({"id":{"type":"string"},"where":{"type":"string"}}), &["id"]),"description":"Retain an open finding by exact ID from previous_review. An optional corrected location keeps its identity and original evidence."},
        "resolution_evidence":{"type":"object","additionalProperties":scenario,"description":"For each resolved blocking reproduction, map its finding ID to the same scenario and a newer passing validation on the reviewed tree."},
        "resolved":{"type":"array","items":{"type":"string"},"description":prompts::REVIEW_RESOLVED_DESCRIPTION}
    })
}

/// `review` of a plan, an audit or a code change: the problems found; the
/// runtime approves when none is critical, high or medium.
pub(super) fn plain_review() -> Value {
    let finding = object(
        json!({"severity":{"enum":["critical","high","medium","low"],"description":prompts::PLAIN_REVIEW_SEVERITY},"issue":{"type":"string"},"where":{"type":"string","description":prompts::PLAIN_REVIEW_WHERE}}),
        &["severity", "issue", "where"],
    );
    json!({
        "report":{"type":"string","description":prompts::PLAIN_REVIEW_REPORT},
        "findings":{"type":"array","items":finding,"description":prompts::PLAIN_REVIEW_FINDINGS}
    })
}

/// A task as the planner writes it; the runtime derives everything else.
pub(super) fn planned_task() -> Value {
    object(
        json!({
            "id":{"type":"string","description":prompts::PLANNED_TASK_ID},
            "title":{"type":"string"},
            "goal":{"type":"string","description":prompts::PLANNED_TASK_GOAL},
            "kind":{"enum":KINDS},
            "depends_on":{"type":"array","items":{"type":"string"},"description":prompts::PLANNED_TASK_DEPENDENCIES_DESCRIPTION},

            "result_inputs":{"type":"array","items":object(json!({"alias":{"type":"string"},"result_ref":{"type":"string","description":prompts::PLANNED_TASK_RESULT_REF}}), &["alias","result_ref"])},
            "planning_inputs":{"type":"array","items":{"type":"string"},"description":"Exact planning report references returned by consult_planning or the retained planning index. List reports this role must be able to read."},
            "open_decisions":{"type":"array","items":{"type":"string","minLength":1},"description":"Unresolved task-local choices worth a separate Options turn. Omit for an already decided implementation."},
            "deferred":{"type":"boolean","description":prompts::PLANNED_TASK_DEFERRED},
            "change_kind":{"enum":["none","artifact","code"],"description":prompts::PLANNED_TASK_CHANGE_KIND},
            "outputs":{"type":"array","items":{"type":"string"},"description":prompts::PLANNED_TASK_OUTPUTS_DESCRIPTION},
            "delivery":{"enum":["untracked"],"description":prompts::PLANNED_TASK_DELIVERY},
            "acceptance_criteria":{"type":"array","minItems":1,"items":object(json!({"id":{"type":"string","description":prompts::ACCEPTANCE_CRITERION_ID_DESCRIPTION},"text":{"type":"string"}}), &["id","text"])},
            "section":{"type":"string"},
            "summary":{"type":"string"}
        }),
        &[
            "id",
            "title",
            "goal",
            "kind",
            "change_kind",
            "acceptance_criteria",
        ],
    )
}

/// The work kinds a planned task may have; orchestration belongs to the
/// runtime's own role.
const KINDS: [&str; 18] = [
    "context",
    "requirements",
    "options",
    "planning",
    "architecture",
    "dependencies",
    "implementation",
    "refactoring",
    "static_analysis",
    "test_design",
    "testing",
    "security",
    "performance",
    "integration",
    "research",
    "writing",
    "analysis",
    "delivery",
];

/// The properties of `revise_plan`; the plan and its revision are the ones
/// the orchestrator was shown.
pub(super) fn revision() -> Value {
    json!({
        "base_revision":{"type":"integer","minimum":0,"description":"Accepted revision shown in task_states. Required for patches."},
        "scope":{"type":"string","description":prompts::REVISION_SCOPE},
        "reason":{"type":"string","description":prompts::REVISION_REASON},
        "tasks":{"type":"array","items":planned_task(),"description":prompts::REVISION_TASKS},
        "patches":{"type":"array","items":object(json!({
            "task":{"type":"string"},
            "goal":{"type":"string"},
            "depends_on":{"type":"array","items":{"type":"string"}},
            "open_decisions":{"type":"array","items":{"type":"string","minLength":1},"description":"Explicitly [] to close all decisions for this task. Omit to preserve."},
            "criteria":{"type":"object","additionalProperties":{"type":"string"},"description":"Replacement spelt by existing criterion id."},
            "add_criteria":{"type":"array","items":object(json!({"id":{"type":"string"},"text":{"type":"string"}}), &["id","text"])}
        }), &["task"])},
        "witnesses":{"type":"array","items":plan_witness(),"description":"Optional quantitative assertions checked before independent review. Supply expected match counts."},
        "finding_actions":{"type":"array","items":object(json!({
            "task":{"type":"string"},"finding":{"type":"string"},"reason":{"type":"string"},
            "destination":object(json!({"task":{"type":"string"},"criterion":{"type":"string"}}), &["task","criterion"])
        }), &["task","finding","reason"]),"description":"Require independent review to supersede an invalid reproduction or transfer its proof to a named criterion on another task. Omit destination to supersede."},
        "retire":{"type":"array","items":{"type":"string"},"description":prompts::REVISION_RETIRE_DESCRIPTION},
        "replacements":{"type":"object","additionalProperties":{"type":"string"},"description":prompts::REVISION_REPLACEMENTS_DESCRIPTION},
        "redo":{"type":"array","items":{"type":"string"},"description":prompts::REVISION_REDO_DESCRIPTION}
    })
}

pub(super) fn plan_witness() -> Value {
    let commit = json!({"type":"string","description":"Full immutable Git commit ID."});
    let expected = json!({"type":"integer","minimum":0});
    json!({"oneOf":[
        object(json!({
            "kind":{"enum":["search"]},"commit":commit,"pattern":{"type":"string"},
            "paths":{"type":"array","items":{"type":"string"}},"expected":expected,"expected_files":expected
        }), &["kind","commit","pattern"]),
        object(json!({
            "kind":{"enum":["test_filter"]},"commit":commit,"discovery":{"type":"string"},
            "selector":{"type":"string"},"exact":{"type":"boolean"},"expected":expected
        }), &["kind","commit","discovery","selector"])
    ]})
}

#[cfg(test)]
mod filesystem_tests {
    use super::*;

    #[test]
    fn tasks_expose_outputs_and_dependencies_without_path_allowlists() {
        let schema = planned_task();
        assert!(schema["properties"].get("scope").is_none());
        assert!(schema["properties"].get("outputs").is_some());
        assert!(schema["properties"].get("depends_on").is_some());
    }
}
