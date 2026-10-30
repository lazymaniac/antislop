use super::super::*;
pub(super) fn route(context: &Context<'_>, request: Request) -> Result<Route> {
    route_pipeline(
        context,
        request,
        &[
            send,
            addressed_send_and_accept,
            scoped_accept,
            reject_plan,
            scoped_reject,
            review_recovery_and_resume,
            scoped_resume,
            restart_and_cancel,
            scoped_cancel,
            pause_and_mode,
        ],
    )
}

fn send(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Send { conversation, text } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            core.submit_addressed(&conversation, &text, None)?
        }),
        Request::SendToAgent { message } => Route::Handled({
            validate_project_conversation(core, project, &message.conversation)?;
            if let Some(plan) = &message.plan {
                validate_conversation_plan(core, &message.conversation, plan)?;
            }
            core.send_to_agent(&message)?
        }),
        request => Route::Unmatched(request),
    })
}

fn addressed_send_and_accept(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::SendAddressed {
            conversation,
            text,
            to,
        } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            core.submit_addressed(&conversation, &text, to.as_deref())?
        }),
        Request::Accept { conversation, plan } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            validate_conversation_plan(core, &conversation, &plan)?;
            json!(core.accept_plan(&conversation, &plan)?.len())
        }),
        request => Route::Unmatched(request),
    })
}

fn scoped_accept(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::AcceptScoped {
            conversation,
            plan,
            scope_hash,
        } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            if let Some(expected) = &scope_hash {
                match core.plan_scope_hash(&conversation, &plan) {
                    Ok(current) => require(expected == &current, "stale_scope")?,
                    Err(_) => return Err(Error::Invalid("stale_scope".into())),
                }
            }
            validate_conversation_plan(core, &conversation, &plan)?;
            json!(core.accept_plan(&conversation, &plan)?.len())
        }),
        request => Route::Unmatched(request),
    })
}

fn reject_plan(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Reject {
            conversation,
            plan,
            reason,
        } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            validate_conversation_plan(core, &conversation, &plan)?;
            core.reject_plan(&conversation, &plan, reason.as_deref())?;
            Value::Null
        }),
        request => Route::Unmatched(request),
    })
}

fn scoped_reject(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::RejectScoped {
            conversation,
            plan,
            reason,
            scope_hash,
        } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            if let Some(expected) = &scope_hash {
                match core.plan_scope_hash(&conversation, &plan) {
                    Ok(current) => require(expected == &current, "stale_scope")?,
                    Err(_) => return Err(Error::Invalid("stale_scope".into())),
                }
            }
            validate_conversation_plan(core, &conversation, &plan)?;
            core.reject_plan(&conversation, &plan, reason.as_deref())?;
            Value::Null
        }),
        request => Route::Unmatched(request),
    })
}

fn review_recovery_and_resume(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::ReviewComments { conversation, plan } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            validate_conversation_plan(core, &conversation, &plan)?;
            json!(core.review_comments(&conversation, &plan)?)
        }),
        Request::Recover(req) => Route::Handled({
            validate_project_conversation(core, project, &req.conversation)?;
            validate_conversation_plan(core, &req.conversation, &req.plan)?;
            validate_plan_task(core, &req.plan, &req.task)?;
            json!(core.recover_task(req)?)
        }),
        Request::Resume { conversation, plan } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            validate_conversation_plan(core, &conversation, &plan)?;
            json!(core.resume_plan(&conversation, &plan)?.len())
        }),
        request => Route::Unmatched(request),
    })
}

fn scoped_resume(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::ResumeScoped {
            conversation,
            plan,
            scope_hash,
        } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            if let Some(expected) = &scope_hash {
                match core.plan_scope_hash(&conversation, &plan) {
                    Ok(current) => require(expected == &current, "stale_scope")?,
                    Err(_) => return Err(Error::Invalid("stale_scope".into())),
                }
            }
            validate_conversation_plan(core, &conversation, &plan)?;
            json!(core.resume_plan(&conversation, &plan)?.len())
        }),
        request => Route::Unmatched(request),
    })
}

fn restart_and_cancel(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::RestartOrchestrator { conversation } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            core.restart_orchestrator(&conversation)?;
            json!("restarting the orchestrator in a fresh session")
        }),
        Request::Cancel { conversation } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            core.cancel(&conversation);
            Value::Null
        }),
        request => Route::Unmatched(request),
    })
}

fn scoped_cancel(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::CancelScoped {
            conversation,
            scope_hash,
        } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            if let Some(expected) = &scope_hash {
                match core.conversation_scope_hash(&conversation) {
                    Ok(current) => require(expected == &current, "stale_scope")?,
                    Err(_) => return Err(Error::Invalid("stale_scope".into())),
                }
            }
            core.cancel(&conversation);
            Value::Null
        }),
        request => Route::Unmatched(request),
    })
}

fn pause_and_mode(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Pause {
            conversation,
            paused,
        } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            core.pause(&conversation, paused)?;
            Value::Null
        }),
        Request::Mode {
            conversation,
            autonomous,
        } => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            core.set_mode(&conversation, autonomous)?;
            Value::Null
        }),
        request => Route::Unmatched(request),
    })
}
