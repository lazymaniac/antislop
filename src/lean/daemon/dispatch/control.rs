use super::*;

pub(super) fn route(context: &Context<'_>, request: Request) -> Result<Route> {
    route_pipeline(
        context,
        request,
        &[
            identity,
            authorize_task_execution,
            runtime_notice,
            shutdown,
            force_shutdown,
            conversations_and_start,
            trust_status,
            trust,
        ],
    )
}

fn identity(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    let config = context.config;
    Ok(match request {
        Request::Control(crate::lean::daemon::ControlRequest::Identity) => Route::Handled({
            let _guard = core
                .config_update
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            json!({
                "build": build(),
                "busy": core.busy(),
                "pid": std::process::id(),
                "project": project,
                "config": config,
                "config_modified": core.config_modified.load(Ordering::SeqCst),
            })
        }),
        request => Route::Unmatched(request),
    })
}

fn shutdown(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    Ok(match request {
        Request::Control(crate::lean::daemon::ControlRequest::Shutdown) => Route::Handled({
            require(
                !core.busy(),
                "the daemon is executing work; use antislop stop --force to cancel it",
            )?;
            std::fs::write(core.state.join("stopped"), [])?;
            core.native.shutdown()?;
            Value::Null
        }),
        request => Route::Unmatched(request),
    })
}

fn force_shutdown(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    Ok(match request {
        Request::Control(crate::lean::daemon::ControlRequest::ForceShutdown) => Route::Handled({
            std::fs::write(core.state.join("stopped"), [])?;
            let deadline = Instant::now() + Duration::from_secs(5);
            while core.busy() && Instant::now() < deadline {
                for conversation in core.conversations()? {
                    core.cancel(&conversation.id);
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            // End any CLI still alive before recording interrupted work.
            for (pid, started_ms, provider) in core.store.running_processes()? {
                super::super::super::agent::reap(pid, started_ms, &provider);
            }
            core.store.interrupt_running_turns()?;
            core.store.interrupt_unfinished_tasks()?;
            core.native.shutdown()?;
            Value::Null
        }),
        request => Route::Unmatched(request),
    })
}

fn conversations_and_start(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Control(crate::lean::daemon::ControlRequest::Conversations) => {
            Route::Handled(json!(core.conversations()?))
        }
        Request::Control(crate::lean::daemon::ControlRequest::Start) => {
            Route::Handled(json!(core.start(project)?))
        }
        Request::Control(crate::lean::daemon::ControlRequest::StartIn {
            project: target_project,
        }) => Route::Handled({
            let target_path = Path::new(&target_project).canonicalize()?;
            require(
                super::super::super::trust::Registry::from_env()?.is_trusted(&target_path),
                &format!(
                    "{} is not a trusted directory: run antislop there in a terminal first",
                    target_path.display()
                ),
            )?;
            core.store.trust_project(&target_path.to_string_lossy())?;
            json!(core.start(&target_path)?)
        }),
        request => Route::Unmatched(request),
    })
}

fn trust_status(context: &Context<'_>, request: Request) -> Result<Route> {
    let project = context.project;
    Ok(match request {
        Request::Control(crate::lean::daemon::ControlRequest::TrustStatus) => Route::Handled({
            json!({
                "project": project.to_string_lossy(),
                "trusted": super::super::super::trust::Registry::from_env()?.is_trusted(project),
            })
        }),
        request => Route::Unmatched(request),
    })
}

fn trust(context: &Context<'_>, request: Request) -> Result<Route> {
    let project = context.project;
    Ok(match request {
        Request::Control(crate::lean::daemon::ControlRequest::Trust { trusted }) => {
            Route::Handled({
                let registry = super::super::super::trust::Registry::from_env()?;
                if trusted {
                    registry.trust(project)?;
                } else {
                    registry.untrust(project)?;
                }
                json!({
                    "project": project.to_string_lossy(),
                    "trusted": trusted,
                })
            })
        }
        request => Route::Unmatched(request),
    })
}

fn authorize_task_execution(context: &Context<'_>, request: Request) -> Result<Route> {
    Ok(match request {
        Request::Control(crate::lean::daemon::ControlRequest::AuthorizeTaskExecution {
            authorization,
        }) => {
            let _guard = context
                .core
                .graph_updates
                .lock()
                .expect("graph update mutex");
            let result = context
                .core
                .store
                .authorize_task_execution(&authorization)?;
            let text = format!(
                "Explicit user/client execution authorization registered for task {} under answered decision {}. Only this exact contract and its next input generation receive a scoped exception to requirement {}'s NoNewExecutions; all other constraints and deferred work remain enforced. The writer is not yet released or approved: submit the independently reviewed revision. Preserve the following contract exactly; change only revision.reason to a fresh reason naming the scoped authorization so an immutable rejected receipt is not replayed. Do not ask the user for the same permission again. Authorized contract:\n{}",
                authorization.contract.id,
                authorization.decision,
                authorization.requirement,
                serde_json::to_string(&authorization.contract)?
            );
            context.core.store.add_message(
                &authorization.conversation,
                crate::lean::RUNTIME,
                &text,
            )?;
            drop(_guard);
            context.core.wake(&authorization.conversation);
            Route::Handled(result)
        }
        request => Route::Unmatched(request),
    })
}

fn runtime_notice(context: &Context<'_>, request: Request) -> Result<Route> {
    Ok(match request {
        Request::Control(crate::lean::daemon::ControlRequest::RuntimeNotice {
            conversation,
            text,
        }) => {
            validate_project_conversation(context.core, context.project, &conversation)?;
            require(
                !text.trim().is_empty() && text.len() <= 8192,
                "runtime notice must contain 1–8192 bytes",
            )?;
            let message =
                context
                    .core
                    .store
                    .add_message(&conversation, crate::lean::RUNTIME, &text)?;
            context.core.wake(&conversation);
            Route::Handled(json!({"recorded":true,"message":message.seq}))
        }
        request => Route::Unmatched(request),
    })
}
