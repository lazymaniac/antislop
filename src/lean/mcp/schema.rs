//! Small role-specific schemas keep process control out of task prompts.
use serde_json::{Value, json};

fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}

/// `addressed`: the open comments an orchestrator's answer settles.
pub(super) fn addressed() -> Value {
    json!({"type":"array","items":object(
        json!({"id":{"type":"integer","description":"The comment number"},"how":{"type":"string","description":"What the answer or revised plan changed for it"}}),
        &["id", "how"],
    ),"description":"Comments this answer settles; a comment you only answered stays open for the user"})
}

/// `ask_user`: a question with the choices the user picks from.
pub(super) fn question() -> Value {
    let choice = object(
        json!({"label":{"type":"string"},"description":{"type":"string","description":"What choosing it means"}}),
        &["label"],
    );
    json!({
        "context":{"type":"string","description":"Why the answer matters, in a few sentences"},
        "question":{"type":"string"},
        "options":{"type":"array","minItems":2,"maxItems":6,"items":choice}
    })
}

/// `select`: the options an options task weighed and the one it chose.
pub(super) fn selection() -> Value {
    let alternative = object(
        json!({"id":{"type":"string","description":"Short id, e.g. A"},"approach":{"type":"string"},"tradeoffs":{"type":"string","description":"What this approach costs or risks"}}),
        &["id", "approach", "tradeoffs"],
    );
    json!({
        "alternatives":{"type":"array","minItems":1,"maxItems":4,"items":alternative},
        "selected":{"type":"string","description":"The id of the chosen alternative"},
        "rationale":{"type":"string","description":"Technical reasoning and evidence; for one feasible alternative explain why the constraints exclude others"},
        "user_summary":{"type":"string","description":"Up to three short sentences in the user conversation language: chosen approach, user-visible benefit, and meaningful tradeoff. No internal task/check ids, review history or instructions to agents. Full technical detail stays in rationale and alternatives."}
    })
}

/// `review`: one entry per listed check and the problems found; the runtime
/// derives the verdict.
pub(super) fn review() -> Value {
    let check = object(
        json!({"status":{"enum":["ok","issue","blocked","n/a"]},"note":{"type":"string","description":"What you inspected and found. issue is a blocking contract failure; optional low findings leave a satisfied check ok. blocked means required evidence cannot be obtained; n/a needs a scope reason."}}),
        &["status", "note"],
    );
    let finding = object(
        json!({"check":{"type":"string","description":"The listed check id the problem breaks"},"severity":{"enum":["critical","high","medium","low"],"description":"critical, high and medium send the task back; low is non-blocking and must not alone mark a satisfied check issue"},"issue":{"type":"string"},"where":{"type":"string","description":"File and line, or the part of the report"}}),
        &["check", "severity", "issue", "where"],
    );
    json!({
        "report":{"type":"string","description":"What you reviewed and how"},
        "checks":{"type":"object","additionalProperties":check,"description":"Every check id the prompt lists, each with status and note"},
        "findings":{"type":"array","items":finding},
        "resolved":{"type":"array","items":{"type":"string"},"description":"Ids of your earlier findings, as the prompt lists them, that the updated work fixes wholly or in part; what remains of a partly fixed one is a new, narrower finding"}
    })
}

/// `review` of a plan, an audit or a code change: the problems found; the
/// runtime approves when none is critical, high or medium.
pub(super) fn plain_review() -> Value {
    let finding = object(
        json!({"severity":{"enum":["critical","high","medium","low"],"description":"critical, high and medium reject"},"issue":{"type":"string"},"where":{"type":"string","description":"File and line, task id, or the part of the request"}}),
        &["severity", "issue", "where"],
    );
    json!({
        "report":{"type":"string","description":"What you checked and how"},
        "findings":{"type":"array","items":finding,"description":"Empty when there is nothing to fix"}
    })
}

/// A task as the planner writes it; the runtime derives everything else.
pub(super) fn planned_task() -> Value {
    object(
        json!({
            "id":{"type":"string","description":"Short unique id, e.g. t1"},
            "title":{"type":"string"},
            "goal":{"type":"string","description":"What the task must achieve, in full sentences"},
            "kind":{"enum":KINDS},
            "depends_on":{"type":"array","items":{"type":"string"},"description":"Ids of tasks that must be approved first. With required gates enabled, generated gate ids such as quality-refactoring, static-analyzer and integration-gate are also valid without declaring them. When that kind already has explicitly declared checks, its canonical gate id refers to all of those checks. Each dependent receives dependency summaries"},

            "deferred":{"type":"boolean","description":"True keeps future work in the graph but runtime cannot schedule or launch it. Its options and code gates are deferred too. Release through a reviewed plan revision when its prerequisites are ready."},
            "change_kind":{"enum":["none","artifact","code"],"description":"none: report only; artifact: analysis/document in explicit outputs, no code gates; code: implementation with required code gates. Describes the result; all agents have full filesystem access."},
            "outputs":{"type":"array","items":{"type":"string"},"description":"Explicit relative output files, required for artifact"},
            "delivery":{"enum":["untracked"],"description":"Deliver a new artifact without modifying the project HEAD or index"},
            "acceptance_criteria":{"type":"array","minItems":1,"items":object(json!({"id":{"type":"string","description":"Task-local stable id, e.g. C1"},"text":{"type":"string"}}), &["id","text"])},
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
        "scope":{"type":"string","description":"Complete proposed scope in Markdown; omit to preserve the accepted scope. A change awaits user acceptance with the reviewed plan."},
        "reason":{"type":"string","description":"What the revision changes and why"},
        "tasks":{"type":"array","items":planned_task(),"description":"Changed or added tasks, each complete"},
        "retire":{"type":"array","items":{"type":"string"},"description":"Ids of tasks to drop; their obligations must move to another task"},
        "replacements":{"type":"object","additionalProperties":{"type":"string"},"description":"New task id -> retired local task name, or source_task UUID from unfinished_checkpoints for a code writer recovering an earlier plan. Historical sources must belong to this conversation and remain unapproved. The successor retains the report/checkpoint and starts with reassessment; approval never transfers. No historical task needs to be listed in retire."},
        "redo":{"type":"array","items":{"type":"string"},"description":"Ids of tasks whose existing work needs correction. Checkpoints remain preserved; reassess before continuing, never recreate integrated changes from scratch"}
    })
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
