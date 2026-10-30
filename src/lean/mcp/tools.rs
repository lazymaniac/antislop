use super::gateway::{ASK, ASK_USER, REVIEW, SUBMIT_PLAN, SUBMIT_REQUIREMENTS, TOOL, WAIT};
use super::schema;
use serde_json::{Value, json};

type ToolSpec = (String, Value, Vec<&'static str>);

/// The roles that review plans, audits, or code changes without check coverage.
fn plain_reviewer(role: &str) -> bool {
    matches!(role, "plan-verifier" | "auditor") || role.starts_with("code-review:")
}

pub(crate) fn allowed_tools(role: &str) -> Vec<&'static str> {
    if let Some((_, verifier)) = crate::lean::task_role(role) {
        return task_tools(verifier);
    }
    match role {
        "orchestrator" | "comment-orchestrator" => orchestrator_tools(),
        "planner" => vec![SUBMIT_PLAN, TOOL],
        "requirements-extraction-analyst" => vec!["submit_ledger"],
        "requirements-audit-analyst" => vec!["submit_audit"],
        "requirements-audit-verifier" => vec!["submit_audit_review"],
        _ if plain_reviewer(role) => vec![REVIEW],
        _ => vec![TOOL],
    }
}

fn task_tools(verifier: bool) -> Vec<&'static str> {
    if verifier {
        vec![REVIEW, ASK, WAIT]
    } else {
        vec!["complete", "select", ASK, WAIT]
    }
}

fn orchestrator_tools() -> Vec<&'static str> {
    vec![
        TOOL,
        ASK_USER,
        SUBMIT_REQUIREMENTS,
        SUBMIT_PLAN,
        "answer_agent",
        "defer_to_user",
        "retry_task",
        "revise_plan",
        "request_replan",
    ]
}

const RETRY: &str = "Stop only after recorded=true; if rejected, fix what the error names and call again within the remaining turn time.";

pub(crate) fn tool_list(role: &str) -> Vec<Value> {
    let orchestrator = matches!(role, "orchestrator" | "comment-orchestrator");
    allowed_tools(role)
        .iter()
        .map(|name| tool_json(name, role, orchestrator))
        .collect()
}

fn tool_json(name: &str, role: &str, orchestrator: bool) -> Value {
    let (description, fields, required) = tool_contract(name, role, orchestrator);
    let mut properties = fields.as_object().unwrap().clone();
    properties.insert("turn".into(), json!({"type":"string"}));
    json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})
}

fn tool_contract(name: &str, role: &str, orchestrator: bool) -> ToolSpec {
    match name {
        "submit_plan" | "revise_plan" => plan_contract(name, orchestrator),
        "submit_ledger"
        | "submit_audit"
        | "submit_audit_review"
        | "answer_agent"
        | "defer_to_user"
        | "retry_task"
        | "request_replan" => control_contract(name),
        _ => turn_contract(name, role, orchestrator),
    }
}

fn turn_contract(name: &str, role: &str, orchestrator: bool) -> ToolSpec {
    match name {
        "finish_turn" if orchestrator => (format!("End the turn with your answer to the user. {RETRY}"), json!({"text":{"type":"string","description":"Your answer, in Markdown"},"addressed":schema::addressed()}), vec!["turn", "text"]),
        "finish_turn" if role == "planner" => (format!("Only after submit_plan refused a plan of this turn and you cannot correct what it names, for instance because the instructions demand what it refuses: end the turn without a plan and name the blocker; the user sees it at once. {RETRY}"), json!({"text":{"type":"string","description":"What blocks the plan and what would unblock it, in Markdown"}}), vec!["turn", "text"]),
        "finish_turn" => (format!("End the turn with your answer. {RETRY}"), json!({"text":{"type":"string","description":"Your answer, in Markdown"}}), vec!["turn", "text"]),
        "ask_user" => (format!("Ask the user one question with 2–6 choices; this ends your turn and the answer arrives as the user's next message. {RETRY}"), schema::question(), vec!["turn", "context", "question", "options"]),
        "submit_requirements" => (format!("Interview only: end the turn with the requirements summary for the user to approve: the goal and its acceptance criteria. {RETRY}"), json!({"markdown":{"type":"string","description":"The summary in Markdown"}}), vec!["turn", "markdown"]),
        "complete" => (format!("Finish your task once its whole goal is done, not after a part of it; this ends your turn. {RETRY}"), json!({"summary":{"type":"string","description":"A few sentences on what was delivered; tasks that depend on this one receive only this"},"report":{"type":"string","description":"The full result your verifier checks: what you did, where, and how you checked it"}}), vec!["turn", "summary", "report"]),
        "select" => (format!("Options tasks only: record the approach you chose among 2–4 alternatives; this ends your turn. {RETRY}"), schema::selection(), vec!["turn", "alternatives", "selected", "rationale"]),
        "review" if plain_reviewer(role) => (format!("Record your review; this ends your turn. The runtime approves when no finding is critical, high or medium. {RETRY}"), schema::plain_review(), vec!["turn", "report", "findings"]),
        "review" => (format!("Record your review of the task; this ends your turn. The runtime approves the task when every check is ok or n/a and no finding is critical, high or medium. {RETRY}"), schema::review(), vec!["turn", "report", "checks", "findings"]),
        "ask" => ("Ask the orchestrator a question about your task without ending your turn. Waits up to 25 seconds and returns the answer if one came; asking the same question again returns its state.".into(), json!({"question":{"type":"string"}}), vec!["turn", "question"]),
        "wait_for_answer" => ("End your turn until the question you asked is answered; your work so far is kept and the task resumes with the answer.".into(), json!({"note":{"type":"string","description":"Where you stopped, for when the answer arrives"}}), vec!["turn"]),
        _ => (format!("End the turn with your answer. {RETRY}"), json!({"text":{"type":"string","description":"Your answer, in Markdown"}}), vec!["turn", "text"]),
    }
}

