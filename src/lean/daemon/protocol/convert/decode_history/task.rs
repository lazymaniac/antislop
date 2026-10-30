use super::super::super::{Request, WireRequest};

fn decode_task_rounds(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::TaskRounds {
            conversation,
            plan,
            id,
        } => Ok(Request::History(
            crate::lean::daemon::HistoryRequest::TaskRounds {
                conversation,
                plan,
                id,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_task_round(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::TaskRound {
            conversation,
            plan,
            id,
            round,
            references,
        } => Ok(Request::History(
            crate::lean::daemon::HistoryRequest::TaskRound {
                conversation,
                plan,
                id,
                round,
                references,
            },
        )),
        other => Err(Box::new(other)),
    }
}

pub(super) fn decode_group_task(
    wire: WireRequest,
) -> std::result::Result<Request, Box<WireRequest>> {
    decode_task_rounds(wire).or_else(|wire| decode_task_round(*wire))
}
