use super::gateway::{ASK, ASK_USER, REVIEW, SUBMIT_PLAN, SUBMIT_REQUIREMENTS, TOOL, WAIT};
use super::schema;
use serde_json::{Value, json};

type ToolSpec = (String, Value, Vec<&'static str>);

/// The roles that review plans, audits, or code changes without check coverage.
fn plain_reviewer(role: &str) -> bool {
    matches!(role, "plan-reviewer" | "auditor") || role.starts_with("code-review:")
}

pub(crate) fn allowed_tools(role: &str) -> Vec<&'static str> {
    if let Some((_, reviewer)) = crate::lean::task_role(role) {
        return task_tools(reviewer);
    }
    match role {
        "orchestrator" | "comment-orchestrator" => orchestrator_tools(),
        "planner" => vec![SUBMIT_PLAN, TOOL, "read_evidence"],
        "requirements-extraction-analyst" => vec!["submit_ledger"],
        "requirements-audit-analyst" => {
            vec!["submit_audit", "read_dependency_result", "read_evidence"]
        }
        "requirements-audit-reviewer" => vec![
            "submit_audit_review",
            "read_dependency_result",
            "read_evidence",
        ],
        _ if plain_reviewer(role) => vec![REVIEW, "read_evidence"],
        _ => vec![TOOL],
    }
}

pub(super) fn scoped_tools(
    role: &str,
    contract: Option<&crate::lean::tasks::TaskContract>,
) -> Vec<&'static str> {
    let mut names = allowed_tools(role);
    if crate::lean::task_role(role).is_some_and(|(_, reviewer)| !reviewer)
        && let Some(task) = contract
    {
        names.retain(|name| {
            if task.kind == crate::config::WorkKind::Options {
                *name != "complete"
            } else {
                *name != "select"
            }
        });
    }
    names
}

pub(super) fn scoped_tool_list(
    role: &str,
    contract: Option<&crate::lean::tasks::TaskContract>,
) -> Vec<Value> {
    let names = scoped_tools(role, contract);
    let mut list = tool_list(role);
    list.retain(|tool| {
        tool["name"]
            .as_str()
            .is_some_and(|name| names.contains(&name))
    });
    if let Some(task) = contract
        && let Some(review) = list.iter_mut().find(|tool| tool["name"] == "review")
    {
        let checks = &mut review["inputSchema"]["properties"]["checks"];
        let check_schema = checks["additionalProperties"].clone();
        let obligations = crate::lean::tasks::obligations(task);
        checks["properties"] = obligations
            .iter()
            .map(|(id, text)| {
                let mut schema = check_schema.clone();
                schema["description"] = text.clone().into();
                (id.clone(), schema)
            })
            .collect::<serde_json::Map<_, _>>()
            .into();
        checks["required"] = json!(obligations.iter().map(|(id, _)| id).collect::<Vec<_>>());
        checks["additionalProperties"] = false.into();
    }
    list
}

fn task_tools(reviewer: bool) -> Vec<&'static str> {
    if reviewer {
        vec![
            REVIEW,
            ASK,
            WAIT,
            "read_dependency_result",
            "read_evidence",
            "current_turn",
        ]
    } else {
        vec![
            "complete",
            "select",
            "report_blocker",
            ASK,
            WAIT,
            "read_dependency_result",
            "record_evidence",
            "read_evidence",
            "current_turn",
        ]
    }
}

fn orchestrator_tools() -> Vec<&'static str> {
    vec![
        TOOL,
        ASK_USER,
        SUBMIT_REQUIREMENTS,
        SUBMIT_PLAN,
        "read_dependency_result",
        "read_evidence",
        "answer_agent",
        "defer_to_user",
        "retry_task",
        "resume_task",
        "revise_plan",
        "request_replan",
    ]
}

const RETRY: &str = "Use the exact turn UUID from the current prompt, never a session, task or plan id. Stop only after recorded=true; if rejected, fix what the error names and call again within the remaining turn time.";

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
    if matches!(name, "complete" | "select") {
        properties.insert("evidence".into(),json!({"type":"array","maxItems":64,"items":{"type":"string"},"description":"Optional existing MCP evidence ids to retain in this result. Evidence recorded this turn is attached automatically; use this for earlier evidence when amending a report."}));
    }
    if name != "current_turn" {
        let description = if crate::lean::task_role(role).is_some() {
            "Exact active turn id from the latest prompt; call current_turn if uncertain. Never reuse a previous turn id."
        } else {
            "Exact active turn id from the latest prompt. Never reuse a previous turn id."
        };
        properties.insert(
            "turn".into(),
            json!({"type":"string","description":description}),
        );
    }
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
        | "resume_task"
        | "request_replan" => control_contract(name),
        _ => turn_contract(name, role, orchestrator),
    }
}

