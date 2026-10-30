use super::super::super::{Request, WireRequest};

fn decode_run_context(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::RunContext { conversation, turn } => Ok({
            Request::Session(crate::lean::daemon::SessionRequest::RunContext { conversation, turn })
        }),
        other => Err(Box::new(other)),
    }
}

fn decode_session_events(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::SessionEvents {
            conversation,
            turn,
            after,
            limit,
            include_messages,
        } => Ok(Request::Session(
            crate::lean::daemon::SessionRequest::SessionEvents {
                conversation,
                turn,
                after,
                limit,
                include_messages,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_session_content(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::SessionContent {
            conversation,
            event,
            field,
            offset,
        } => Ok(Request::Session(
            crate::lean::daemon::SessionRequest::SessionContent {
                conversation,
                event,
                field,
                offset,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_sessions(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Sessions { conversation } => Ok(Request::Session(
            crate::lean::daemon::SessionRequest::Sessions { conversation },
        )),
        other => Err(Box::new(other)),
    }
}

pub(super) fn decode_group_all(
    wire: WireRequest,
) -> std::result::Result<Request, Box<WireRequest>> {
    decode_run_context(wire)
        .or_else(|wire| decode_session_events(*wire))
        .or_else(|wire| decode_session_content(*wire))
        .or_else(|wire| decode_sessions(*wire))
}
