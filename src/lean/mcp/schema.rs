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
        "alternatives":{"type":"array","minItems":2,"maxItems":4,"items":alternative},
        "selected":{"type":"string","description":"The id of the chosen alternative"},
        "rationale":{"type":"string","description":"Why it fits this task better than the others"}
    })
}

/// `review`: one entry per listed check and the problems found; the runtime
/// derives the verdict.
pub(super) fn review() -> Value {
    let check = object(
        json!({"status":{"enum":["ok","issue","blocked","n/a"]},"note":{"type":"string","description":"What you inspected and found"}}),
        &["status", "note"],
    );
    let finding = object(
        json!({"check":{"type":"string","description":"The listed check id the problem breaks"},"severity":{"enum":["critical","high","medium","low"],"description":"critical, high and medium send the task back"},"issue":{"type":"string"},"where":{"type":"string","description":"File and line, or the part of the report"}}),
        &["check", "severity", "issue", "where"],
    );
    json!({
        "report":{"type":"string","description":"What you verified and how"},
        "checks":{"type":"object","additionalProperties":check,"description":"Every check id the prompt lists, each with status and note"},
        "findings":{"type":"array","items":finding},
        "resolved":{"type":"array","items":{"type":"string"},"description":"Ids of your earlier findings, as the prompt lists them, that the updated work fixes"}
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

/// A requirement as the extraction analyst submits it.
pub(super) fn ledger_entry() -> Value {
    let criterion = object(
        json!({"id":{"type":"string","description":"Globally unique, e.g. R1.1"},"text":{"type":"string","description":"How acceptance is checked"},"mandatory":{"type":"boolean","description":"Omit for true"},"integration":{"type":"boolean","description":"Accepts the combined outcomes of the child requirements"}}),
        &["id", "text"],
    );
    object(
        json!({
            "id":{"type":"string","description":"Stable id, e.g. R1; existing requirements keep theirs"},
            "quote":{"type":"string","description":"The user's words this comes from, copied exactly; long enough to occur once in its source"},
            "source":{"type":"string","description":"The source id the quote is in; omit for the first source that has it"},
            "interpretation":{"type":"string","description":"The obligation in full sentences"},
            "qualifiers":{"type":"array","items":{"type":"string"},"description":"Exceptions, limits and conditions the user gave"},
            "parent":{"type":"string","description":"Id of the requirement this one refines"},
            "depends_on":{"type":"array","items":{"type":"string"}},
            "mandatory":{"type":"boolean","description":"Omit for true; false for a preference"},
            "criteria":{"type":"array","items":criterion},
            "task_ids":{"type":"array","items":{"type":"string"},"description":"Planned task ids, if a plan exists"}
        }),
        &["id", "quote", "interpretation", "criteria"],
    )
}

/// `submit_audit`: problems found and evidence per acceptance criterion.
pub(super) fn audit() -> Value {
    let finding = object(
        json!({"severity":{"enum":["critical","high","medium","low"],"description":"critical, high and medium reject"},"issue":{"type":"string"},"where":{"type":"string","description":"File, requirement or criterion id"}}),
        &["severity", "issue", "where"],
    );
    let evidence = object(
        json!({"criterion_id":{"type":"string"},"task_id":{"type":"string","description":"The plan's id of the task that delivered it"},"result":{"type":"string","description":"The concrete checks you made and what you observed"},"artifact":{"type":"string","description":"A file or command output that shows it"}}),
        &["criterion_id", "task_id", "result"],
    );
    json!({
        "findings":{"type":"array","items":finding,"description":"Empty when there is nothing to fix"},
        "evidence":{"type":"array","items":evidence}
    })
}

/// `submit_audit_review`: one assessment per proposed evidence id.
pub(super) fn audit_review() -> Value {
    let assessment = object(
        json!({"evidence":{"type":"string","description":"The proposed evidence id, e.g. E1"},"approved":{"type":"boolean"},"note":{"type":"string","description":"What you checked and observed"}}),
        &["evidence", "approved", "note"],
    );
    json!({
        "note":{"type":"string","description":"Coverage and gaps of the audit"},
        "assessments":{"type":"array","items":assessment}
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
            "scope":{"type":"array","items":{"type":"string"},"description":"Project paths the task works in"},
            "writes":{"type":"boolean","description":"Omit to follow the kind: implementation, refactoring, testing, static_analysis and integration write"},
            "covers":{"type":"array","items":{"type":"string"},"description":"Acceptance criterion ids (or requirement ids) from the requirements this task delivers"},
            "section":{"type":"string"},
            "summary":{"type":"string"}
        }),
        &["id", "title", "goal", "kind"],
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
        "reason":{"type":"string","description":"What the revision changes and why"},
        "tasks":{"type":"array","items":planned_task(),"description":"Changed or added tasks, each complete"},
        "retire":{"type":"array","items":{"type":"string"},"description":"Ids of tasks to drop; their obligations must move to another task"},
        "redo":{"type":"array","items":{"type":"string"},"description":"Ids of approved tasks whose work must be done again because their result is wrong. An approved task changed only in its title, goal or summary otherwise keeps its result"}
    })
}
