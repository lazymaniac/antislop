use super::super::super::{Request, WireRequest};

fn encode_task_rounds(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::History(crate::lean::daemon::HistoryRequest::TaskRounds {
            conversation,
            plan,
            id,
        }) => Ok(WireRequest::TaskRounds {
            conversation,
            plan,
            id,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_task_round(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::History(crate::lean::daemon::HistoryRequest::TaskRound {
            conversation,
            plan,
            id,
            round,
            references,
        }) => Ok(WireRequest::TaskRound {
            conversation,
            plan,
            id,
            round,
            references,
        }),
        other => Err(Box::new(other)),
    }
}

pub(super) fn encode_group_task(
    request: Request,
) -> std::result::Result<WireRequest, Box<Request>> {
    encode_task_rounds(request).or_else(|request| encode_task_round(*request))
}
