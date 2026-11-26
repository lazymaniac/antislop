use super::gateway::{ASK, ASK_USER, REVIEW, SUBMIT_PLAN, SUBMIT_REQUIREMENTS, State, TOOL, WAIT};
use super::schema;
use crate::lean::prompts;
use serde_json::{Value, json};

type ToolSpec = (String, Value, Vec<&'static str>);

/// The roles that review plans, audits, or code changes without check coverage.
fn plain_reviewer(role: &str) -> bool {
    matches!(role, "plan-reviewer" | "auditor") || role.starts_with("code-review:")
}

pub(crate) fn allowed_tools(role: &str) -> Vec<&'static str> {
    let mut names = role_tools(role);
    if names.contains(&"current_turn") && !role.starts_with("planning-") {
        names.extend([
            "prepare_validation",
            "complete_validation",
            "invalidate_validation",
            "read_validations",
        ]);
    }
    names
}

fn role_tools(role: &str) -> Vec<&'static str> {
    if let Some((_, reviewer)) = crate::lean::task_role(role) {
        return task_tools(reviewer);
    }
    match role {
        "orchestrator" | "comment-orchestrator" => orchestrator_tools(),
        "planner" => vec![
            SUBMIT_PLAN,
            "consult_planning",
            "read_dependency_result",
            "read_planning_result",
            "read_plan_context",
            "read_audit",
            TOOL,
            "get_images",
            "read_evidence",
            "current_turn",
            "read_task_contract",
        ],
        "auditor" => vec![
            REVIEW,
            "read_plan_context",
            "read_audit",
            "read_dependency_result",
            "read_planning_result",
            "get_images",
            "read_evidence",
            "current_turn",
            "read_task_contract",
        ],
        _ if role.starts_with("planning-") => {
            vec![TOOL, "current_turn", "read_planning_result", "get_images"]
        }
        _ if plain_reviewer(role) => vec![
            REVIEW,
            "get_images",
            "read_dependency_result",
            "read_planning_result",
            "read_evidence",
            "current_turn",
            "read_plan_context",
            "read_task_contract",
            "read_audit",
        ],
        _ => vec![TOOL],
    }
}

pub(super) fn scoped_tools(state: &State) -> Vec<&'static str> {
    let role = &state.role;
    let mut names = allowed_tools(role);
    if role.starts_with("planning-") {
        names.retain(|name| match *name {
            "read_planning_result" => state.planning_reports,
            "get_images" => state.planning_images,
            _ => true,
        });
    }
    if crate::lean::task_role(role).is_some_and(|(_, reviewer)| !reviewer)
        && let Some(task) = &state.contract
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

pub(super) fn scoped_tool_list(state: &State) -> Vec<Value> {
    let names = scoped_tools(state);
    let mut list = tool_list(&state.role);
    list.retain(|tool| {
        tool["name"]
            .as_str()
            .is_some_and(|name| names.contains(&name))
    });
    if let Some(task) = &state.contract
        && let Some(review) = list.iter_mut().find(|tool| tool["name"] == "review")
    {
        let checks = &mut review["inputSchema"]["properties"]["checks"];
        let check_schema = checks["additionalProperties"].clone();
        let obligations = crate::lean::tasks::obligations(task);
        checks["properties"] = obligations
            .iter()
            .map(|(id, _)| (id.clone(), check_schema.clone()))
            .collect::<serde_json::Map<_, _>>()
            .into();
        checks["required"] = json!(obligations.iter().map(|(id, _)| id).collect::<Vec<_>>());
        checks["additionalProperties"] = false.into();
        review["inputSchema"]["properties"]["findings"]["items"]["properties"]["check"] =
            json!({"enum":obligations.iter().map(|(id, _)| id).collect::<Vec<_>>()});
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
            "read_planning_result",
            "get_images",
            "read_evidence",
            "current_turn",
            "read_task_contract",
        ]
    } else {
        vec![
            "complete",
            "select",
            "report_blocker",
            ASK,
            WAIT,
            "read_dependency_result",
            "read_planning_result",
            "get_images",
            "record_evidence",
            "read_evidence",
            "current_turn",
            "read_task_contract",
        ]
    }
}

