use super::super::super::{Request, WireRequest};

fn decode_list_runs(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::ListRuns { conversation } => Ok(Request::Ledger(
            crate::lean::daemon::LedgerRequest::ListRuns { conversation },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_get_ledger_proposal(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::GetLedgerProposal {
            conversation,
            run_id,
        } => Ok(Request::Ledger(
            crate::lean::daemon::LedgerRequest::GetLedgerProposal {
                conversation,
                run_id,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_list_requirements(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::ListRequirements { query } => Ok(Request::Ledger(
            crate::lean::daemon::LedgerRequest::ListRequirements { query },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_get_ledger_entry(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::GetLedgerEntry {
            conversation,
            run_id,
            id,
            revision,
        } => Ok(Request::Ledger(
            crate::lean::daemon::LedgerRequest::GetLedgerEntry {
                conversation,
                run_id,
                id,
                revision,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_get_ledger_diff(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::GetLedgerDiff {
            conversation,
            run_id,
            from_revision,
            to_revision,
        } => Ok(Request::Ledger(
            crate::lean::daemon::LedgerRequest::GetLedgerDiff {
                conversation,
                run_id,
                from_revision,
                to_revision,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_get_evidence(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::GetEvidence {
            conversation,
            run_id,
            entry_id,
        } => Ok(Request::Ledger(
            crate::lean::daemon::LedgerRequest::GetEvidence {
                conversation,
                run_id,
                entry_id,
            },
        )),
        other => Err(Box::new(other)),
    }
}

fn decode_get_ledger_source(wire: WireRequest) -> std::result::Result<Request, Box<WireRequest>> {
    match wire {
        WireRequest::GetLedgerSource {
            conversation,
            run_id,
            source_id,
        } => Ok(Request::Ledger(
            crate::lean::daemon::LedgerRequest::GetLedgerSource {
                conversation,
                run_id,
                source_id,
            },
        )),
        other => Err(Box::new(other)),
    }
}

pub(super) fn decode_group_reads(
    wire: WireRequest,
) -> std::result::Result<Request, Box<WireRequest>> {
    decode_list_runs(wire)
        .or_else(|wire| decode_get_ledger_proposal(*wire))
        .or_else(|wire| decode_list_requirements(*wire))
        .or_else(|wire| decode_get_ledger_entry(*wire))
        .or_else(|wire| decode_get_ledger_diff(*wire))
        .or_else(|wire| decode_get_evidence(*wire))
        .or_else(|wire| decode_get_ledger_source(*wire))
}
