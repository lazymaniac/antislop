use super::super::super::{Request, WireRequest};

fn encode_identity(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Control(crate::lean::daemon::ControlRequest::Identity) => {
            Ok(WireRequest::Identity)
        }
        other => Err(Box::new(other)),
    }
}

fn encode_get_config(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Control(crate::lean::daemon::ControlRequest::GetConfig) => {
            Ok(WireRequest::GetConfig)
        }
        other => Err(Box::new(other)),
    }
}

fn encode_set_config(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Control(crate::lean::daemon::ControlRequest::SetConfig {
            toml,
            expected_revision,
        }) => Ok(WireRequest::SetConfig {
            toml,
            expected_revision,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_shutdown(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Control(crate::lean::daemon::ControlRequest::Shutdown) => {
            Ok(WireRequest::Shutdown)
        }
        other => Err(Box::new(other)),
    }
}

fn encode_force_shutdown(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Control(crate::lean::daemon::ControlRequest::ForceShutdown) => {
            Ok(WireRequest::ForceShutdown)
        }
        other => Err(Box::new(other)),
    }
}

fn encode_conversations(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Control(crate::lean::daemon::ControlRequest::Conversations) => {
            Ok(WireRequest::Conversations)
        }
        other => Err(Box::new(other)),
    }
}

fn encode_start(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Control(crate::lean::daemon::ControlRequest::Start) => {
            Ok(WireRequest::Start { project: None })
        }
        other => Err(Box::new(other)),
    }
}

fn encode_start_in(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Control(crate::lean::daemon::ControlRequest::StartIn { project }) => Ok({
            WireRequest::Start {
                project: Some(project),
            }
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_trust_status(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Control(crate::lean::daemon::ControlRequest::TrustStatus) => {
            Ok(WireRequest::TrustStatus)
        }
        other => Err(Box::new(other)),
    }
}

fn encode_trust(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Control(crate::lean::daemon::ControlRequest::Trust { trusted }) => {
            Ok(WireRequest::Trust { trusted })
        }
        other => Err(Box::new(other)),
    }
}

pub(super) fn encode_group_all(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    encode_identity(request)
        .or_else(|request| encode_get_config(*request))
        .or_else(|request| encode_set_config(*request))
        .or_else(|request| encode_shutdown(*request))
        .or_else(|request| encode_force_shutdown(*request))
        .or_else(|request| encode_conversations(*request))
        .or_else(|request| encode_start(*request))
        .or_else(|request| encode_start_in(*request))
        .or_else(|request| encode_trust_status(*request))
        .or_else(|request| encode_trust(*request))
}