fn turn_contract(name: &str, role: &str, orchestrator: bool) -> ToolSpec {
    match name {
        "current_turn" => ("Read this gateway's current task role, active turn id and allowed tools. Call with an empty object after any stale-turn rejection; copy the returned id exactly. This never ends a turn and grants no access to another agent.".into(), json!({}), vec![]),
        "report_blocker" => ("End a worker turn when missing input or an unsatisfied prerequisite prevents all further permitted work. Record the concrete external blocker, evidence, and actionable condition for resuming. An implementation, test, fixture or output this task is assigned to create is work to perform, not a missing prerequisite; do not ask another actor to supply your own deliverable or block merely because that work is unfinished or large. Preserve completed work and checkpoints. The task stays unapproved and its dependents do not run; the orchestrator must resolve the blocker or revise the contract. Do not claim success or repeat a question already answered without new evidence.".into(), json!({"report":{"type":"string"},"resume_condition":{"type":"string","description":"An actionable prerequisite another actor can resolve, with evidence; never completion of this task's own assigned deliverable"},"unblocked_work_remaining":{"type":"boolean","description":"True if ANY independent permitted edits or analysis remain, even when a later validation is blocked. A true value is rejected so you can continue that work within this turn. Report a blocker only after no permitted independent work remains."}}), vec!["turn", "report", "resume_condition", "unblocked_work_remaining"]),
        "record_evidence" => ("Persist raw command/measurement output as immutable MCP execution evidence before completing. Supply the actual output, not a reconstruction or repository source contents. Disk-backed content survives worktree cleanup. Up to 256 KiB UTF-8 per labelled record; split larger outputs into distinct labels. Repeating a label with identical text returns the same id; different text needs a new label. Every record is attached to your completion automatically.".into(),json!({"label":{"type":"string","maxLength":160},"text":{"type":"string","maxLength":262144}}),vec!["turn","label","text"]),
        "read_evidence" => ("Read immutable MCP execution evidence recorded in this conversation, in pages of at most 16 KiB. Discover ids in read_dependency_result.evidence, the report's MCP evidence section, or task instructions. Reuse cursor until complete=true. This never ends the turn and does not grant filesystem access.".into(),json!({"evidence":{"type":"string"},"cursor":{"type":"string"}}),vec!["turn","evidence"]),
        "read_dependency_result" => ("Read complete scoped worker reports in pages up to 16 KiB. dependency=self reads your own earlier worker results (also for reviewers). The orchestrator may inspect unapproved and blocked producers within its bound plan; task_status/task_approved remain explicit and inspection never approves the task or releases its dependents. available_rounds contains current-input rounds. report_history lists immutable result_ref values including blocked reports and older input generations. result_type=blocked is an unapproved blocker report, never a successful result; self may read it without completing or approving the task. available_rounds includes completed reports only: to read an older report, pass its exact result_ref. Historical=true means obsolete inputs: preserve the evidence but do not use it to claim current acceptance. Amendments never replace the original; read the earlier report and apply later corrections. Evidence metadata provides ids for read_evidence. This does not end the turn.".into(), json!({"dependency":{"type":"string","description":"Dependency name, scoped producer task id, or self"},"result_ref":{"type":"string","description":"Exact immutable reference from report_history; enables explicitly marked historical-input reports"},"round":{"type":"integer","minimum":0,"description":"Optional current-input worker round; when result_ref is present it must match"},"cursor":{"type":"string"}}), vec!["turn", "dependency"]),
        "finish_turn" if orchestrator => (format!("End the turn with your answer to the user. {RETRY}"), json!({"text":{"type":"string","description":"Your answer, in Markdown"},"addressed":schema::addressed()}), vec!["turn", "text"]),
        "finish_turn" if role == "planner" => (format!("Only after submit_plan or the plan reviewer refused a plan and you cannot correct it within the agreed requirements: end the turn without a plan and name the blocker and decision needed; the orchestrator receives it. {RETRY}"), json!({"text":{"type":"string","description":"What blocks the plan and what would unblock it, in Markdown"}}), vec!["turn", "text"]),
        "finish_turn" => (format!("End the turn with your answer. {RETRY}"), json!({"text":{"type":"string","description":"Your answer, in Markdown"}}), vec!["turn", "text"]),
        "ask_user" => (format!("Ask the user one question with 2–6 choices; this ends your turn and the answer arrives as the user's next message. {RETRY}"), schema::question(), vec!["turn", "context", "question", "options"]),
        "submit_requirements" => (format!("Interview only: end the turn with the requirements summary for the user to approve: the goal and its acceptance criteria. {RETRY}"), json!({"markdown":{"type":"string","description":"The summary in Markdown"}}), vec!["turn", "markdown"]),
        "complete" => (format!("Finish your task once its whole goal is done, not after a part of it; this ends your turn. {RETRY}"), json!({"summary":{"type":"string","description":"A few sentences on what was delivered; tasks that depend on this one receive only this"},"report":{"type":"string","description":"The full result your reviewer checks: what you did, where, and how you checked it"}}), vec!["turn", "summary", "report"]),
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
        let mut fields = json!({"document":{"type":"string","description":"The plan for the user in Markdown: approach, task order and what each task delivers"},"tasks":{"type":"array","minItems":1,"items":schema::planned_task()},"keep":{"type":"array","items":{"type":"string"},"description":"Ids of approved tasks of the plan that ran whose accepted results this plan keeps unchanged; they do not run again. Leave them out of tasks: a redeclared task retains existing work and is assessed before executor continuation; use revise_plan.replacements for unfinished successors"}});
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
        "answer_agent" => ("Answer a task agent's pending question. Answers cannot change contracts or permissions.".into(), json!({"request":{"type":"string","description":"The request id the agent's question message names"},"answer":{"type":"string"},"disposition":{"enum":["continue","wait_for_revision","wait_for_dependency","escalate_to_user"]},"target":{"type":"string","description":"Full revision request or dependency task UUID required for waiting dispositions"}}), vec!["turn", "request", "answer"]),
        "defer_to_user" => ("Hand a task agent's question to the user when only the user can decide; the task waits and you pass the user's answer on with answer_agent.".into(), json!({"request":{"type":"string","description":"The request id the agent's question message names"},"explanation":{"type":"string","description":"What the user must decide and why, for the user"}}), vec!["turn", "request", "explanation"]),
        "retry_task" => ("Run a failed task again in its own worktree, keeping its changes, when it failed for a reason outside its contract (a turn limit, a crashed CLI). Use only current task_states names or runtime_ids. This operates on this turn's accepted plan, never a newer draft. Use resume_task for an already resolved coordinating blocker under the unchanged contract; contract changes need revise_plan. Shares a limit of two recoveries per task; the default edits_only mode asks one bound edit/analysis-only recovery question at that limit. Select scope=existing_authorized_execution for requested validation: it ALWAYS asks one explicit, bound recovery decision and requires current registered execution authority for the exact task/contract/generation. It cannot create grants, reset budgets, permit retries or release unavailable/deferred steps. Do not first ask a generic ask_user validation question: use this control to bind the actual scope before the user chooses. Wait for the answer and do not change the contract.".into(), json!({"task":{"type":"string","description":"The task id in the plan"},"guidance":{"type":"string","description":"What the worker should know, such as checking its worktree before starting over"},"scope":{"enum":["edits_only","existing_authorized_execution"],"description":"Default edits_only; existing_authorized_execution always requires a new explicit bound user choice and preserves existing authorizations and budgets"}}), vec!["turn", "task", "guidance"]),
        "resume_task" => ("Continue a non-deferred coordinating task when its blocker is resolved OR independent authorized work remains that the blocker does not prevent, within its UNCHANGED contract. Read its full blocker report and actual evidence first; explain the resolution or exactly which independent work can proceed, and direct the worker to its next concrete change. Keep any unresolved later steps explicitly blocked. Its own unfinished implementation is assigned work, not a prerequisite another producer must supply. Keeps the worktree, contract, input generation and original execution authorization; NEVER renews/reset budgets or permits retrying failed experiments. Do not bypass pending user decisions or still-missing inputs, start dependent steps without prerequisites, restart active tasks or release future/deferred work. Contract/dependency changes require revise_plan. Shares retry_task's limit of two recoveries per task; the default edits_only mode asks one bound edit/analysis-only recovery question at that limit. Select scope=existing_authorized_execution for requested validation: it ALWAYS asks one explicit, bound recovery decision and requires current registered execution authority for the exact task/contract/generation. It cannot create grants, reset budgets, permit retries or release unavailable/deferred steps. Do not first ask a generic ask_user validation question: use this control to bind the actual scope before the user chooses. Wait for the answer and do not change the contract.".into(),json!({"task":{"type":"string","description":"Current accepted-plan local name or runtime task UUID"},"round":{"type":"integer","minimum":0,"description":"Exact round from current task_states"},"input_generation":{"type":"integer","minimum":0,"description":"Exact input_generation from current task_states; stale views are rejected"},"resolution":{"type":"string","description":"Concrete resolution with evidence and next implementation step under the existing scope and cumulative execution limits"},"scope":{"enum":["edits_only","existing_authorized_execution"],"description":"Default edits_only; select existing_authorized_execution for the single requested validation continuation under an existing registered grant; always asks an explicit bound user decision"}}),vec!["turn","task","round","input_generation","resolution"]),
        "request_replan" => ("Ask the planner to redraft the whole plan when contract repairs are not enough. It is reviewed like any new plan.".into(), json!({"reason":{"type":"string","description":"What the current plan gets wrong and what the new plan must change"}}), vec!["turn", "reason"]),
        _ => ("Change the running plan you were shown.".into(), schema::revision(), vec!["turn", "reason", "tasks", "retire"]),
    }
}
