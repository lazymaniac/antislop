use super::super::super::{Request, WireRequest};

fn decode_events(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Events {
            conversation,
            after,
            wait_ms,
            limit,
        } => Ok({
            if limit.is_some() {
                Request::History(crate::lean::daemon::HistoryRequest::EventsPaged {
                    conversation,
                    after,
                    wait_ms,
                    limit,
                })
            } else {
                Request::History(crate::lean::daemon::HistoryRequest::Events {
                    conversation,
                    after,
                    wait_ms,
                })
            }
        }),
        other => Err(Box::new(other)),
    }
}

fn decode_history(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::History {
            conversation,
            after,
            limit,
        } => Ok(Request::History(
            crate::lean::daemon::HistoryRequest::History {
                conversation,
                after,
                limit,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_mail(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Mail { conversation } => Ok(Request::History(
            crate::lean::daemon::HistoryRequest::Mail { conversation },
        )),
        other => Err(Box::new(other)),
    }
}

pub(super) fn decode_group_events(
    wire: WireRequest,
) -> std::result::Result<Request, Box<WireRequest>> {
    decode_events(wire)
        .or_else(|wire| decode_history(*wire))
        .or_else(|wire| decode_mail(*wire))
}
