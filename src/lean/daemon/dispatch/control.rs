use super::*;

pub(super) fn route(context: &Context<'_>, request: Request) -> Result<Route> {
    route_pipeline(
        context,
        request,
        &[
            identity,
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
