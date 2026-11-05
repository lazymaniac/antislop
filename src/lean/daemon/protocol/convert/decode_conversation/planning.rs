use super::super::super::{Request, WireRequest};

fn decode_plans(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Plans {
            conversation,
            offset,
            limit,
        } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::Plans {
                conversation,
                offset,
                limit,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_plan(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Plan { conversation, plan } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::Plan { conversation, plan },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_diff(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Diff {
            conversation,
            plan,
            id,
        } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::Diff {
                conversation,
                plan,
                id,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_review(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Review {
            conversation,
            plan,
            id,
            revision_token,
        } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::Review {
                conversation,
                plan,
                id,
                revision_token,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_task_checks(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::TaskChecks {
            conversation,
            plan,
            id,
        } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::TaskChecks {
                conversation,
                plan,
                id,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_task(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::Task {
            conversation,
            plan,
            id,
        } => Ok(Request::Conversation(
            crate::lean::daemon::ConversationRequest::Task {
                conversation,
                plan,
                id,
            },
        )),
        other => Err(Box::new(other)),
    }
}

pub(super) fn decode_group_planning(
    wire: WireRequest,
) -> std::result::Result<Request, Box<WireRequest>> {
    decode_plans(wire)
        .or_else(|wire| decode_plan(*wire))
        .or_else(|wire| decode_diff(*wire))
        .or_else(|wire| decode_review(*wire))
        .or_else(|wire| decode_task_checks(*wire))
        .or_else(|wire| decode_task(*wire))
}
