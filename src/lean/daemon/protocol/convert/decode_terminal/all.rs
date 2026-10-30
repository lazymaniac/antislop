use super::super::super::{Request, WireRequest};

fn decode_terminal_open(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::TerminalOpen {
            conversation,
            role,
            columns,
            rows,
            retry,
        } => Ok(Request::Terminal(
            crate::lean::daemon::TerminalRequest::TerminalOpen {
                conversation,
                role,
                columns,
                rows,
                retry,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_terminal_read(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::TerminalRead {
            conversation,
            role,
            handle,
            after,
        } => Ok(Request::Terminal(
            crate::lean::daemon::TerminalRequest::TerminalRead {
                conversation,
                role,
                handle,
                after,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_terminal_input(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::TerminalInput {
            conversation,
            role,
            handle,
            sequence,
            input,
        } => Ok(Request::Terminal(
            crate::lean::daemon::TerminalRequest::TerminalInput {
                conversation,
                role,
                handle,
                sequence,
                input,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_terminal_resize(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::TerminalResize {
            conversation,
            role,
            handle,
            columns,
            rows,
        } => Ok(Request::Terminal(
            crate::lean::daemon::TerminalRequest::TerminalResize {
                conversation,
                role,
                handle,
                columns,
                rows,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_terminal_detach(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::TerminalDetach {
            conversation,
            role,
            handle,
        } => Ok(Request::Terminal(
            crate::lean::daemon::TerminalRequest::TerminalDetach {
                conversation,
                role,
                handle,
            },
        )),
        other => Err(Box::new(other)),
    }
}

pub(super) fn decode_group_all(
    wire: WireRequest,
) -> std::result::Result<Request, Box<WireRequest>> {
    decode_terminal_open(wire)
        .or_else(|wire| decode_terminal_read(*wire))
        .or_else(|wire| decode_terminal_input(*wire))
        .or_else(|wire| decode_terminal_resize(*wire))
        .or_else(|wire| decode_terminal_detach(*wire))
}
