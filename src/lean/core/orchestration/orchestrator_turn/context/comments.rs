use super::super::*;
use crate::lean::prompts;

#[derive(Default)]
pub(in crate::lean::core::orchestration::orchestrator_turn) struct CommentContext {
    pub guidance: String,
    pub original_request: Option<String>,
}

pub(super) fn comment_context(
    core: &Core,
    conversation: &str,
    upto: u64,
    is_comment: bool,
) -> Result<CommentContext> {
    if !is_comment {
        return Ok(CommentContext::default());
    }
    let mut guidance = prompts::APPEND_COMMENT_INSTRUCTIONS.to_owned();
    append_comment_plans(core, conversation, upto, &mut guidance)?;
    let original_request = core
        .store
        .messages_by(conversation, "user", 0)?
        .into_iter()
        .find(|message| !message.text.starts_with(COMMENT_PREFIX))
        .map(|message| message.text);
    Ok(CommentContext {
        guidance,
        original_request,
    })
}

fn append_comment_plans(
    core: &Core,
    conversation: &str,
    upto: u64,
    context: &mut String,
) -> Result<()> {
    let comments = core.store.comments(conversation, "open")?;
    let mut plans: std::collections::BTreeSet<_> = comments
        .iter()
        .filter(|comment| comment.message <= upto)
        .filter_map(|comment| comment.plan_id.as_deref())
        .collect();
    let latest = core.store.latest_plan(conversation)?;
    if let Some(task) = latest.first() {
        plans.insert(task.plan_id.as_str());
        context.push_str(&format!(
            prompts::template!(APPEND_COMMENT_PLANS),
            task.plan_id
        ));
    }
    append_referenced_plans(core, conversation, plans, context)
}

fn append_referenced_plans<'a>(
    core: &Core,
    conversation: &str,
    plans: impl IntoIterator<Item = &'a str>,
    context: &mut String,
) -> Result<()> {
    for plan in plans {
        if let Some(document) = core.store.plan_document(conversation, plan)? {
            context.push_str(&format!(
                prompts::template!(APPEND_REFERENCED_PLANS),
                document.len(),
                plan = plan
            ));
        }
    }
    Ok(())
}