fn plan_contract(name: &str, orchestrator: bool) -> ToolSpec {
    if name == "submit_plan" {
        let mut fields = json!({"document":{"type":"string","description":"The plan for the user in Markdown: approach, task order and what each task delivers"},"tasks":{"type":"array","minItems":1,"items":schema::planned_task()},"keep":{"type":"array","items":{"type":"string"},"description":"Ids of approved tasks of the plan that ran whose accepted results this plan keeps unchanged; they do not run again. Leave them out of tasks: a task in tasks runs again from the start"}});
        if orchestrator {
            fields["addressed"] = schema::addressed();
        }
        return ("Submit the complete task plan; this ends your turn. The runtime checks it at once: if it is rejected, fix what the error names and submit the whole plan again within the remaining turn time. Stop only after recorded=true.".into(), fields, vec!["turn", "document", "tasks"]);
    }
    ("Change the running plan you were shown. Give only the tasks that change or are added, in full; the others stay as they are. The revision is reviewed independently before it applies.".into(), schema::revision(), vec!["turn", "reason", "tasks", "retire"])
}

fn control_contract(name: &str) -> ToolSpec {
    match name {
        "submit_ledger" => (format!("Submit the requirements you extracted; this ends your turn. The runtime finds each quote in the sources and checks the requirements at once. {RETRY}"), json!({"entries":{"type":"array","items":schema::ledger_entry()}}), vec!["turn", "entries"]),
        "submit_audit" => (format!("Submit the audit; this ends your turn. It approves unless a finding is critical, high or medium. {RETRY}"), schema::audit(), vec!["turn", "findings", "evidence"]),
        "submit_audit_review" => (format!("Submit your review of the audit; this ends your turn. Assess every proposed evidence id. {RETRY}"), schema::audit_review(), vec!["turn", "note", "assessments"]),
        "answer_agent" => ("Answer a task agent's pending question. Answers cannot change contracts or permissions.".into(), json!({"request":{"type":"string","description":"The request id the agent's question message names"},"answer":{"type":"string"}}), vec!["turn", "request", "answer"]),
        "defer_to_user" => ("Hand a task agent's question to the user when only the user can decide; the task waits and you pass the user's answer on with answer_agent.".into(), json!({"request":{"type":"string","description":"The request id the agent's question message names"},"explanation":{"type":"string","description":"What the user must decide and why, for the user"}}), vec!["turn", "request", "explanation"]),
        "retry_task" => ("Run a failed task again in its own worktree, keeping its changes, when it failed for a reason outside its contract (a turn limit, a crashed CLI). A stalled task needs revise_plan instead. At most twice per task; then ask the user.".into(), json!({"task":{"type":"string","description":"The task id in the plan"},"guidance":{"type":"string","description":"What the worker should know, such as checking its worktree before starting over"}}), vec!["turn", "task", "guidance"]),
        "request_replan" => ("Ask the planner to redraft the whole plan when contract repairs are not enough. It is reviewed like any new plan.".into(), json!({"reason":{"type":"string","description":"What the current plan gets wrong and what the new plan must change"}}), vec!["turn", "reason"]),
        _ => ("Change the running plan you were shown.".into(), schema::revision(), vec!["turn", "reason", "tasks", "retire"]),
    }
}