fn orchestrator_tools() -> Vec<&'static str> {
    vec![
        TOOL,
        "get_images",
        "current_turn",
        ASK_USER,
        SUBMIT_REQUIREMENTS,
        SUBMIT_PLAN,
        "read_dependency_result",
        "read_planning_result",
        "read_plan_context",
        "read_task_contract",
        "read_audit",
        "read_evidence",
        "answer_agent",
        "defer_to_user",
        "decide_approval",
        "retry_task",
        "resume_task",
        "revise_plan",
        "request_replan",
        "create_session_pr",
        "merge_session_with_main",
    ]
}

const RETRY: &str = prompts::RETRY;

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
        properties.insert("evidence".into(),json!({"type":"array","maxItems":64,"items":{"type":"string"},"description":prompts::RESULT_EVIDENCE_DESCRIPTION}));
    }
    if name != "current_turn" {
        let description = if allowed_tools(role).contains(&"current_turn") {
            prompts::ACTIVE_TURN_WITH_RECOVERY_DESCRIPTION
        } else {
            prompts::ACTIVE_TURN_DESCRIPTION
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
        "get_images" => (
            prompts::TOOL_GET_IMAGES.into(),
            json!({"offset":{"type":"integer","minimum":0,"default":0}}),
            vec!["turn"],
        ),
        "submit_plan" | "revise_plan" => plan_contract(name, orchestrator),
        "answer_agent"
        | "defer_to_user"
        | "decide_approval"
        | "retry_task"
        | "resume_task"
        | "create_session_pr"
        | "merge_session_with_main"
        | "request_replan" => control_contract(name),
        _ => turn_contract(name, role, orchestrator),
    }
}

