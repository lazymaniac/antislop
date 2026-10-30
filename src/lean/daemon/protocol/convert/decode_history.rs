mod content;
mod events;
mod task;
use super::super::{Request, WireRequest};

pub(super) fn decode_history(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    task::decode_group_task(wire)
        .or_else(|wire| content::decode_group_content(*wire))
        .or_else(|wire| events::decode_group_events(*wire))
}
