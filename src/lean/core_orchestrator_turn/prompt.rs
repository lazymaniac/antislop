use super::super::*;
use super::context::AuditResponse;
use crate::lean::core_planning_context::append_ledger;
use crate::lean::core_prompt_helpers::orchestrator_prompt;
use crate::lean::store::Workflow;
use std::path::Path;

pub(super) struct PromptContext<'a> {
    pub(super) project: &'a Path,
    pub(super) first: bool,
    /// A new session, which needs the earlier conversation.
    pub(super) fresh: bool,
    pub(super) answered: u64,
    pub(super) asked: &'a [Message],
    pub(super) open: &'a [Comment],
    pub(super) history: &'a [Message],
    pub(super) comment_context: &'a str,
    pub(super) editable_graph: bool,
    pub(super) graph: &'a [Task],
    pub(super) graph_json: &'a str,
    pub(super) interviewing: bool,
    pub(super) workflow: &'a Workflow,
    pub(super) ledger_context: Option<&'a str>,
    /// The open user decision, if any was ever asked.
    pub(super) decision: Option<&'a Option<crate::lean::interview::Question>>,
    pub(super) audit_response: Option<&'a AuditResponse>,
}

pub(super) fn orchestrator_prompt_text(context: PromptContext<'_>) -> String {
    let mut prompt = orchestrator_prompt(
        context.project,
        context.first || context.answered == 0,
        context.asked,
        context.open,
    );
    // A resumed session already holds the conversation.
    append_history(
        &mut prompt,
        context.fresh,
        context.answered,
        context.history,
    );
    prompt.push_str(context.comment_context);
    append_decision(&mut prompt, context.decision);
    append_graph(
        &mut prompt,
        context.editable_graph && !context.interviewing,
        context.graph,
        context.graph_json,
    );
    prepend_phase(&mut prompt, context.interviewing, context.workflow);
    append_audit_response(&mut prompt, context.audit_response);
    append_ledger(context.ledger_context, &mut prompt);
    prompt.insert_str(0, "\nOnly the orchestrator calls revise_plan for incremental changes: provide changed/added contracts and explicit retire entries. request_replan delegates a complete replacement draft to the planner. The runtime imposes no access or execution limits on agents. A deferred task remains a deliberate scheduling hold until a reviewed revision releases it; do not infer a permission hold from historical instructions. Preserve actual prerequisites, evidence and user decisions.\n");
    prompt
}

fn append_audit_response(prompt: &mut String, audit: Option<&AuditResponse>) {
    let Some(audit) = audit else { return };
    prompt.push_str("\nAUDIT RESPONSE GATE: this auditor report must be answered before any workflow transition. Record `Audit response:` followed by one line for each finding, in report order: `finding N: resolved - <why it is resolved>` or `finding N: open - <why it remains open and what happens next>`. An open finding that needs work must be routed through a reviewed plan revision. The response is recorded as the orchestrator's answer.\n");
    prompt.push_str("Original auditor report:\n");
    prompt.push_str(&audit.report);
    prompt.push('\n');
    if let Some(error) = &audit.error {
        prompt.push_str(&format!("The audit failed: {error}. Explicitly report `audit status: unaudited - <reason>` and keep the result open.\n"));
    } else if audit.findings.is_empty() {
        let state = if audit.approved { "complete" } else { "open" };
        prompt.push_str(&format!("The audit recorded no findings and its verdict is {}. Record `audit status: {state} - <your assessment>` before answering the user.\n", if audit.approved { "approved" } else { "not approved" }));
    } else {
        for (index, finding) in audit.findings.iter().enumerate() {
            prompt.push_str(&format!(
                "finding {} [{}] ({} at {}): {}\n",
                index + 1,
                finding["id"].as_str().unwrap_or("no id"),
                finding["severity"]
                    .as_str()
                    .unwrap_or("unspecified severity"),
                finding["where"].as_str().unwrap_or("unspecified location"),
                finding["issue"].as_str().unwrap_or("unspecified issue"),
            ));
        }
    }
}

fn append_history(prompt: &mut String, first: bool, answered: u64, history: &[Message]) {
    if !first || answered == 0 {
        return;
    }
    prompt.push_str("\nPrevious conversation context (most recent messages):\n");
    for message in history {
        prompt.push_str(&format!(
            "{}: {}\n",
            message.author,
            tasks::bound(&message.text, 2048)
        ));
    }
}

/// Names the one decision an answer can resolve, so an answer is never read
/// as the reply to an earlier question it does not name.
fn append_decision(
    prompt: &mut String,
    decision: Option<&Option<crate::lean::interview::Question>>,
) {
    let rule = "An answer resolves only the decision whose id it names. A decision asked before the newest one and never answered was superseded: it stays unanswered, and you ask it again if it still matters.";
    match decision {
        None => {}
        Some(None) => prompt.push_str(&format!("\nNo user decision is open. {rule}\n")),
        Some(Some(question)) => prompt.push_str(&format!(
            "\nOpen user decision {}: {} {rule}\n",
            question.id,
            tasks::bound(&question.question, 600)
        )),
    }
}

