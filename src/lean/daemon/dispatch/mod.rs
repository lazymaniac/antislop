use super::*;

#[derive(Debug)]
pub(super) enum Route {
    Handled(Value),
    Unmatched(Request),
}

pub(super) fn handle(
    core: &Arc<Core>,
    project: &Path,
    config: &Path,
    request: Request,
    frontend: Option<(&str, &super::lease::Leases)>,
) -> Result<Value> {
    ensure_accepting(core, &request)?;
    let scope = super::lease::scope(&request);
    let observer = match (frontend, scope) {
        (Some((client, leases)), Some((conversation, access))) => {
            leases.access(conversation, client, access)?
        }
        _ => false,
    };
    let follows = matches!(scope, Some((_, super::lease::Access::Follow)));
    let mut reply = handle_request(core, project, config, request)?;
    if follows && let Some(reply) = reply.as_object_mut() {
        reply.insert("observer".into(), json!(observer));
    }
    Ok(reply)
}

fn handle_request(
    core: &Arc<Core>,
    project: &Path,
    config: &Path,
    request: Request,
) -> Result<Value> {
    let context = Context {
        core,
        project,
        config,
    };
    let operation = match &request {
        Request::Conversation(request) => request
            .operation()
            .map(|(conversation, id, kind)| (conversation.to_owned(), id.to_owned(), kind)),
        _ => None,
    };
    match operation {
        Some((conversation, id, kind)) => core.store.idempotent(&conversation, &id, kind, || {
            route_request(&context, request)
        }),
        None => route_request(&context, request),
    }
}

fn route_request(context: &Context<'_>, request: Request) -> Result<Value> {
    match route_all(context, request)? {
        Route::Handled(value) => Ok(value),
        Route::Unmatched(_) => Err(Error::Invalid("unhandled daemon request".into())),
    }
}

fn ensure_accepting(core: &Core, request: &Request) -> Result<()> {
    require(
        !core.state.join("stopped").exists()
            || matches!(
                request,
                Request::Control(crate::lean::daemon::ControlRequest::Identity)
                    | Request::Control(crate::lean::daemon::ControlRequest::Shutdown)
                    | Request::Control(crate::lean::daemon::ControlRequest::ForceShutdown)
            ),
        "the daemon is stopping",
    )
}

type Router = for<'a> fn(&Context<'a>, Request) -> Result<Route>;

fn route_all(context: &Context<'_>, request: Request) -> Result<Route> {
    route_pipeline(
        context,
        request,
        &[
            ledger::route,
            config::route,
            control::route,
            planning::route,
            conversation::route,
            terminal::route,
            history::route,
        ],
    )
}

fn route_pipeline(context: &Context<'_>, request: Request, routers: &[Router]) -> Result<Route> {
    let mut request = request;
    for router in routers {
        match router(context, request)? {
            Route::Handled(value) => return Ok(Route::Handled(value)),
            Route::Unmatched(unmatched) => request = unmatched,
        }
    }
    Ok(Route::Unmatched(request))
}

pub(super) struct Context<'a> {
    pub(super) core: &'a Arc<Core>,
    pub(super) project: &'a Path,
    pub(super) config: &'a Path,
}

mod config;
mod control;
mod conversation;
mod history;
mod ledger;
mod planning;
mod terminal;

fn validate_project_conversation(core: &Core, project: &Path, conversation: &str) -> Result<()> {
    let conv = core.store.conversation(conversation)?;
    let owner = core.store.repository_id(&project.to_string_lossy())?;
    let target = core.store.repository_id(&conv.project)?;
    require(
        conv.project == project.to_string_lossy() || (owner.is_some() && owner == target),
        &format!(
            "conversation {conversation} does not belong to project {}",
            project.display()
        ),
    )?;
    Ok(())
}

fn validate_conversation_plan(core: &Core, conversation: &str, plan: &str) -> Result<()> {
    require(
        core.store
            .plan_belongs_to_conversation(conversation, plan)?,
        &format!("plan {plan} does not belong to conversation {conversation}"),
    )?;
    Ok(())
}

fn validate_plan_task(core: &Core, plan: &str, task_id: &str) -> Result<()> {
    require(
        core.store.task_in_plan(plan, task_id)?,
        &format!("task {task_id} does not belong to plan {plan}"),
    )?;
    Ok(())
}
