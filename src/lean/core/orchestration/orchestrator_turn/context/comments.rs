use super::super::*;
use crate::lean::prompts;

pub(super) fn comment_context(
    core: &Core,
    conversation: &str,
    upto: u64,
    is_comment: bool,
) -> Result<String> {
    let mut context = String::new();
    if is_comment {
        append_comment_instructions(&mut context);
        append_comment_plans(core, conversation, upto, &mut context)?;
    }
    Ok(context)
}

fn append_comment_instructions(context: &mut String) {
    context.push_str(prompts::APPEND_COMMENT_INSTRUCTIONS);
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
    append_original_request(core, conversation, context)?;
    append_referenced_plans(core, conversation, plans, context)
}

fn append_original_request(core: &Core, conversation: &str, context: &mut String) -> Result<()> {
    if let Some(request) = core
        .store
        .messages_by(conversation, "user", 0)?
        .iter()
        .find(|message| !message.text.starts_with(COMMENT_PREFIX))
    {
        context.push_str(&format!(
            prompts::template!(APPEND_ORIGINAL_REQUEST),
            request.text
        ));
    }
    Ok(())
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
