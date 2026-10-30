use super::super::super::{Request, WireRequest};

fn encode_send_to_agent(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::SendToAgent {
            message,
        }) => Ok(WireRequest::SendToAgent { message }),
        other => Err(Box::new(other)),
    }
}

fn encode_send(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Send {
            conversation,
            text,
        }) => Ok(WireRequest::Send {
            conversation,
            text,
            to: None,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_send_addressed(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::SendAddressed {
            conversation,
            text,
            to,
        }) => Ok(WireRequest::Send {
            conversation,
            text,
            to,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_accept(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Accept {
            conversation,
            plan,
        }) => Ok(WireRequest::Accept {
            conversation,
            plan,
            scope_hash: None,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_accept_scoped(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::AcceptScoped {
            conversation,
            plan,
            scope_hash,
        }) => Ok(WireRequest::Accept {
            conversation,
            plan,
            scope_hash,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_reject(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Reject {
            conversation,
            plan,
            reason,
        }) => Ok(WireRequest::Reject {
            conversation,
            plan,
            reason,
            scope_hash: None,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_reject_scoped(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::RejectScoped {
            conversation,
            plan,
            reason,
            scope_hash,
        }) => Ok(WireRequest::Reject {
            conversation,
            plan,
            reason,
            scope_hash,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_resume(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Resume {
            conversation,
            plan,
        }) => Ok(WireRequest::Resume {
            conversation,
            plan,
            scope_hash: None,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_resume_scoped(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::ResumeScoped {
            conversation,
            plan,
            scope_hash,
        }) => Ok(WireRequest::Resume {
            conversation,
            plan,
            scope_hash,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_review_comments(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::ReviewComments {
            conversation,
            plan,
        }) => Ok(WireRequest::ReviewComments { conversation, plan }),
        other => Err(Box::new(other)),
    }
}

fn encode_recover(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Recover(req)) => Ok({
            WireRequest::Recover {
                conversation: req.conversation,
                plan: req.plan,
                task: req.task,
                action: req.action,
                consent_files: req.consent_files,
                freshness_token: req.freshness_token,
                message: req.message,
            }
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_restart_orchestrator(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::RestartOrchestrator {
            conversation,
        }) => Ok(WireRequest::RestartOrchestrator { conversation }),
        other => Err(Box::new(other)),
    }
}

fn encode_cancel(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Cancel {
            conversation,
        }) => Ok(WireRequest::Cancel {
            conversation,
            scope_hash: None,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_cancel_scoped(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::CancelScoped {
            conversation,
            scope_hash,
        }) => Ok(WireRequest::Cancel {
            conversation,
            scope_hash,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_pause(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Pause {
            conversation,
            paused,
        }) => Ok(WireRequest::Pause {
            conversation,
            paused,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_mode(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Mode {
            conversation,
            autonomous,
        }) => Ok(WireRequest::Mode {
            conversation,
            autonomous,
        }),
        other => Err(Box::new(other)),
    }
}

pub(super) fn encode_group_messages(
    request: Request,
) -> std::result::Result<WireRequest, Box<Request>> {
    encode_send_to_agent(request)
        .or_else(|request| encode_send(*request))
        .or_else(|request| encode_send_addressed(*request))
        .or_else(|request| encode_accept(*request))
        .or_else(|request| encode_accept_scoped(*request))
        .or_else(|request| encode_reject(*request))
        .or_else(|request| encode_reject_scoped(*request))
        .or_else(|request| encode_resume(*request))
        .or_else(|request| encode_resume_scoped(*request))
        .or_else(|request| encode_review_comments(*request))
        .or_else(|request| encode_recover(*request))
        .or_else(|request| encode_restart_orchestrator(*request))
        .or_else(|request| encode_cancel(*request))
        .or_else(|request| encode_cancel_scoped(*request))
        .or_else(|request| encode_pause(*request))
        .or_else(|request| encode_mode(*request))
}