fn turn_contract(name: &str, role: &str, orchestrator: bool) -> ToolSpec {
    match name {
        "read_plan_context" => (
            prompts::TOOL_READ_PLAN_CONTEXT.into(),
            json!({"cursor":{"type":"string"}}),
            vec!["turn"],
        ),
        "read_task_contract" => (
            prompts::TOOL_READ_TASK_CONTRACT.into(),
            json!({"task":{"type":"string"},"cursor":{"type":"string"}}),
            vec!["turn", "task"],
        ),
        "read_audit" => (
            prompts::TOOL_READ_AUDIT.into(),
            json!({"audit_ref":{"type":"string"},"cursor":{"type":"string"}}),
            vec!["turn", "audit_ref"],
        ),
        "prepare_validation" => (
            prompts::TOOL_PREPARE_VALIDATION.into(),
            json!({"command":{"type":"array","minItems":1,"items":{"type":"string"},"description":prompts::VALIDATION_COMMAND_DESCRIPTION},"directory":{"type":"string","description":prompts::TOOL_PREPARE_VALIDATION_DIRECTORY},"scope":{"type":"string","description":prompts::TOOL_PREPARE_VALIDATION_SCOPE},"environment":{"type":"string","description":prompts::TOOL_PREPARE_VALIDATION_ENVIRONMENT}}),
            vec!["turn", "command", "scope", "environment"],
        ),
        "complete_validation" => (
            prompts::TOOL_COMPLETE_VALIDATION.into(),
            json!({"validation":{"type":"string"},"exit_code":{"type":"integer"},"output":{"type":"string"},"problems":{"type":"string"}}),
            vec!["turn", "validation", "exit_code", "output", "problems"],
        ),
        "invalidate_validation" => (
            prompts::TOOL_INVALIDATE_VALIDATION.into(),
            json!({"validation":{"type":"string"},"reason":{"type":"string"}}),
            vec!["turn", "validation", "reason"],
        ),
        "read_validations" => (
            prompts::TOOL_READ_VALIDATIONS.into(),
            json!({"cursor":{"type":"string"}}),
            vec!["turn"],
        ),
        "consult_planning" => (
            prompts::TOOL_CONSULT_PLANNING.into(),
            json!({"kind":{"enum":["research","analysis","dependencies","architecture","test_design"]},"question":{"type":"string"},"refresh":{"type":"boolean"}}),
            vec!["turn", "kind", "question"],
        ),
        "read_planning_result" => (
            prompts::TOOL_READ_PLANNING_RESULT.into(),
            json!({"reference":{"type":"string"},"cursor":{"type":"string"}}),
            vec!["turn", "reference"],
        ),
        "current_turn" => (prompts::TOOL_CURRENT_TURN.into(), json!({}), vec![]),
        "report_blocker" => (
            prompts::TOOL_REPORT_BLOCKER.into(),
            json!({"report":{"type":"string"},"resume_condition":{"type":"string","description":prompts::TOOL_REPORT_BLOCKER_RESUME_CONDITION},"unblocked_work_remaining":{"type":"boolean","description":prompts::TOOL_REPORT_BLOCKER_UNBLOCKED_WORK_REMAINING}}),
            vec![
                "turn",
                "report",
                "resume_condition",
                "unblocked_work_remaining",
            ],
        ),
        "record_evidence" => (
            prompts::TOOL_RECORD_EVIDENCE.into(),
            json!({"label":{"type":"string","maxLength":160},"text":{"type":"string","maxLength":262144}}),
            vec!["turn", "label", "text"],
        ),
        "read_evidence" => (
            prompts::TOOL_READ_EVIDENCE.into(),
            json!({"evidence":{"type":"string"},"cursor":{"type":"string"}}),
            vec!["turn", "evidence"],
        ),
        "read_dependency_result" => (
            prompts::TOOL_READ_DEPENDENCY_RESULT.into(),
            json!({"view":{"enum":["report","review"],"description":prompts::TOOL_READ_DEPENDENCY_RESULT_VIEW},"dependency":{"type":"string","description":prompts::TOOL_READ_DEPENDENCY_RESULT_DEPENDENCY},"result_ref":{"type":"string","description":prompts::TOOL_READ_DEPENDENCY_RESULT_RESULT_REF},"round":{"type":"integer","minimum":0,"description":prompts::TOOL_READ_DEPENDENCY_RESULT_ROUND},"cursor":{"type":"string"}}),
            vec!["turn", "dependency"],
        ),
        "finish_turn" if orchestrator => (
            format!(prompts::template!(ORCHESTRATOR_FINISH_TURN), RETRY = RETRY),
            json!({"text":{"type":"string","description":prompts::TOOL_FINISH_TURN_TEXT},"addressed":schema::addressed()}),
            vec!["turn", "text"],
        ),
        "finish_turn" if role == "planner" => (
            format!(prompts::template!(PLANNER_FINISH_TURN), RETRY = RETRY),
            json!({"text":{"type":"string","description":prompts::PLANNER_BLOCKER_DESCRIPTION}}),
            vec!["turn", "text"],
        ),
        "finish_turn" => (
            format!(prompts::template!(FINISH_TURN), RETRY = RETRY),
            json!({"text":{"type":"string","description":prompts::TOOL_FINISH_TURN_TEXT}}),
            vec!["turn", "text"],
        ),
        "ask_user" => (
            format!(prompts::template!(TOOL_ASK_USER), RETRY = RETRY),
            schema::question(),
            vec!["turn", "context", "question", "options"],
        ),
        "submit_requirements" => (
            format!(prompts::template!(TOOL_SUBMIT_REQUIREMENTS), RETRY = RETRY),
            json!({"markdown":{"type":"string","description":prompts::TOOL_SUBMIT_REQUIREMENTS_MARKDOWN},"title":{"type":"string","description":prompts::TOOL_SUBMIT_REQUIREMENTS_TITLE}}),
            vec!["turn", "markdown"],
        ),
        "complete" => (
            format!(prompts::template!(TOOL_COMPLETE), RETRY = RETRY),
            json!({"summary":{"type":"string","description":prompts::TOOL_COMPLETE_SUMMARY},"report":{"type":"string","description":prompts::TOOL_COMPLETE_REPORT},"commit_message":{"type":"string","description":prompts::TOOL_COMPLETE_COMMIT_MESSAGE}}),
            vec!["turn", "summary", "report", "commit_message"],
        ),
        "select" => (
            format!(prompts::template!(TOOL_SELECT), RETRY = RETRY),
            schema::selection(),
            vec![
                "turn",
                "alternatives",
                "selected",
                "rationale",
                "user_summary",
            ],
        ),
        "review" if plain_reviewer(role) => (
            format!(prompts::template!(PLAIN_REVIEW_TOOL), RETRY = RETRY),
            schema::plain_review(),
            vec!["turn", "report", "findings"],
        ),
        "review" => (
            format!(prompts::template!(TASK_REVIEW_TOOL), RETRY = RETRY),
            schema::review(),
            vec!["turn", "report", "checks", "findings"],
        ),
        "ask" => (
            prompts::TOOL_ASK.into(),
            json!({"question":{"type":"string"}}),
            vec!["turn", "question"],
        ),
        "wait_for_answer" => (
            prompts::TOOL_WAIT_FOR_ANSWER.into(),
            json!({"note":{"type":"string","description":prompts::TOOL_WAIT_FOR_ANSWER_NOTE}}),
            vec!["turn"],
        ),
        _ => (
            format!(prompts::template!(FINISH_TURN), RETRY = RETRY),
            json!({"text":{"type":"string","description":prompts::TOOL_FINISH_TURN_TEXT}}),
            vec!["turn", "text"],
        ),
    }
}

