use super::super::*;
pub(super) fn route(context: &Context<'_>, request: Request) -> Result<Route> {
    route_pipeline(
        context,
        request,
        &[plans_and_plan, task, task_checks, task_diff, review_task],
    )
}

fn plans_and_plan(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Plans {
            conversation,
            offset,
            limit,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            let offset = offset.unwrap_or(0);
            let limit = limit.unwrap_or(50);
            json!(core.plans(&conversation, offset, limit)?)
        }),
        Request::Conversation(crate::lean::daemon::ConversationRequest::Plan {
            conversation,
            plan,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            validate_conversation_plan(core, &conversation, &plan)?;
            json!(core.plan_history(&conversation, &plan)?)
        }),
        request => Route::Unmatched(request),
    })
}

fn task(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Task {
            conversation,
            plan,
            id,
        }) => Route::Handled({
            if let Some(conv) = &conversation {
                validate_project_conversation(core, project, conv)?;
                validate_conversation_plan(core, conv, &plan)?;
            } else {
                let conv = core.store.plan_conversation(&plan)?;
                validate_project_conversation(core, project, &conv)?;
            }
            validate_plan_task(core, &plan, &id)?;
            json!(core.task(&plan, &id)?)
        }),
        request => Route::Unmatched(request),
    })
}

fn task_checks(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::TaskChecks {
            conversation,
            plan,
            id,
        }) => Route::Handled({
            if let Some(conv) = &conversation {
                validate_project_conversation(core, project, conv)?;
                validate_conversation_plan(core, conv, &plan)?;
            } else {
                let conv = core.store.plan_conversation(&plan)?;
                validate_project_conversation(core, project, &conv)?;
            }
            validate_plan_task(core, &plan, &id)?;
            json!(core.task_checks(&plan, &id)?)
        }),
        request => Route::Unmatched(request),
    })
}

fn task_diff(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Diff {
            conversation,
            plan,
            id,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            validate_conversation_plan(core, &conversation, &plan)?;
            validate_plan_task(core, &plan, &id)?;
            core.diff(&conversation, &plan, &id)?
        }),
        request => Route::Unmatched(request),
    })
}

fn review_task(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Review {
            conversation,
            plan,
            id,
            revision_token,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            if let Some(expected) = &revision_token {
                match core.diff_revision_token(&conversation, &plan, &id) {
                    Ok(current) => require(expected == &current, "stale_revision")?,
                    Err(_) => return Err(Error::Invalid("stale_revision".into())),
                }
            }
            validate_conversation_plan(core, &conversation, &plan)?;
            validate_plan_task(core, &plan, &id)?;
            core.review_change(&conversation, &plan, &id)?;
            Value::Null
        }),
        request => Route::Unmatched(request),
    })
}
