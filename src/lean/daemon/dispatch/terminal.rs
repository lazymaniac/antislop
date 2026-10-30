use super::*;

pub(super) fn route(context: &Context<'_>, request: Request) -> Result<Route> {
    route_pipeline(
        context,
        request,
        &[
            terminal_open,
            terminal_read,
            terminal_input,
            terminal_resize,
            terminal_detach,
        ],
    )
}

fn terminal_open(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Terminal(crate::lean::daemon::TerminalRequest::TerminalOpen {
            conversation,
            role,
            columns,
            rows,
            retry,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            json!(core.terminal_open(&conversation, &role, columns, rows, retry)?)
        }),
        request => Route::Unmatched(request),
    })
}

fn terminal_read(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Terminal(crate::lean::daemon::TerminalRequest::TerminalRead {
            conversation,
            role,
            handle,
            after,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            json!(core.terminal_read(&conversation, &role, &handle, after)?)
        }),
        request => Route::Unmatched(request),
    })
}

fn terminal_input(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Terminal(crate::lean::daemon::TerminalRequest::TerminalInput {
            conversation,
            role,
            handle,
            sequence,
            input,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            json!(core.terminal_input(&conversation, &role, &handle, sequence, input)?)
        }),
        request => Route::Unmatched(request),
    })
}

fn terminal_resize(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Terminal(crate::lean::daemon::TerminalRequest::TerminalResize {
            conversation,
            role,
            handle,
            columns,
            rows,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            json!(core.terminal_resize(&conversation, &role, &handle, columns, rows)?)
        }),
        request => Route::Unmatched(request),
    })
}

fn terminal_detach(context: &Context<'_>, request: Request) -> Result<Route> {
    let core = context.core;
    let project = context.project;
    Ok(match request {
        Request::Terminal(crate::lean::daemon::TerminalRequest::TerminalDetach {
            conversation,
            role,
            handle,
        }) => Route::Handled({
            validate_project_conversation(core, project, &conversation)?;
            json!(core.terminal_detach(&conversation, &role, &handle)?)
        }),
        request => Route::Unmatched(request),
    })
}