fn plan_contract(name: &str, orchestrator: bool) -> ToolSpec {
    if name == "submit_plan" {
        let mut fields = json!({"document":{"type":"string","description":prompts::PLAN_DOCUMENT_DESCRIPTION},"tasks":{"type":"array","minItems":1,"items":schema::planned_task()},"keep":{"type":"array","items":{"type":"string"},"description":prompts::PLAN_KEPT_TASKS_DESCRIPTION}});
        fields["team"] = json!({"enum": crate::lean::tasks::Team::ALL.map(crate::lean::tasks::Team::name), "description": format!(prompts::template!(PLAN_TEAM_DESCRIPTION), crate::lean::tasks::team_catalog())});
        fields["scope"] = json!({"type":"string","description":prompts::PLAN_SCOPE_DESCRIPTION});
        fields["team_reason"] =
            json!({"type": "string", "description": prompts::PLAN_TEAM_REASON_DESCRIPTION});
        if orchestrator {
            fields["addressed"] = schema::addressed();
        }
        return (
            prompts::SUBMIT_PLAN_TOOL.into(),
            fields,
            vec!["turn", "document", "tasks"],
        );
    }
    (
        prompts::REVISE_PLAN_TOOL.into(),
        schema::revision(),
        vec!["turn", "reason", "tasks", "retire"],
    )
}

fn control_contract(name: &str) -> ToolSpec {
    match name {
        "answer_agent" => (
            prompts::TOOL_ANSWER_AGENT.into(),
            json!({"request":{"type":"string","description":prompts::TOOL_ANSWER_AGENT_REQUEST},"answer":{"type":"string"},"disposition":{"enum":["continue","wait_for_revision","wait_for_dependency","escalate_to_user"]},"target":{"type":"string","description":prompts::TOOL_ANSWER_AGENT_TARGET}}),
            vec!["turn", "request", "answer"],
        ),
        "defer_to_user" => (
            prompts::TOOL_DEFER_TO_USER.into(),
            json!({"request":{"type":"string","description":prompts::TOOL_ANSWER_AGENT_REQUEST},"explanation":{"type":"string","description":prompts::TOOL_DEFER_TO_USER_EXPLANATION}}),
            vec!["turn", "request", "explanation"],
        ),
        "decide_approval" => (
            prompts::TOOL_DECIDE_APPROVAL.into(),
            json!({"request":{"type":"string","description":prompts::TOOL_DECIDE_APPROVAL_REQUEST},"choice":{"type":"string","description":prompts::TOOL_DECIDE_APPROVAL_CHOICE},"explanation":{"type":"string","description":prompts::TOOL_DECIDE_APPROVAL_EXPLANATION}}),
            vec!["turn", "request", "choice"],
        ),
        "retry_task" => (
            prompts::TOOL_RETRY_TASK.into(),
            json!({"task":{"type":"string"},"guidance":{"type":"string"}}),
            vec!["turn", "task", "guidance"],
        ),
        "resume_task" => (
            prompts::TOOL_RESUME_TASK.into(),
            json!({"task":{"type":"string"},"round":{"type":"integer","minimum":0},"input_generation":{"type":"integer","minimum":0},"resolution":{"type":"string"}}),
            vec!["turn", "task", "round", "input_generation", "resolution"],
        ),
        "create_session_pr" => (
            prompts::TOOL_CREATE_SESSION_PR.into(),
            json!({}),
            vec!["turn"],
        ),
        "merge_session_with_main" => (
            prompts::TOOL_MERGE_SESSION_WITH_MAIN.into(),
            json!({}),
            vec!["turn"],
        ),
        "request_replan" => (
            prompts::TOOL_REQUEST_REPLAN.into(),
            json!({"reason":{"type":"string","description":prompts::TOOL_REQUEST_REPLAN_REASON}}),
            vec!["turn", "reason"],
        ),
        _ => (
            prompts::LEGACY_REVISE_PLAN_TOOL.into(),
            schema::revision(),
            vec!["turn", "reason", "tasks", "retire"],
        ),
    }
}
