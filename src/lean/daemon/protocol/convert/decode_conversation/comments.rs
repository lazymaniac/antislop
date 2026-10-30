use super::super::super::{Request, WireRequest};

fn decode_comment_at(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::CommentAt {
            conversation,
            plan,
            target,
            text,
            round,
            run,
        } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::CommentAt {
                conversation,
                plan,
                target,
                text,
                round,
                run,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_comments(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Comments {
            conversation,
            plan,
            status,
        } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::Comments {
                conversation,
                plan,
                status,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_comment(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Comment {
            conversation,
            plan,
            target,
            text,
        } => Ok({
            if plan.is_some() {
                Request::Conversation(crate::lean::daemon::ConversationRequest::CommentScoped {
                    conversation,
                    plan,
                    target,
                    text,
                })
            } else {
                Request::Conversation(crate::lean::daemon::ConversationRequest::Comment {
                    conversation,
                    target,
                    text,
                })
            }
        }),
        other => Err(Box::new(other)),
    }
}

fn decode_comment_status(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::CommentStatus {
            conversation,
            id,
            status,
        } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::CommentStatus {
                conversation,
                id,
                status,
            },
        )),
        other => Err(Box::new(other)),
    }
}

pub(super) fn decode_group_comments(
    wire: WireRequest,
) -> std::result::Result<Request, Box<WireRequest>> {
    decode_comment_at(wire)
        .or_else(|wire| decode_comments(*wire))
        .or_else(|wire| decode_comment(*wire))
        .or_else(|wire| decode_comment_status(*wire))
}
