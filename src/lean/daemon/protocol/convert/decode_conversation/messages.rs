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
        } => Ok({
            if to.is_some() {
                Request::Conversation(crate::lean::daemon::ConversationRequest::SendAddressed {
                    conversation,
                    text,
                    to,
                })
            } else {
                Request::Conversation(crate::lean::daemon::ConversationRequest::Send {
                    conversation,
                    text,
                })
            }
        }),
        other => Err(Box::new(other)),
    }
}

fn decode_accept(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Accept {
            conversation,
            plan,
            scope_hash,
        } => Ok({
            if scope_hash.is_some() {
                Request::Conversation(crate::lean::daemon::ConversationRequest::AcceptScoped {
                    conversation,
                    plan,
                    scope_hash,
                })
            } else {
                Request::Conversation(crate::lean::daemon::ConversationRequest::Accept {
                    conversation,
                    plan,
                })
            }
        }),
        other => Err(Box::new(other)),
    }
}

fn decode_reject(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Reject {
            conversation,
            plan,
            reason,
            scope_hash,
        } => Ok({
            if scope_hash.is_some() {
                Request::Conversation(crate::lean::daemon::ConversationRequest::RejectScoped {
                    conversation,
                    plan,
                    reason,
                    scope_hash,
                })
            } else {
                Request::Conversation(crate::lean::daemon::ConversationRequest::Reject {
                    conversation,
                    plan,
                    reason,
                })
            }
        }),
        other => Err(Box::new(other)),
    }
}

fn decode_resume(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Resume {
            conversation,
            plan,
            scope_hash,
        } => Ok({
            if scope_hash.is_some() {
                Request::Conversation(crate::lean::daemon::ConversationRequest::ResumeScoped {
                    conversation,
                    plan,
                    scope_hash,
                })
            } else {
                Request::Conversation(crate::lean::daemon::ConversationRequest::Resume {
                    conversation,
                    plan,
                })
            }
        }),
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
            scope_hash,
        } => Ok({
            if scope_hash.is_some() {
                Request::Conversation(crate::lean::daemon::ConversationRequest::CancelScoped {
                    conversation,
                    scope_hash,
                })
            } else {
                Request::Conversation(crate::lean::daemon::ConversationRequest::Cancel {
                    conversation,
                })
            }
        }),
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

fn decode_mode(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Mode {
            conversation,
            autonomous,
        } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::Mode {
                conversation,
                autonomous,
            },
        )),
        other => Err(Box::new(other)),
    }
}

pub(super) fn decode_group_messages(
    wire: WireRequest,
) -> std::result::Result<Request, Box<WireRequest>> {
    decode_send_to_agent(wire)
        .or_else(|wire| decode_send(*wire))
        .or_else(|wire| decode_accept(*wire))
        .or_else(|wire| decode_reject(*wire))
        .or_else(|wire| decode_resume(*wire))
        .or_else(|wire| decode_review_comments(*wire))
        .or_else(|wire| decode_recover(*wire))
        .or_else(|wire| decode_restart_orchestrator(*wire))
        .or_else(|wire| decode_cancel(*wire))
        .or_else(|wire| decode_pause(*wire))
        .or_else(|wire| decode_mode(*wire))
}
