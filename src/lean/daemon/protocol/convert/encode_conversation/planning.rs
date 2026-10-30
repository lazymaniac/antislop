use super::super::super::{Request, WireRequest};

fn encode_plans(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Plans {
            conversation,
            offset,
            limit,
        }) => Ok(WireRequest::Plans {
            conversation,
            offset,
            limit,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_plan(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Plan {
            conversation,
            plan,
        }) => Ok(WireRequest::Plan { conversation, plan }),
        other => Err(Box::new(other)),
    }
}

fn encode_diff(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Diff {
            conversation,
            plan,
            id,
        }) => Ok(WireRequest::Diff {
            conversation,
            plan,
            id,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_review(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Review {
            conversation,
            plan,
            id,
        }) => Ok(WireRequest::Review {
            conversation,
            plan,
            id,
            scope_hash: None,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_review_scoped(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::ReviewScoped {
            conversation,
            plan,
            id,
            scope_hash,
        }) => Ok(WireRequest::Review {
            conversation,
            plan,
            id,
            scope_hash,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_task_checks(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::TaskChecks {
            conversation,
            plan,
            id,
        }) => Ok(WireRequest::TaskChecks {
            conversation,
            plan,
            id,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_task(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Task { plan, id }) => Ok({
            WireRequest::Task {
                conversation: None,
                plan,
                id,
            }
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_task_scoped(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::TaskScoped {
            conversation,
            plan,
            id,
        }) => Ok(WireRequest::Task {
            conversation,
            plan,
            id,
        }),
        other => Err(Box::new(other)),
    }
}

pub(super) fn encode_group_planning(
    request: Request,
) -> std::result::Result<WireRequest, Box<Request>> {
    encode_plans(request)
        .or_else(|request| encode_plan(*request))
        .or_else(|request| encode_diff(*request))
        .or_else(|request| encode_review(*request))
        .or_else(|request| encode_review_scoped(*request))
        .or_else(|request| encode_task_checks(*request))
        .or_else(|request| encode_task(*request))
        .or_else(|request| encode_task_scoped(*request))
}
