use super::super::super::{Request, WireRequest};

fn encode_terminal_open(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Terminal(crate::lean::daemon::TerminalRequest::TerminalOpen {
            conversation,
            role,
            columns,
            rows,
            retry,
        }) => Ok(WireRequest::TerminalOpen {
            conversation,
            role,
            columns,
            rows,
            retry,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_terminal_read(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Terminal(crate::lean::daemon::TerminalRequest::TerminalRead {
            conversation,
            role,
            handle,
            after,
        }) => Ok(WireRequest::TerminalRead {
            conversation,
            role,
            handle,
            after,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_terminal_input(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Terminal(crate::lean::daemon::TerminalRequest::TerminalInput {
            conversation,
            role,
            handle,
            sequence,
            input,
        }) => Ok(WireRequest::TerminalInput {
            conversation,
            role,
            handle,
            sequence,
            input,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_terminal_resize(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Terminal(crate::lean::daemon::TerminalRequest::TerminalResize {
            conversation,
            role,
            handle,
            columns,
            rows,
        }) => Ok(WireRequest::TerminalResize {
            conversation,
            role,
            handle,
            columns,
            rows,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_terminal_detach(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Terminal(crate::lean::daemon::TerminalRequest::TerminalDetach {
            conversation,
            role,
            handle,
        }) => Ok(WireRequest::TerminalDetach {
            conversation,
            role,
            handle,
        }),
        other => Err(Box::new(other)),
    }
}

pub(super) fn encode_group_all(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    encode_terminal_open(request)
        .or_else(|request| encode_terminal_read(*request))
        .or_else(|request| encode_terminal_input(*request))
        .or_else(|request| encode_terminal_resize(*request))
        .or_else(|request| encode_terminal_detach(*request))
}
