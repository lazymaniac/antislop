use super::super::super::{Request, WireRequest};

fn encode_reject_ledger_proposal(
    request: Request,
) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Ledger(crate::lean::daemon::LedgerRequest::RejectLedgerProposal {
            conversation,
            run_id,
            proposal_id,
            diff_hash,
            comment,
        }) => Ok(WireRequest::RejectLedgerProposal {
            conversation,
            run_id,
            proposal_id,
            diff_hash,
            comment,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_propose_ledger_change(
    request: Request,
) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Ledger(crate::lean::daemon::LedgerRequest::ProposeLedgerChange { change }) => {
            Ok(WireRequest::ProposeLedgerChange { change })
        }
        other => Err(Box::new(other)),
    }
}

fn encode_apply_ledger_change(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Ledger(crate::lean::daemon::LedgerRequest::ApplyLedgerChange {
            proposal_id,
            diff_hash,
            change,
        }) => Ok(WireRequest::ApplyLedgerChange {
            proposal_id,
            diff_hash,
            change,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_create_run(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Ledger(crate::lean::daemon::LedgerRequest::CreateRun { conversation, goal }) => {
            Ok(WireRequest::CreateRun { conversation, goal })
        }
        other => Err(Box::new(other)),
    }
}

fn encode_add_ledger_source(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Ledger(crate::lean::daemon::LedgerRequest::AddLedgerSource {
            conversation,
            run_id,
            text,
        }) => Ok(WireRequest::AddLedgerSource {
            conversation,
            run_id,
            text,
        }),
        other => Err(Box::new(other)),
    }
}

pub(super) fn encode_group_writes(
    request: Request,
) -> std::result::Result<WireRequest, Box<Request>> {
    encode_reject_ledger_proposal(request)
        .or_else(|request| encode_propose_ledger_change(*request))
        .or_else(|request| encode_apply_ledger_change(*request))
        .or_else(|request| encode_create_run(*request))
        .or_else(|request| encode_add_ledger_source(*request))
}
