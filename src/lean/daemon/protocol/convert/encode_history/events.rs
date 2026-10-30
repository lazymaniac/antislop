use super::super::super::{Request, WireRequest};

fn encode_events(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::History(crate::lean::daemon::HistoryRequest::Events {
            conversation,
            after,
            wait_ms,
        }) => Ok(WireRequest::Events {
            conversation,
            after,
            wait_ms,
            limit: None,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_events_paged(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::History(crate::lean::daemon::HistoryRequest::EventsPaged {
            conversation,
            after,
            wait_ms,
            limit,
        }) => Ok(WireRequest::Events {
            conversation,
            after,
            wait_ms,
            limit,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_history(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::History(crate::lean::daemon::HistoryRequest::History {
            conversation,
            after,
            limit,
        }) => Ok(WireRequest::History {
            conversation,
            after,
            limit,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_mail(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::History(crate::lean::daemon::HistoryRequest::Mail { conversation }) => {
            Ok(WireRequest::Mail { conversation })
        }
        other => Err(Box::new(other)),
    }
}

pub(super) fn encode_group_events(
    request: Request,
) -> std::result::Result<WireRequest, Box<Request>> {
    encode_events(request)
        .or_else(|request| encode_events_paged(*request))
        .or_else(|request| encode_history(*request))
        .or_else(|request| encode_mail(*request))
}
