#[derive(Debug, Clone, PartialEq)]
pub enum LedgerRequest {
    ListRuns {
        conversation: String,
    },
    RejectLedgerProposal {
        conversation: String,
        run_id: String,
        proposal_id: String,
        diff_hash: String,
        comment: String,
    },
    GetLedgerProposal {
        conversation: String,
        run_id: String,
    },
    ListRequirements {
        query: crate::lean::ledger::LedgerQuery,
    },
    GetLedgerEntry {
        conversation: String,
        run_id: String,
        id: String,
        revision: Option<i64>,
    },
    GetLedgerDiff {
        conversation: String,
        run_id: String,
        from_revision: i64,
        to_revision: i64,
    },
    GetEvidence {
        conversation: String,
        run_id: String,
        entry_id: Option<String>,
    },
    ProposeLedgerChange {
        change: crate::lean::ledger::LedgerChange,
    },
    ApplyLedgerChange {
        proposal_id: String,
        diff_hash: String,
        change: crate::lean::ledger::LedgerChange,
    },
    CreateRun {
        conversation: String,
        goal: String,
    },
    AddLedgerSource {
        conversation: String,
        run_id: String,
        text: String,
    },
    GetLedgerSource {
        conversation: String,
        run_id: String,
        source_id: String,
    },
}
