use super::super::super::{Request, WireRequest};

fn encode_activation(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    use crate::lean::daemon::ControlRequest;
    match request {
        Request::Control(ControlRequest::ActivateRuntime {
            request_id,
            executable,
            path,
            conversation,
            task,
        }) => Ok(WireRequest::ActivateRuntime {
            request_id,
            executable,
            path,
            conversation,
            task,
        }),
        Request::Control(ControlRequest::RuntimeActivationStatus) => {
            Ok(WireRequest::RuntimeActivationStatus)
        }
        Request::Control(ControlRequest::RuntimeActivationOperation { request_id }) => {
            Ok(WireRequest::RuntimeActivationOperation { request_id })
        }
        Request::Control(ControlRequest::CancelRuntimeActivation { request_id }) => {
            Ok(WireRequest::CancelRuntimeActivation { request_id })
        }
        Request::Control(ControlRequest::ClassifyRuntimeFailure {
            conversation,
            message,
            reason,
        }) => Ok(WireRequest::ClassifyRuntimeFailure {
            conversation,
            message,
            reason,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_runtime_notice(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Control(crate::lean::daemon::ControlRequest::RuntimeNotice {
            conversation,
            text,
        }) => Ok(WireRequest::RuntimeNotice { conversation, text }),
        other => Err(Box::new(other)),
    }
}

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

pub(super) fn encode_group_all(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    encode_activation(request)
        .or_else(|request| encode_runtime_notice(*request))
        .or_else(|request| encode_identity(*request))
        .or_else(|request| encode_get_config(*request))
        .or_else(|request| encode_set_config(*request))
        .or_else(|request| encode_shutdown(*request))
        .or_else(|request| encode_force_shutdown(*request))
        .or_else(|request| encode_conversations(*request))
        .or_else(|request| encode_start(*request))
        .or_else(|request| encode_start_in(*request))
}
