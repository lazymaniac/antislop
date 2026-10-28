use super::super::*;

pub(super) fn comment_context(
    core: &Core,
    conversation: &str,
    upto: u64,
    answered: u64,
    is_comment: bool,
) -> Result<String> {
    let mut context = String::new();
    if is_comment {
        append_comment_instructions(&mut context);
        append_comment_plans(core, conversation, upto, &mut context)?;
    } else {
        append_comment_followup(core, conversation, answered, &mut context)?;
    }
    Ok(context)
}

fn append_comment_followup(
    core: &Core,
    conversation: &str,
    answered: u64,
    context: &mut String,
) -> Result<()> {
    if core.store.last_answered(conversation, ORCHESTRATOR)? >= answered {
        return Ok(());
    }
    context.push_str("\n\nThe comment agent handled feedback since your previous turn. Continue from its current plan and answer:\n");
    append_last_orchestrator_reply(core, conversation, context)?;
    append_latest_plan(core, conversation, context)?;
    Ok(())
}

fn append_last_orchestrator_reply(
    core: &Core,
    conversation: &str,
    context: &mut String,
) -> Result<()> {
    if let Some(reply) = core
        .store
        .messages_by(conversation, ORCHESTRATOR, 0)?
        .last()
    {
        context.push_str(&reply.text);
        context.push('\n');
    }
    Ok(())
}

fn append_latest_plan(core: &Core, conversation: &str, context: &mut String) -> Result<()> {
    if let Some(task) = core.store.latest_plan(conversation)?.first()
        && let Some(document) = core.store.plan_document(conversation, &task.plan_id)?
    {
        context.push_str(&document);
        context.push('\n');
    }
    Ok(())
}

fn append_comment_instructions(context: &mut String) {
    context.push_str(
        "\n\nAddress each new user comment explicitly by its #id now. For plan feedback, \
         produce a revised complete plan in the required plan format when changes are \
         needed; otherwise explain your decision or ask a concrete question. Do not \
         merely acknowledge or wait for plan acceptance. Only comments included in \
         this turn can be addressed.\n\n\
         Name the comments your answer or revised plan took care of in `addressed` of \
         finish_turn or submit_plan, each with its id and how. A comment you only \
         answered, or that needs the user's decision, stays out of it and stays open.\n",
    );
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
        context.push_str(&format!("\nCurrent plan: {}. Preserve changes from this version when revising historical feedback.\n", task.plan_id));
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
        context.push_str(&format!("\nOriginal user request:\n{}\n", request.text));
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
            context.push_str(&format!("\nReferenced plan {plan}:\n{document}\n"));
        }
    }
    Ok(())
}
