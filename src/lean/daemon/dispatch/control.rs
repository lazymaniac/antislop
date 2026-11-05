use super::*;

pub(super) fn route(context: &Context<'_>, request: Request) -> Result<Route> {
    route_pipeline(
        context,
        request,
        &[
            identity,
            runtime_notice,
            shutdown,
            force_shutdown,
            conversations_and_start,
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
                "project_identity": core.store.repository_id(&project.to_string_lossy())?,
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
                core.store
                    .repository_id(&project.to_string_lossy())?
                    .as_deref()
                    == Some(crate::lean::paths::project_identity(&target_path)?.as_str()),
                "project belongs to another repository; open its own runtime state",
            )?;
            json!(core.start(&target_path)?)
        }),
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
