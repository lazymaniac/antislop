use super::*;
mod actions;
mod tasks;
pub(super) fn route(context: &Context<'_>, request: Request) -> Result<Route> {
    route_pipeline(context, request, &[actions::route, tasks::route])
}
