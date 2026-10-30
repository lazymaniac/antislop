mod content;
mod events;
mod task;
use super::super::{Request, WireRequest};

pub(super) fn encode_history(request: Request) -> WireRequest {
    task::encode_group_task(request)
        .or_else(|request| content::encode_group_content(*request))
        .or_else(|request| events::encode_group_events(*request))
        .unwrap_or_else(|_| unreachable!("request routed to wrong protocol group"))
}
