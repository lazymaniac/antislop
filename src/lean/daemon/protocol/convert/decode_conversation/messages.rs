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
            operation_id,
            text,
            to,
        } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::Send {
                conversation,
                operation_id,
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
            operation_id,
            plan,
            reason,
            revision_token,
        } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::Reject {
                conversation,
                operation_id,
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
            operation_id,
            plan,
            revision_token,
        } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::Resume {
                conversation,
                operation_id,
                plan,
                revision_token,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_review_comments(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::ReviewComments {
            conversation,
            operation_id,
            plan,
        } => Ok({
            Request::Conversation(crate::lean::daemon::ConversationRequest::ReviewComments {
                conversation,
                operation_id,
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
        WireRequest::RestartOrchestrator {
            conversation,
            operation_id,
        } => Ok({
            Request::Conversation(
                crate::lean::daemon::ConversationRequest::RestartOrchestrator {
                    conversation,
                    operation_id,
                },
            )
        }),
        other => Err(Box::new(other)),
    }
}

fn decode_take_control(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::TakeControl { conversation } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::TakeControl { conversation },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_cancel(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Cancel {
            conversation,
            operation_id,
            revision_token,
        } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::Cancel {
                conversation,
                operation_id,
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
            operation_id,
            request,
            choice,
        } => Ok(Request::Conversation(C::AnswerApproval {
            conversation,
            operation_id,
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
        .or_else(|wire| decode_take_control(*wire))
        .or_else(|wire| decode_cancel(*wire))
        .or_else(|wire| decode_pause(*wire))
        .or_else(|wire| decode_approvals(*wire))
}
