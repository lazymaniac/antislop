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
            to,
        }) => Ok(WireRequest::Send {
            conversation,
            text,
            to,
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
            revision_token,
        }) => Ok(WireRequest::Reject {
            conversation,
            plan,
            reason,
            revision_token,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_resume(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Resume {
            conversation,
            plan,
            revision_token,
        }) => Ok(WireRequest::Resume {
            conversation,
            plan,
            revision_token,
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
            revision_token,
        }) => Ok(WireRequest::Cancel {
            conversation,
            revision_token,
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

fn encode_approvals(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    use crate::lean::daemon::ConversationRequest as C;
    match request {
        Request::Conversation(C::SetApprovalMode { conversation, mode }) => {
            Ok(WireRequest::SetApprovalMode { conversation, mode })
        }
        Request::Conversation(C::AnswerApproval {
            conversation,
            request,
            choice,
        }) => Ok(WireRequest::AnswerApproval {
            conversation,
            request,
            choice,
        }),
        other => Err(Box::new(other)),
    }
}

pub(super) fn encode_group_messages(
    request: Request,
) -> std::result::Result<WireRequest, Box<Request>> {
    encode_send_to_agent(request)
        .or_else(|request| encode_send(*request))
        .or_else(|request| encode_reject(*request))
        .or_else(|request| encode_resume(*request))
        .or_else(|request| encode_review_comments(*request))
        .or_else(|request| encode_recover(*request))
        .or_else(|request| encode_restart_orchestrator(*request))
        .or_else(|request| encode_cancel(*request))
        .or_else(|request| encode_pause(*request))
        .or_else(|request| encode_approvals(*request))
}
