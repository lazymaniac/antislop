use super::super::*;
pub(super) fn route(context: &Context<'_>, request: Request) -> Result<Route> {
    route_pipeline(
        context,
        request,
        &[
            plans_and_plan,
            task,
            scoped_task,
            task_checks,
            task_diff,
            review_task,
            scoped_task_review,
        ],
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
        Request::Conversation(crate::lean::daemon::ConversationRequest::Task { plan, id }) => {
            Route::Handled({
                let conv = core.store.plan_conversation(&plan)?;
                validate_project_conversation(core, project, &conv)?;
                validate_plan_task(core, &plan, &id)?;
                json!(core.task(&plan, &id)?)
            })
        }
        request => Route::Unmatched(request),
    })
}

fn scoped_task(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::TaskScoped {
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
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            validate_conversation_plan(core, &conversation, &plan)?;
            validate_plan_task(core, &plan, &id)?;
            core.review_change(&conversation, &plan, &id)?;
            Value::Null
        }),
        request => Route::Unmatched(request),
    })
}

fn scoped_task_review(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::ReviewScoped {
            conversation,
            plan,
            id,
            scope_hash,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            if let Some(expected) = &scope_hash {
                match core.diff_scope_hash(&conversation, &plan, &id) {
                    Ok(current) => require(expected == &current, "stale_scope")?,
                    Err(_) => return Err(Error::Invalid("stale_scope".into())),
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
