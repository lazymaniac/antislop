use super::super::super::{Request, WireRequest};

fn encode_run_context(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Session(crate::lean::daemon::SessionRequest::RunContext {
            conversation,
            turn,
        }) => Ok(WireRequest::RunContext { conversation, turn }),
        other => Err(Box::new(other)),
    }
}

fn encode_session_action(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Session(crate::lean::daemon::SessionRequest::SessionAction {
            conversation,
            action,
        }) => Ok(WireRequest::SessionAction {
            conversation,
            action,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_session_events(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Session(crate::lean::daemon::SessionRequest::SessionEvents {
            conversation,
            turn,
            after,
            limit,
            include_messages,
        }) => Ok(WireRequest::SessionEvents {
            conversation,
            turn,
            after,
            limit,
            include_messages,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_session_content(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Session(crate::lean::daemon::SessionRequest::SessionContent {
            conversation,
            event,
            field,
            offset,
        }) => Ok(WireRequest::SessionContent {
            conversation,
            event,
            field,
            offset,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_result_documents(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Session(crate::lean::daemon::SessionRequest::ResultDocuments { conversation }) => {
            Ok(WireRequest::ResultDocuments { conversation })
        }
        other => Err(Box::new(other)),
    }
}

fn encode_result_document_content(
    request: Request,
) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Session(crate::lean::daemon::SessionRequest::ResultDocumentContent {
            conversation,
            id,
            offset,
        }) => Ok(WireRequest::ResultDocumentContent {
            conversation,
            id,
            offset,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_sessions(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Session(crate::lean::daemon::SessionRequest::Sessions { conversation }) => {
            Ok(WireRequest::Sessions { conversation })
        }
        other => Err(Box::new(other)),
    }
}

fn encode_delete_session(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Session(crate::lean::daemon::SessionRequest::DeleteSession { conversation }) => {
            Ok(WireRequest::DeleteSession { conversation })
        }
        other => Err(Box::new(other)),
    }
}

pub(super) fn encode_group_all(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    encode_run_context(request)
        .or_else(|request| encode_session_action(*request))
        .or_else(|request| encode_session_events(*request))
        .or_else(|request| encode_session_content(*request))
        .or_else(|request| encode_result_documents(*request))
        .or_else(|request| encode_result_document_content(*request))
        .or_else(|request| encode_sessions(*request))
        .or_else(|request| encode_delete_session(*request))
}
