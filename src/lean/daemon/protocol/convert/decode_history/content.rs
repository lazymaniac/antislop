use super::super::super::{Request, WireRequest};

fn decode_history_content(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::HistoryContent {
            conversation,
            owner,
            offset,
        } => Ok(Request::History(
            crate::lean::daemon::HistoryRequest::HistoryContent {
                conversation,
                owner,
                offset,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_history_messages(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::HistoryMessages {
            conversation,
            scope,
            after,
            limit,
        } => Ok(Request::History(
            crate::lean::daemon::HistoryRequest::HistoryMessages {
                conversation,
                scope,
                after,
                limit,
            },
        )),
        other => Err(Box::new(other)),
    }
}

pub(super) fn decode_group_content(
    wire: WireRequest,
) -> std::result::Result<Request, Box<WireRequest>> {
    decode_history_content(wire).or_else(|wire| decode_history_messages(*wire))
}
