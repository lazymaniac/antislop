use super::super::*;
use crate::lean::daemon::ConversationRequest;
pub(super) fn route(context: &Context<'_>, request: Request) -> Result<Route> {
    route_pipeline(
        context,
        request,
        &[
            sources,
            send,
            addressed_send,
            reject_plan,
            review_recovery_and_resume,
            resume_plan,
            restart_and_cancel,
            cancel_work,
            pause,
            approvals,
            revise_and_team,
        ],
    )
}

fn send(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Conversation(ConversationRequest::SendToAgent { message }) => Route::Handled({
            validate_project_conversation(core, project, &message.conversation)?;
            if let Some(plan) = &message.plan {
                validate_conversation_plan(core, &message.conversation, plan)?;
            }
            core.send_to_agent(&message)?
        }),
        request => Route::Unmatched(request),
    })
}

fn addressed_send(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Conversation(ConversationRequest::Send {
            conversation,
            operation_id: _,
            text,
            to,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            core.submit_addressed(&conversation, &text, to.as_deref())?
        }),
        request => Route::Unmatched(request),
    })
}

fn reject_plan(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Conversation(ConversationRequest::Reject {
            conversation,
            operation_id: _,
            plan,
            reason,
            revision_token,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            if let Some(expected) = &revision_token {
                match core.plan_revision_token(&conversation, &plan) {
                    Ok(current) => require(expected == &current, "stale_revision")?,
                    Err(_) => return Err(Error::Invalid("stale_revision".into())),
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
        Request::Conversation(ConversationRequest::ReviewComments {
            conversation,
            operation_id: _,
            plan,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            validate_conversation_plan(core, &conversation, &plan)?;
            json!(core.review_comments(&conversation, &plan)?)
        }),
        Request::Conversation(ConversationRequest::Recover(req)) => Route::Handled({
            validate_project_conversation(core, project, &req.conversation)?;
            validate_conversation_plan(core, &req.conversation, &req.plan)?;
            validate_plan_task(core, &req.plan, &req.task)?;
            json!(core.recover_task(req)?)
        }),
        request => Route::Unmatched(request),
    })
}

fn resume_plan(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Conversation(ConversationRequest::Resume {
            conversation,
            operation_id: _,
            plan,
            revision_token,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            require(
                core.store.pending_scope_revision(&plan)?.is_none() || revision_token.is_some(),
                "scope amendment acceptance requires its reviewed revision_token",
            )?;
            if let Some(expected) = &revision_token {
                match core.plan_revision_token(&conversation, &plan) {
                    Ok(current) => require(expected == &current, "stale_revision")?,
                    Err(_) => return Err(Error::Invalid("stale_revision".into())),
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
        Request::Conversation(ConversationRequest::RestartOrchestrator {
            conversation,
            operation_id: _,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            core.restart_orchestrator(&conversation)?;
            json!("restarting the orchestrator in a fresh session")
        }),
        request => Route::Unmatched(request),
    })
}

fn cancel_work(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Conversation(ConversationRequest::Cancel {
            conversation,
            operation_id: _,
            revision_token,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            if let Some(expected) = &revision_token {
                match core.conversation_revision_token(&conversation) {
                    Ok(current) => require(expected == &current, "stale_revision")?,
                    Err(_) => return Err(Error::Invalid("stale_revision".into())),
                }
            }
            core.cancel(&conversation);
            Value::Null
        }),
        request => Route::Unmatched(request),
    })
}

fn pause(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Conversation(ConversationRequest::Pause {
            conversation,
            paused,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            core.pause(&conversation, paused)?;
            Value::Null
        }),
        request => Route::Unmatched(request),
    })
}

fn approvals(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        // The lease itself changed before routing; only the conversation is checked.
        Request::Conversation(ConversationRequest::TakeControl { conversation }) => {
            Route::Handled({
                validate_project_conversation(core, project, &conversation)?;
                Value::Null
            })
        }
        Request::Conversation(ConversationRequest::SetApprovalMode { conversation, mode }) => {
            Route::Handled({
                validate_project_conversation(core, project, &conversation)?;
                core.set_approval_mode(&conversation, &mode)?;
                Value::Null
            })
        }
        Request::Conversation(ConversationRequest::AnswerApproval {
            conversation,
            operation_id: _,
            request,
            choice,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            core.answer_approval(&conversation, &request, &choice)?;
            Value::Null
        }),
        request => Route::Unmatched(request),
    })
}

fn revise_and_team(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Conversation(ConversationRequest::RevisePlan {
            conversation,
            operation_id,
            revision,
            request_ai_review,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            validate_conversation_plan(core, &conversation, &revision.plan)?;
            let key = operation_id
                .as_deref()
                .filter(|id| !id.is_empty())
                .map_or_else(crate::id, str::to_owned);
            core.apply_user_revision(&conversation, &key, revision, request_ai_review)?
        }),
        Request::Conversation(ConversationRequest::SelectTeam {
            conversation,
            team,
            reason,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            let parsed = crate::lean::tasks::Team::parse(&team)?;
            let reason = if reason.trim().is_empty() {
                format!("User selected `{}` rigor mode in UI", parsed.name())
            } else {
                reason
            };
            core.select_rigor(&conversation, parsed, &reason)?;
            json!({"team": parsed.name(), "reason": reason})
        }),
        request => Route::Unmatched(request),
    })
}

fn sources(context: &Context<'_>, request: Request) -> Result<Route> {
    Ok(match request {
        Request::Conversation(ConversationRequest::AddSource { conversation, text }) => {
            validate_project_conversation(context.core, context.project, &conversation)?;
            Route::Handled(json!(
                context.core.store.register_source(&conversation, &text)?
            ))
        }
        Request::Conversation(ConversationRequest::GetSource {
            conversation,
            source_id,
        }) => {
            validate_project_conversation(context.core, context.project, &conversation)?;
            Route::Handled(
                json!({"text":context.core.store.source_text(&conversation,&source_id)?}),
            )
        }
        other => Route::Unmatched(other),
    })
}