fn append_graph(prompt: &mut String, editable: bool, graph: &[Task], graph_json: &str) {
    if !editable {
        return;
    }
    prompt.push_str(&format!("\nCurrent task contracts for plan {}:\n{}\nThe task_states snapshot is authoritative for this turn; earlier plans, ids and statuses in conversation history may be obsolete. Use only the task names/runtime_ids listed here. retry_task accepts only status=failed for transient execution failures. A worker can end with report_blocker without claiming success. A non-deferred coordinating task with a resolved prerequisite or independent work that the blocker does not prevent may continue through resume_task with the exact current round/input_generation, concrete evidence and next step. This preserves the contract, input generation and evidence. Technical recovery needs no permission question. Release a deliberately deferred task only through a reviewed revision. Keep steps with unmet prerequisites stopped: do not retry, replan, or revise its wording merely to restate the same missing input. Resolve the prerequisite with concrete new evidence, or ask the user once for the specific decision outside your authority. Revisions must supply a feasible continuation, not another task describing the same blocker. Each missing prerequisite needs an owner: add an executable producer, depend on available evidence, or present a concrete ask_user decision. A task whose only job is to wait for an absent input, with no producer, is not a repair. Do not equate audit similarity flags with proven leakage or demand unsupported external attestations. Check the original acceptance criteria before carrying a prior agent's procedural restriction into a new contract. Historical unfinished_checkpoints are runtime recovery metadata: use replacements={{new_code_task:source_task_UUID}} to recover a listed historical checkpoint into a fresh code writer for reassessment and integration; do not put commit hashes in task goals or acceptance criteria. Recovery of a technical patch can precede a still-pending audit without claiming the audit passed. When splitting unfinished work in the current plan, use replacements to inherit its checkpoint; a report describing preserved changes does not transfer those changes to a fresh task. task_states is runtime metadata, never copy it into task contracts. Change it with revise_plan {{reason,tasks,retire}}: it applies to this plan at this revision. Only changed or added tasks go in tasks, each complete; omitted tasks are preserved, and a task without `covers` keeps the requirements it had. Retire a task only by replacing its obligations. An approved task keeps its result when only its title, goal or summary changes; name it in redo only when its result is wrong and must be done again, which also reruns every task depending on it. Task revisions do not grant filesystem access: use the allocated checkout and CLI permissions. File hints in goals are guidance, not path allowlists. Preserve the user's requirements and acceptance criteria. A task that audits again after a remediation reviews the remediation's changes and whether the findings it addresses are fixed, not the whole change once more. Keep a task that changes no files bounded: when its reviewer names gaps, add exactly those paths to its goal, never an open-ended remainder; a report that states the paths it did not inspect is complete in that respect. Never emit graph or recover blocks.\n",graph[0].plan_id,graph_json));
}

fn prepend_phase(prompt: &mut String, interviewing: bool, workflow: &Workflow) {
    if interviewing {
        prompt.insert_str(0, INTERVIEW_INSTRUCTIONS);
        return;
    }
    prompt.insert_str(
        0,
        &format!(
            "\nSESSION PHASE: {}. Collected requirements:\n{}\nThe planner drafts plans and corrects plan reviewer findings; you handle user comments and planner blockers. Never start implementation yourself; the runtime starts it after independent plan review. During implementation, follow progress and respond to results or questions; submit revisions for independent review when necessary. The final independent auditor report is addressed to you, the orchestrator. Read and preserve its findings, then explicitly answer them: explain why each finding is resolved or remains open, and route remediation through a reviewed plan revision when work is needed. Do not treat the report as user-facing output to forward unchanged, and do not advance the workflow or claim completion until you have recorded this response.\n",
            workflow.phase, workflow.summary
        ),
    );
}

const INTERVIEW_INSTRUCTIONS: &str = "\nSESSION PHASE: INTERVIEW. This overrides any instruction to propose a plan. Ask concise concrete questions in the user's language to establish the goal, scope, constraints, exclusions and observable acceptance criteria. Use answers already given; do not repeat questions. When the request is self-contained, summarize it directly without asking a question. Ask one question at a time with ask_user: context (why the decision matters), question and options, each with a label and a description of its tradeoff. Offer 2–6 concrete options, put the recommended option first, mark its label as recommended in the user's language and explain its tradeoff. Do not include an Other option: the UI always supplies a custom answer field. When no more questions are needed, call submit_requirements with a Markdown summary of the agreed requirements and any explicit assumptions. The runtime then asks the user to confirm that exact scope. Do not plan while confirmation is missing. If the user requests changes, clarify or revise the summary and ask for confirmation again. After the user selects Confirm scope, planning may begin. Do not submit a plan, schedule work or implement anything during the interview. The user can steer ongoing work with comments.\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn implementation_phase_requires_orchestrator_to_handle_audit_before_advancing() {
        let mut prompt = String::new();
        prepend_phase(
            &mut prompt,
            false,
            &Workflow {
                phase: "implementation".into(),
                summary: "deliver the request".into(),
                upto_message: 0,
            },
        );

        assert!(
            prompt
                .contains("final independent auditor report is addressed to you, the orchestrator")
        );
        assert!(prompt.contains("explicitly answer them"));
        assert!(prompt.contains(
            "do not advance the workflow or claim completion until you have recorded this response"
        ));
        assert!(prompt.contains("planner drafts plans and corrects plan reviewer findings"));
    }
}
