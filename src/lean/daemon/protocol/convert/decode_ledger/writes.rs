use super::super::super::{Request, WireRequest};

fn decode_reject_ledger_proposal(
    wire: WireRequest,
) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::RejectLedgerProposal {
            conversation,
            run_id,
            proposal_id,
            diff_hash,
            comment,
        } => Ok(Request::Ledger(
            crate::lean::daemon::LedgerRequest::RejectLedgerProposal {
                conversation,
                run_id,
                proposal_id,
                diff_hash,
                comment,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_propose_ledger_change(
    wire: WireRequest,
) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::ProposeLedgerChange { change } => Ok(Request::Ledger(
            crate::lean::daemon::LedgerRequest::ProposeLedgerChange { change },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_apply_ledger_change(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::ApplyLedgerChange {
            proposal_id,
            diff_hash,
            change,
        } => Ok(Request::Ledger(
            crate::lean::daemon::LedgerRequest::ApplyLedgerChange {
                proposal_id,
                diff_hash,
                change,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_create_run(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::CreateRun { conversation, goal } => Ok({
            Request::Ledger(crate::lean::daemon::LedgerRequest::CreateRun { conversation, goal })
        }),
        other => Err(Box::new(other)),
    }
}

fn decode_add_ledger_source(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::AddLedgerSource {
            conversation,
            run_id,
            text,
        } => Ok(Request::Ledger(
            crate::lean::daemon::LedgerRequest::AddLedgerSource {
                conversation,
                run_id,
                text,
            },
        )),
        other => Err(Box::new(other)),
    }
}

pub(super) fn decode_group_writes(
    wire: WireRequest,
) -> std::result::Result<Request, Box<WireRequest>> {
    decode_reject_ledger_proposal(wire)
        .or_else(|wire| decode_propose_ledger_change(*wire))
        .or_else(|wire| decode_apply_ledger_change(*wire))
        .or_else(|wire| decode_create_run(*wire))
        .or_else(|wire| decode_add_ledger_source(*wire))
}
