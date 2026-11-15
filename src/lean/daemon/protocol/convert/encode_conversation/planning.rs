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
            revision_token,
        }) => Ok(WireRequest::Review {
            conversation,
            plan,
            id,
            revision_token,
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
        Request::Conversation(crate::lean::daemon::ConversationRequest::Task {
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

fn encode_revise_plan(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::RevisePlan {
            conversation,
            operation_id,
            revision,
            request_ai_review,
            allow_uncovered,
        }) => Ok(WireRequest::RevisePlan {
            conversation,
            operation_id,
            revision,
            request_ai_review,
            allow_uncovered,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_select_team(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::SelectTeam {
            conversation,
            team,
            reason,
        }) => Ok(WireRequest::SelectTeam {
            conversation,
            team,
            reason,
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
        .or_else(|request| encode_task_checks(*request))
        .or_else(|request| encode_task(*request))
        .or_else(|request| encode_revise_plan(*request))
        .or_else(|request| encode_select_team(*request))
}
