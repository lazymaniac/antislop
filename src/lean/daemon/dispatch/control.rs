use super::*;

pub(super) fn route(context: &Context<'_>, request: Request) -> Result<Route> {
    route_pipeline(
        context,
        request,
        &[
            activation,
            identity,
            runtime_notice,
            shutdown,
            force_shutdown,
            conversations_and_start,
        ],
    )
}

fn activation(context: &Context<'_>, request: Request) -> Result<Route> {
    use crate::lean::daemon::{ControlRequest, activation};
    Ok(match request {
        Request::Control(ControlRequest::ActivateRuntime {
            request_id,
            executable,
            path,
            conversation,
            task,
        }) => Route::Handled(activation::request(
            context.core,
            context.project,
            context.config,
            Path::new(&executable),
            path.as_deref(),
            conversation.as_deref(),
            task.as_deref(),
            request_id.as_deref(),
        )?),
        Request::Control(ControlRequest::RuntimeActivationStatus) => {
            Route::Handled(activation::status(&context.core.state, None)?)
        }
        Request::Control(ControlRequest::ClassifyRuntimeFailure {
            conversation,
            message,
            reason,
        }) => {
            validate_project_conversation(context.core, context.project, &conversation)?;
            Route::Handled(context.core.store.classify_preflight_failure(
                &conversation,
                message,
                &reason,
            )?)
        }
        Request::Control(ControlRequest::RuntimeActivationOperation { request_id }) => {
            Route::Handled(activation::status(&context.core.state, Some(&request_id))?)
        }
        Request::Control(ControlRequest::CancelRuntimeActivation { request_id }) => {
            Route::Handled(activation::cancel_operation(context.core, &request_id)?)
        }
        request => Route::Unmatched(request),
    })
}

fn identity(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    let config = context.config;
    Ok(match request {
        Request::Control(crate::lean::daemon::ControlRequest::Identity) => Route::Handled({
            let _guard = core.config.updating();
            json!({
                "build": build(),
                "capabilities": super::super::version_info()["capabilities"],
                "busy": core.busy(),
                "pid": std::process::id(),
                "project": project,
                "project_identity": core.store.repository_id(&project.to_string_lossy())?,
                "config": config,
                "config_modified": core.config.modified(),
                "launch_path": std::env::var("PATH").ok(),
                "launch_error": crate::lean::doctor::preflight(&core.config_snapshot()).err().map(|e|e.to_string()),
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
            core.cancel_operations();
            super::super::activation::cancel_all(core)?;
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
            core.cancel_operations();
            super::super::activation::cancel_all(core)?;
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
