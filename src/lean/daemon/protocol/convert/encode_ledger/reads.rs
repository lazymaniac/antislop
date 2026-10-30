use super::super::super::{Request, WireRequest};

fn encode_list_runs(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Ledger(crate::lean::daemon::LedgerRequest::ListRuns { conversation }) => {
            Ok(WireRequest::ListRuns { conversation })
        }
        other => Err(Box::new(other)),
    }
}

fn encode_get_ledger_proposal(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Ledger(crate::lean::daemon::LedgerRequest::GetLedgerProposal {
            conversation,
            run_id,
        }) => Ok(WireRequest::GetLedgerProposal {
            conversation,
            run_id,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_list_requirements(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Ledger(crate::lean::daemon::LedgerRequest::ListRequirements { query }) => {
            Ok(WireRequest::ListRequirements { query })
        }
        other => Err(Box::new(other)),
    }
}

fn encode_get_ledger_entry(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Ledger(crate::lean::daemon::LedgerRequest::GetLedgerEntry {
            conversation,
            run_id,
            id,
            revision,
        }) => Ok(WireRequest::GetLedgerEntry {
            conversation,
            run_id,
            id,
            revision,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_get_ledger_diff(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Ledger(crate::lean::daemon::LedgerRequest::GetLedgerDiff {
            conversation,
            run_id,
            from_revision,
            to_revision,
        }) => Ok(WireRequest::GetLedgerDiff {
            conversation,
            run_id,
            from_revision,
            to_revision,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_get_evidence(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Ledger(crate::lean::daemon::LedgerRequest::GetEvidence {
            conversation,
            run_id,
            entry_id,
        }) => Ok(WireRequest::GetEvidence {
            conversation,
            run_id,
            entry_id,
        }),
        other => Err(Box::new(other)),
    }
}

fn encode_get_ledger_source(request: Request) -> std::result::Result<WireRequest, Box<Request>> {
    match request {
        Request::Ledger(crate::lean::daemon::LedgerRequest::GetLedgerSource {
            conversation,
            run_id,
            source_id,
        }) => Ok(WireRequest::GetLedgerSource {
            conversation,
            run_id,
            source_id,
        }),
        other => Err(Box::new(other)),
    }
}

pub(super) fn encode_group_reads(
    request: Request,
) -> std::result::Result<WireRequest, Box<Request>> {
    encode_list_runs(request)
        .or_else(|request| encode_get_ledger_proposal(*request))
        .or_else(|request| encode_list_requirements(*request))
        .or_else(|request| encode_get_ledger_entry(*request))
        .or_else(|request| encode_get_ledger_diff(*request))
        .or_else(|request| encode_get_evidence(*request))
        .or_else(|request| encode_get_ledger_source(*request))
}
