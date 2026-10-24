use super::super::*;
use crate::lean::core_prompt_helpers::orchestrator_prompt;
use crate::lean::store::Workflow;
use std::path::Path;

pub(super) struct PromptContext<'a> {
    pub(super) project: &'a Path,
    pub(super) first: bool,
    pub(super) answered: u64,
    pub(super) asked: &'a [Message],
    pub(super) open: &'a [Comment],
    pub(super) history: &'a [Message],
    pub(super) comment_context: &'a str,
    pub(super) editable_graph: bool,
    pub(super) graph: &'a [Task],
    pub(super) graph_json: &'a str,
    pub(super) interviewing: bool,
    pub(super) workflow: Option<&'a Workflow>,
    pub(super) ledger_context: Option<&'a str>,
}

pub(super) fn orchestrator_prompt_text(context: PromptContext<'_>) -> String {
    let mut prompt = orchestrator_prompt(
        context.project,
        context.first || context.answered == 0,
        context.asked,
        context.open,
    );
    append_history(
        &mut prompt,
        context.first,
        context.answered,
        context.history,
    );
    prompt.push_str(context.comment_context);
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

fn append_graph(prompt: &mut String, editable: bool, graph: &[Task], graph_json: &str) {
    if !editable {
        return;
    }
    prompt.push_str(&format!(
        "\nEditable graph snapshot, base_plan={}:\n{}\nTo insert tasks or change dependencies, return a fenced ```graph JSON block with base_plan and tasks (complete specs of only added or updated tasks). Omitted nodes are preserved. Keep existing IDs. For example, to insert implementation between A and security/performance gates, add B depending on A and update those gates to depend on B. The runtime merges and validates the entire DAG. Before execution it stores a new immutable plan revision for review and approval. During execution it independently reviews the additive edit and applies it atomically within the approved requirements, preserving existing agents and results. Return either graph or plan, never both. During execution, existing task updates may change only depends_on, and only before that task has started. Add a new task for other changes. Never rewrite running or completed tasks. Historical graphs remain accessible through /plans.\n",
        graph[0].plan_id,
        graph_json
    ));
}

fn prepend_phase(prompt: &mut String, interviewing: bool, workflow: Option<&Workflow>) {
    if interviewing {
        prompt.insert_str(0, INTERVIEW_INSTRUCTIONS);
    } else if let Some(workflow) = workflow {
        prompt.insert_str(
            0,
            &format!(
                "\nSESSION PHASE: {}. Collected requirements:\n{}\nPrepare or revise the complete plan for review and comments. Never start implementation yourself; separate user plan approval is required. During implementation, follow progress and respond to results or questions; propose revisions for approval if necessary.\n",
                workflow.phase, workflow.summary
            ),
        );
    }
}

fn append_ledger(context: Option<&str>, prompt: &mut String) {
    let Some(context) = context else { return };
    prompt.push_str(context);
    prompt.push_str(LEDGER_INSTRUCTIONS);
}

pub(super) fn planning_ledger_context(core: &Core, conversation: &str) -> Result<Option<String>> {
    load_planning_context(core.store.planning_run(conversation)?, |run| {
        core.ledger_context(conversation, &run.id)
    })
}

fn load_planning_context<T>(
    run: Option<T>,
    render: impl FnOnce(T) -> Result<String>,
) -> Result<Option<String>> {
    run.map(render).transpose()
}

const LEDGER_INSTRUCTIONS: &str = "\nThe runtime binds this plan to the active requirements run. Each task must include requirement_ids and criterion_ids. Every task serves at least one active requirement. Cover every mandatory criterion across the tasks; supporting tasks need not cover every criterion themselves. Use the reviewed implementation comparison and resolve its findings in the plan. Do not ask the user to copy run or option identifiers.\n";

const INTERVIEW_INSTRUCTIONS: &str = "\nSESSION PHASE: INTERVIEW. This overrides any instruction to propose a plan. Ask concise concrete questions in the user's language to establish the goal, scope, constraints, exclusions and observable acceptance criteria. Use answers already given; do not repeat questions. Even a self-contained request needs a requirements checkpoint. Ask one question at a time using a fenced ```question JSON block: {\"id\":\"scope\",\"context\":\"Why this decision matters\",\"question\":\"What should we build?\",\"options\":[{\"label\":\"Option A\",\"description\":\"Tradeoff\"},{\"label\":\"Option B\",\"description\":\"Tradeoff\"}]}. Offer 2–6 concrete options, put the recommended option first, mark its label as recommended in the user's language and explain its tradeoff. Do not include an Other option: the UI always supplies a custom answer field. Do not combine a question with the requirements checkpoint in the same answer. When no more questions are needed, end with a fenced ```requirements Markdown summary containing those agreed requirements and any explicit assumptions. This completes the interview. In supervised mode, wait for explicit user approval before planning; in autonomous mode planning starts automatically. Do not emit a plan block, schedule work or implement anything. The UI offers approval of the requirements summary; the user can comment before proceeding.\n";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_context_storage_error_is_not_replaced_with_empty_text() {
        let run = Some(());
        let result =
            load_planning_context(run, |_| Err(Error::Invalid("ledger unavailable".into())));

        assert!(matches!(
            result,
            Err(Error::Invalid(message)) if message == "ledger unavailable"
        ));
    }
}
