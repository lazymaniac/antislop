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
    prompt.push_str(&format!("\nCurrent task contracts for plan {}:\n{}\nChange it with revise_plan {{reason,tasks,retire}}: it applies to this plan at this revision. Only changed or added tasks go in tasks, each complete; omitted tasks are preserved, and a task without `covers` keeps the requirements it had. Retire a task only by replacing its obligations. An approved task keeps its result when only its title, goal or summary changes; name it in redo only when its result is wrong and must be done again, which also reruns every task depending on it. A revision needing paths outside the approved scope is put to the user by the runtime, which resubmits it when they approve; do not ask them about it yourself. A task that audits again after a remediation reviews the remediation's changes and whether the findings it addresses are fixed, not the whole change once more. Keep a task that changes no files bounded: when its verifier names gaps, add exactly those paths to its goal, never an open-ended remainder; a report that states the paths it did not inspect is complete in that respect. Never emit graph or recover blocks.\n",graph[0].plan_id,graph_json));
}

fn prepend_phase(prompt: &mut String, interviewing: bool, workflow: &Workflow) {
    if interviewing {
        prompt.insert_str(0, INTERVIEW_INSTRUCTIONS);
        return;
    }
    prompt.insert_str(
        0,
        &format!(
            "\nSESSION PHASE: {}. Collected requirements:\n{}\nThe planner drafts the first plan; you revise it for review and comments. Never start implementation yourself; the runtime starts it once the reviewed plan is accepted. During implementation, follow progress and respond to results or questions; propose revisions for approval if necessary.\n",
            workflow.phase, workflow.summary
        ),
    );
}

const INTERVIEW_INSTRUCTIONS: &str = "\nSESSION PHASE: INTERVIEW. This overrides any instruction to propose a plan. Ask concise concrete questions in the user's language to establish the goal, scope, constraints, exclusions and observable acceptance criteria. Use answers already given; do not repeat questions. Even a self-contained request needs a requirements checkpoint. Ask one question at a time with ask_user: context (why the decision matters), question and options, each with a label and a description of its tradeoff. Offer 2–6 concrete options, put the recommended option first, mark its label as recommended in the user's language and explain its tradeoff. Do not include an Other option: the UI always supplies a custom answer field. When no more questions are needed, call submit_requirements with a Markdown summary of the agreed requirements and any explicit assumptions. This completes the interview. In supervised mode, wait for explicit user approval before planning; in autonomous mode planning starts automatically. Do not submit a plan, schedule work or implement anything. The UI offers approval of the requirements summary; the user can comment before proceeding.\n";
