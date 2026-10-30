use super::super::super::{Request, WireRequest};

fn encode_comment_at(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::CommentAt {
            conversation,
            plan,
            target,
            text,
            round,
            run,
        }) => Ok(WireRequest::CommentAt {
            conversation,
            plan,
            target,
            text,
            round,
            run,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_comments(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Comments {
            conversation,
            plan,
            status,
        }) => Ok(WireRequest::Comments {
            conversation,
            plan,
            status,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_comment(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::Comment {
            conversation,
            target,
            text,
        }) => Ok(WireRequest::Comment {
            conversation,
            plan: None,
            target,
            text,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_comment_scoped(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::CommentScoped {
            conversation,
            plan,
            target,
            text,
        }) => Ok(WireRequest::Comment {
            conversation,
            plan,
            target,
            text,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_comment_status(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Conversation(crate::lean::daemon::ConversationRequest::CommentStatus {
            conversation,
            id,
            status,
        }) => Ok(WireRequest::CommentStatus {
            conversation,
            id,
            status,
        }),
        other => Err(Box::new(other)),
    }
}

pub(super) fn encode_group_comments(
    request: Request,
) -> std::result::Result<WireRequest, Box<Request>> {
    encode_comment_at(request)
        .or_else(|request| encode_comments(*request))
        .or_else(|request| encode_comment(*request))
        .or_else(|request| encode_comment_scoped(*request))
        .or_else(|request| encode_comment_status(*request))
}
