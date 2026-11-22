use super::*;

pub(super) fn route(context: &Context<'_>, request: Request) -> Result<Route> {
    route_pipeline(
        context,
        request,
        &[
            task_rounds,
            task_round,
            run_context,
            session_action,
            session_events,
            history_content,
            history_messages,
            session_content,
            result_documents,
            result_document_content,
            events,
            paged_events_and_sessions,
            history_and_mail,
        ],
    )
}

fn session_action(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Session(crate::lean::daemon::SessionRequest::SessionAction {
            conversation,
            action,
        }) => Route::Handled((|| -> Result<serde_json::Value> {
            validate_project_conversation(core, project, &conversation)?;
            let completed: bool = core.store.db().query_row(
                "SELECT EXISTS(SELECT 1 FROM events WHERE conversation_id=?1 AND kind='session_completed' AND json_extract(payload,'$.cleanup_complete')=1)",
                [&conversation],
                |row| row.get(0),
            )?;
            require(completed, "session actions require a completed session")?;
            let text = match action {
                crate::lean::daemon::SessionActionKind::CreatePr => {
                    "Continue this completed session by creating a pull request for its preserved branch."
                }
                crate::lean::daemon::SessionActionKind::MergeWithMain => {
                    "Continue this completed session by merging its preserved branch with main."
                }
                crate::lean::daemon::SessionActionKind::ContinueWork => {
                    "Continue work in this session, preserving its branch and prior results."
                }
            };
            let message = core.store.add_message(&conversation, "user", text)?;
            core.wake(&conversation);
            Ok(json!(crate::lean::daemon::SessionActionResult::Queued {
                message: message.seq,
            }))
        })().unwrap_or_else(|error| json!(crate::lean::daemon::SessionActionResult::Failed {
            details: error.to_string(),
        }))),
        request => Route::Unmatched(request),
    })
}

fn result_documents(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Session(crate::lean::daemon::SessionRequest::ResultDocuments { conversation }) => {
            Route::Handled({
                validate_project_conversation(core, project, &conversation)?;
                json!(core.store.result_documents(&conversation)?)
            })
        }
        request => Route::Unmatched(request),
    })
}

fn result_document_content(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Session(crate::lean::daemon::SessionRequest::ResultDocumentContent {
            conversation,
            id,
            offset,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            json!(
                core.store
                    .result_document_chunk(&conversation, &id, offset)?
            )
        }),
        request => Route::Unmatched(request),
    })
}

fn task_rounds(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::History(crate::lean::daemon::HistoryRequest::TaskRounds {
            conversation,
            plan,
            id,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            validate_conversation_plan(core, &conversation, &plan)?;
            require(
                uuid::Uuid::parse_str(&id).is_ok(),
                "task history requires a task UUID",
            )?;
            validate_plan_task(core, &plan, &id)?;
            json!(core.store.task_rounds(&id)?)
        }),
        request => Route::Unmatched(request),
    })
}

fn task_round(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::History(crate::lean::daemon::HistoryRequest::TaskRound {
            conversation,
            plan,
            id,
            round,
            references,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            validate_conversation_plan(core, &conversation, &plan)?;
            require(
                uuid::Uuid::parse_str(&id).is_ok(),
                "task history requires a task UUID",
            )?;
            validate_plan_task(core, &plan, &id)?;
            core.store
                .task_round_with_references(&id, round, references.unwrap_or(false))?
        }),
        request => Route::Unmatched(request),
    })
}

fn run_context(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Session(crate::lean::daemon::SessionRequest::RunContext {
            conversation,
            turn,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            json!(core.store.run_context(&conversation, &turn)?)
        }),
        request => Route::Unmatched(request),
    })
}

fn session_events(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Session(crate::lean::daemon::SessionRequest::SessionEvents {
            conversation,
            turn,
            after,
            limit,
            include_messages,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            let mut page = core.store.session_events_with_messages(
                &conversation,
                &turn,
                after,
                limit.unwrap_or(100),
                include_messages.unwrap_or(true),
            )?;
            if after == 0 {
                page["questions"] = json!(core.store.turn_question_history(&conversation, &turn)?);
            }
            page
        }),
        request => Route::Unmatched(request),
    })
}

