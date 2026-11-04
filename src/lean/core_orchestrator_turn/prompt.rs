use super::super::*;
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
    append_ledger(context.ledger_context, &mut prompt);
    prompt.insert_str(0, "\nRuntime capability boundary: only you can call revise_plan. Use deferred=true on future tasks that must remain stopped until a later authorized phase; this is enforced by scheduling and launch checks. Keep permitted independent diagnosis tasks active. Do not invent a producer that cannot run under current authority, approve a proposal as permission, or delete future obligations. Release deferred only through a reviewed revision after actual authorization. For incremental changes to the accepted plan, call revise_plan yourself with only changed/added contracts and an explicit retire list. Do not ask the planner to call revise_plan: request_replan delegates a complete replacement draft via submit_plan, not incremental edits. If the planner reports this capability mismatch, apply the incremental revision yourself; do not repeat the same delegation. Unchanged historical contracts need not be redeclared to obey a newly imposed execution limit; changed/new/redo work and each task launch remain constrained.\n");
    prompt
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
    prompt.push_str(&format!("\nCurrent task contracts for plan {}:\n{}\nThe task_states snapshot is authoritative for this turn; earlier plans, ids and statuses in conversation history may be obsolete. Use only the task names/runtime_ids listed here. retry_task accepts only status=failed for transient execution failures. A worker can end with report_blocker without claiming success. A coordinating task with an unmet resume condition stays stopped: do not retry, replan, or revise its wording merely to restate the same missing input. Resolve the prerequisite with concrete new evidence, or ask the user once for the specific decision outside your authority. Revisions must supply a feasible continuation, not another task describing the same blocker. Each missing prerequisite needs an owner: add an executable producer within existing authority, depend on available evidence, or present a concrete ask_user decision. A task whose only job is to wait for an absent input, with no producer, is not a repair. Do not equate audit similarity flags with proven leakage or demand unsupported external attestations. Check the original acceptance criteria before carrying a prior agent's procedural restriction into a new contract. Historical unfinished_checkpoints are runtime recovery metadata: use replacements={{new_code_task:source_task_UUID}} to recover a listed historical checkpoint into a fresh code writer for reassessment and integration; do not put commit hashes in task goals or acceptance criteria. Recovery of a technical patch can precede a still-pending audit without claiming the audit passed. When splitting unfinished work in the current plan, use replacements to inherit its checkpoint; a report describing preserved changes does not transfer those changes to a fresh task. task_states is runtime metadata, never copy it into task contracts. Change it with revise_plan {{reason,tasks,retire}}: it applies to this plan at this revision. Only changed or added tasks go in tasks, each complete; omitted tasks are preserved, and a task without `covers` keeps the requirements it had. Retire a task only by replacing its obligations. An approved task keeps its result when only its title, goal or summary changes; name it in redo only when its result is wrong and must be done again, which also reruns every task depending on it. Task revisions do not grant filesystem access: use the allocated checkout and CLI permissions. File hints in goals are guidance, not path allowlists. Preserve the user's requirements and execution constraints. A task that audits again after a remediation reviews the remediation's changes and whether the findings it addresses are fixed, not the whole change once more. Keep a task that changes no files bounded: when its reviewer names gaps, add exactly those paths to its goal, never an open-ended remainder; a report that states the paths it did not inspect is complete in that respect. Never emit graph or recover blocks.\n",graph[0].plan_id,graph_json));
}

fn prepend_phase(prompt: &mut String, interviewing: bool, workflow: &Workflow) {
    if interviewing {
        prompt.insert_str(0, INTERVIEW_INSTRUCTIONS);
        return;
    }
    prompt.insert_str(
        0,
        &format!(
            "\nSESSION PHASE: {}. Collected requirements:\n{}\nThe planner drafts plans and corrects plan reviewer findings; you handle user comments and planner blockers. Never start implementation yourself; the runtime starts it once the reviewed plan is accepted. During implementation, follow progress and respond to results or questions; propose revisions for approval if necessary.\n",
            workflow.phase, workflow.summary
        ),
    );
}

const INTERVIEW_INSTRUCTIONS: &str = "\nSESSION PHASE: INTERVIEW. This overrides any instruction to propose a plan. Ask concise concrete questions in the user's language to establish the goal, scope, constraints, exclusions and observable acceptance criteria. Use answers already given; do not repeat questions. Even a self-contained request needs a requirements checkpoint. Ask one question at a time with ask_user: context (why the decision matters), question and options, each with a label and a description of its tradeoff. Offer 2–6 concrete options, put the recommended option first, mark its label as recommended in the user's language and explain its tradeoff. Do not include an Other option: the UI always supplies a custom answer field. When no more questions are needed, call submit_requirements with a Markdown summary of the agreed requirements and any explicit assumptions. This completes the interview. In supervised mode, wait for explicit user approval before planning; in autonomous mode planning starts automatically. Do not submit a plan, schedule work or implement anything. The UI offers approval of the requirements summary; the user can comment before proceeding.\n";
