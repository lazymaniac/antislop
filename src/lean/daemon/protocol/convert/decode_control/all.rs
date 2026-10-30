use super::super::super::{Request, WireRequest};

fn decode_identity(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Identity => Ok(Request::Control(
            crate::lean::daemon::ControlRequest::Identity,
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_get_config(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::GetConfig => Ok(Request::Control(
            crate::lean::daemon::ControlRequest::GetConfig,
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_set_config(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::SetConfig {
            toml,
            expected_revision,
        } => Ok(Request::Control(
            crate::lean::daemon::ControlRequest::SetConfig {
                toml,
                expected_revision,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_shutdown(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Shutdown => Ok(Request::Control(
            crate::lean::daemon::ControlRequest::Shutdown,
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_force_shutdown(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::ForceShutdown => Ok(Request::Control(
            crate::lean::daemon::ControlRequest::ForceShutdown,
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_conversations(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Conversations => Ok(Request::Control(
            crate::lean::daemon::ControlRequest::Conversations,
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_start(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Start { project } => Ok(match project {
            Some(project) => {
                Request::Control(crate::lean::daemon::ControlRequest::StartIn { project })
            }
            None => Request::Control(crate::lean::daemon::ControlRequest::Start),
        }),
        other => Err(Box::new(other)),
    }
}

fn decode_trust_status(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::TrustStatus => Ok(Request::Control(
            crate::lean::daemon::ControlRequest::TrustStatus,
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_trust(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Trust { trusted } => Ok(Request::Control(
            crate::lean::daemon::ControlRequest::Trust { trusted },
        )),
        other => Err(Box::new(other)),
    }
}

pub(super) fn decode_group_all(
    wire: WireRequest,
) -> std::result::Result<Request, Box<WireRequest>> {
    decode_identity(wire)
        .or_else(|wire| decode_get_config(*wire))
        .or_else(|wire| decode_set_config(*wire))
        .or_else(|wire| decode_shutdown(*wire))
        .or_else(|wire| decode_force_shutdown(*wire))
        .or_else(|wire| decode_conversations(*wire))
        .or_else(|wire| decode_start(*wire))
        .or_else(|wire| decode_trust_status(*wire))
        .or_else(|wire| decode_trust(*wire))
}
