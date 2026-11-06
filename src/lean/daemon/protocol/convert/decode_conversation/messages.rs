use super::super::super::{Request, WireRequest};

fn decode_send_to_agent(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::SendToAgent { message } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::SendToAgent { message },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_send(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Send {
            conversation,
            text,
            to,
        } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::Send {
                conversation,
                text,
                to,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_reject(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Reject {
            conversation,
            plan,
            reason,
            revision_token,
        } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::Reject {
                conversation,
                plan,
                reason,
                revision_token,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_resume(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Resume {
            conversation,
            plan,
            revision_token,
        } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::Resume {
                conversation,
                plan,
                revision_token,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_review_comments(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::ReviewComments { conversation, plan } => Ok({
            Request::Conversation(crate::lean::daemon::ConversationRequest::ReviewComments {
                conversation,
                plan,
            })
        }),
        other => Err(Box::new(other)),
    }
}

fn decode_recover(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Recover {
            conversation,
            plan,
            task,
            action,
            consent_files,
            freshness_token,
            message,
        } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::Recover(crate::lean::TaskRecoveryRequest {
                conversation,
                plan,
                task,
                action,
                consent_files,
                freshness_token,
                message,
            }),
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_restart_orchestrator(
    wire: WireRequest,
) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::RestartOrchestrator { conversation } => Ok({
            Request::Conversation(
                crate::lean::daemon::ConversationRequest::RestartOrchestrator { conversation },
            )
        }),
        other => Err(Box::new(other)),
    }
}

fn decode_cancel(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Cancel {
            conversation,
            revision_token,
        } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::Cancel {
                conversation,
                revision_token,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_pause(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Pause {
            conversation,
            paused,
        } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::Pause {
                conversation,
                paused,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_approvals(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    use crate::lean::daemon::ConversationRequest as C;
    match wire {
        WireRequest::SetApprovalMode { conversation, mode } => {
            Ok(Request::Conversation(C::SetApprovalMode {
                conversation,
                mode,
            }))
        }
        WireRequest::AnswerApproval {
            conversation,
            request,
            choice,
        } => Ok(Request::Conversation(C::AnswerApproval {
            conversation,
            request,
            choice,
        })),
        other => Err(Box::new(other)),
    }
}

pub(super) fn decode_group_messages(
    wire: WireRequest,
) -> std::result::Result<Request, Box<WireRequest>> {
    decode_send_to_agent(wire)
        .or_else(|wire| decode_send(*wire))
        .or_else(|wire| decode_reject(*wire))
        .or_else(|wire| decode_resume(*wire))
        .or_else(|wire| decode_review_comments(*wire))
        .or_else(|wire| decode_recover(*wire))
        .or_else(|wire| decode_restart_orchestrator(*wire))
        .or_else(|wire| decode_cancel(*wire))
        .or_else(|wire| decode_pause(*wire))
        .or_else(|wire| decode_approvals(*wire))
}
