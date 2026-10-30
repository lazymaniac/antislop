use super::*;

pub(super) fn route(context: &Context<'_>, request: Request) -> Result<Route> {
    route_pipeline(
        context,
        request,
        &[
            comment_route,
            comment_scoped_route,
            comment_status_route,
            comments_route,
            comment_at_route,
        ],
    )
}

fn comment_route(context: &Context<'_>, request: Request) -> Result<Route> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Comment {
            conversation,
            target,
            text,
        }) => {
            validate_project_conversation(context.core, context.project, &conversation)?;
            Ok(Route::Handled(json!(context.core.comment(
                &conversation,
                &target,
                &text
            )?)))
        }
        request => Ok(Route::Unmatched(request)),
    }
}

fn comment_scoped_route(context: &Context<'_>, request: Request) -> Result<Route> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::CommentScoped {
            conversation,
            plan,
            target,
            text,
        }) => {
            validate_project_conversation(context.core, context.project, &conversation)?;
            if let Some(plan) = &plan {
                validate_conversation_plan(context.core, &conversation, plan)?;
            }
            Ok(Route::Handled(json!(context.core.comment_scoped(
                &conversation,
                plan.as_deref(),
                &target,
                &text
            )?)))
        }
        request => Ok(Route::Unmatched(request)),
    }
}

fn comment_status_route(context: &Context<'_>, request: Request) -> Result<Route> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::CommentStatus {
            conversation,
            id,
            status,
        }) => {
            validate_project_conversation(context.core, context.project, &conversation)?;
            Ok(Route::Handled(json!(context.core.set_comment(
                &conversation,
                id,
                &status
            )?)))
        }
        request => Ok(Route::Unmatched(request)),
    }
}

fn comments_route(context: &Context<'_>, request: Request) -> Result<Route> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Comments {
            conversation,
            plan,
            status,
        }) => Ok(Route::Handled(comments(
            context,
            conversation,
            plan,
            status,
        )?)),
        request => Ok(Route::Unmatched(request)),
    }
}

fn comments(
    context: &Context<'_>,
    conversation: String,
    plan: Option<String>,
    status: Option<String>,
) -> Result<Value> {
    validate_project_conversation(context.core, context.project, &conversation)?;
    if let Some(plan) = &plan {
        validate_conversation_plan(context.core, &conversation, plan)?;
    }
    let filter = comment_status_filter(status.as_deref());
    let comments = load_comments(context.core, &conversation, plan.as_deref(), filter)?;
    decorate_comment_scopes(context.core, &conversation, comments)
}

fn comment_status_filter(status: Option<&str>) -> Option<&str> {
    match status {
        Some(status) if status.eq_ignore_ascii_case("all") => None,
        Some(status) => Some(status),
        None => Some("open"),
    }
}

fn load_comments(
    core: &Core,
    conversation: &str,
    plan: Option<&str>,
    status: Option<&str>,
) -> Result<Vec<super::super::super::store::Comment>> {
    match plan {
        Some(plan) => core.store.comments_by_plan(conversation, plan, status),
        None => core.store.comments_all(conversation, status),
    }
}

fn decorate_comment_scopes(
    core: &Core,
    conversation: &str,
    comments: Vec<impl serde::Serialize>,
) -> Result<Value> {
    let mut values = serde_json::to_value(comments)?;
    if let Some(values) = values.as_array_mut() {
        for comment in values {
            let scope = core
                .store
                .comment_scope(conversation, comment["id"].as_u64().unwrap_or(0))?;
            comment["round"] = scope["round"].clone();
            comment["run"] = scope["run"].clone();
        }
    }
    Ok(values)
}

fn comment_at_route(context: &Context<'_>, request: Request) -> Result<Route> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::CommentAt {
            conversation,
            plan,
            target,
            text,
            round,
            run,
        }) => Ok(Route::Handled(comment_at(
            context,
            conversation,
            plan,
            target,
            text,
            round,
            run,
        )?)),
        request => Ok(Route::Unmatched(request)),
    }
}

fn comment_at(
    context: &Context<'_>,
    conversation: String,
    plan: Option<String>,
    target: String,
    text: String,
    round: Option<u32>,
    run: Option<String>,
) -> Result<Value> {
    validate_project_conversation(context.core, context.project, &conversation)?;
    if let Some(plan) = &plan {
        validate_conversation_plan(context.core, &conversation, plan)?;
    }
    if let Some(run) = &run {
        context
            .core
            .store
            .assert_comment_run(&conversation, plan.as_deref(), &target, run)?;
    }
    validate_round_comment(context.core, plan.as_deref(), &target, round)?;
    Ok(json!(context.core.comment_at(
        &conversation,
        plan.as_deref(),
        &target,
        &text,
        round,
        run.as_deref()
    )?))
}

fn validate_round_comment(
    core: &Core,
    plan: Option<&str>,
    target: &str,
    round: Option<u32>,
) -> Result<()> {
    let Some(round) = round else { return Ok(()) };
    let plan = plan.ok_or_else(|| Error::Invalid("round comment requires a plan".into()))?;
    let (role, id) = target
        .split_once(':')
        .ok_or_else(|| Error::Invalid("round comment requires a task".into()))?;
    require(
        matches!(role, "worker" | "verifier" | "check" | "diff"),
        "round comment requires a task",
    )?;
    let id = id.split(':').next().unwrap_or(id);
    let task = core
        .store
        .task_in_plan_by_id(plan, id)?
        .ok_or_else(|| Error::Invalid("comment task is outside this plan".into()))?;
    core.store
        .task_round_with_references(&task.id, round, true)?;
    Ok(())
}
