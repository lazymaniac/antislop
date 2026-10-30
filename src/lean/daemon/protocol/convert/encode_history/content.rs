use super::super::super::{Request, WireRequest};

fn encode_history_content(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::History(crate::lean::daemon::HistoryRequest::HistoryContent {
            conversation,
            owner,
            offset,
        }) => Ok(WireRequest::HistoryContent {
            conversation,
            owner,
            offset,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_history_messages(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::History(crate::lean::daemon::HistoryRequest::HistoryMessages {
            conversation,
            scope,
            after,
            limit,
        }) => Ok(WireRequest::HistoryMessages {
            conversation,
            scope,
            after,
            limit,
        }),
        other => Err(Box::new(other)),
    }
}

pub(super) fn encode_group_content(
    request: Request,
) -> std::result::Result<WireRequest, Box<Request>> {
    encode_history_content(request).or_else(|request| encode_history_messages(*request))
}