fn history_content(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::History(crate::lean::daemon::HistoryRequest::HistoryContent {
            conversation,
            owner,
            offset,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            json!(core.store.history_content(&conversation, &owner, offset)?)
        }),
        request => Route::Unmatched(request),
    })
}

fn history_messages(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::History(crate::lean::daemon::HistoryRequest::HistoryMessages {
            conversation,
            scope,
            after,
            limit,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            core.store
                .history_messages(&conversation, &scope, after, limit.unwrap_or(100))?
        }),
        request => Route::Unmatched(request),
    })
}

fn session_content(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Session(crate::lean::daemon::SessionRequest::SessionContent {
            conversation,
            event,
            field,
            offset,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            json!(
                core.store
                    .session_content(&conversation, event, &field, offset)?
            )
        }),
        request => Route::Unmatched(request),
    })
}
fn events(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::History(crate::lean::daemon::HistoryRequest::Events {
            conversation,
            after,
            wait_ms,
        }) => Route::Handled(handle_events(
            core,
            project,
            &conversation,
            after,
            wait_ms,
            None,
        )?),
        request => Route::Unmatched(request),
    })
}

fn paged_events_and_sessions(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::History(crate::lean::daemon::HistoryRequest::EventsPaged {
            conversation,
            after,
            wait_ms,
            limit,
        }) => Route::Handled(handle_events(
            core,
            project,
            &conversation,
            after,
            wait_ms,
            limit,
        )?),
        Request::Session(crate::lean::daemon::SessionRequest::Sessions { conversation }) => {
            Route::Handled({
                validate_project_conversation(core, project, &conversation)?;
                json!(core.sessions(&conversation)?)
            })
        }
        Request::Session(crate::lean::daemon::SessionRequest::DeleteSession { conversation }) => {
            Route::Handled({
                validate_project_conversation(core, project, &conversation)?;
                core.delete_session(&conversation)?;
                Value::Null
            })
        }
        request => Route::Unmatched(request),
    })
}

fn history_and_mail(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::History(crate::lean::daemon::HistoryRequest::History {
            conversation,
            after,
            limit,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            json!(core.history(&conversation, after, limit)?)
        }),
        Request::History(crate::lean::daemon::HistoryRequest::Mail { conversation }) => {
            Route::Handled({
                validate_project_conversation(core, project, &conversation)?;
                json!(core.store.mail(&conversation)?)
            })
        }
        request => Route::Unmatched(request),
    })
}

fn handle_events(
    core: &Arc<Core>,
    project: &Path,
    conversation: &str,
    after: u64,
    wait_ms: u64,
    limit: Option<usize>,
) -> Result<Value> {
    validate_project_conversation(core, project, conversation)?;
    let events = wait_for_events(core, conversation, after, wait_ms, limit)?;
    events_result(core, conversation, after, events)
}

fn wait_for_events(
    core: &Arc<Core>,
    conversation: &str,
    after: u64,
    wait_ms: u64,
    limit: Option<usize>,
) -> Result<Vec<super::super::super::store::Event>> {
    let limit = limit.unwrap_or(EVENTS_PER_REPLY).min(EVENTS_PER_REPLY);
    let deadline = Instant::now() + Duration::from_millis(wait_ms).min(LONGEST_WAIT);
    loop {
        let events = core
            .store
            .events_since_budget(conversation, after, limit, REPLY_BYTES)?;
        if !events.is_empty() || Instant::now() >= deadline {
            let take = reply_event_count(&events, REPLY_BYTES);
            return Ok(events.into_iter().take(take).collect());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn events_result(
    core: &Arc<Core>,
    conversation: &str,
    after: u64,
    events: Vec<super::super::super::store::Event>,
) -> Result<Value> {
    let (oldest_seq, latest_seq, total) = core.store.event_bounds(conversation)?;
    let next_after = events.last().map(|event| event.seq);
    let has_more = next_after.is_some_and(|last_seq| last_seq < latest_seq);
    let history_unavailable = total > 0
        && (after > latest_seq || (oldest_seq > 1 && after < oldest_seq.saturating_sub(1)));
    Ok(json!({
        "events": events,
        "idle": core.frontend_idle(conversation)?,
        "has_more": has_more,
        "history_unavailable": history_unavailable,
        "oldest_seq": oldest_seq,
        "latest_seq": latest_seq,
        "total": total,
    }))
}
