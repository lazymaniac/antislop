use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum WireRequest {
    TerminalOpen {
        conversation: String,
        role: String,
        columns: u16,
        rows: u16,
        #[serde(default)]
        retry: bool,
    },
    TerminalRead {
        conversation: String,
        role: String,
        handle: String,
        after: u64,
    },
    TerminalInput {
        conversation: String,
        role: String,
        handle: String,
        sequence: u64,
        input: crate::lean::terminal::Input,
    },
    TerminalResize {
        conversation: String,
        role: String,
        handle: String,
        columns: u16,
        rows: u16,
    },
    TerminalDetach {
        conversation: String,
        role: String,
        handle: String,
    },
    RunContext {
        conversation: String,
        turn: String,
    },
    CommentAt {
        conversation: String,
        plan: Option<String>,
        target: String,
        text: String,
        round: Option<u32>,
        run: Option<String>,
    },

    SendToAgent {
        message: crate::lean::store::AgentMessage,
    },
    TaskRounds {
        conversation: String,
        plan: String,
        id: String,
    },
    TaskRound {
        conversation: String,
        plan: String,
        id: String,
        round: u32,
        #[serde(default)]
        references: Option<bool>,
    },
    SessionEvents {
        conversation: String,
        turn: String,
        after: u64,
        limit: Option<usize>,
        #[serde(default)]
        include_messages: Option<bool>,
    },
    HistoryContent {
        conversation: String,
        owner: crate::lean::store::ContentOwner,
        offset: u64,
    },
    HistoryMessages {
        conversation: String,
        scope: crate::lean::store::HistoryScope,
        after: u64,
        limit: Option<usize>,
    },
    SessionContent {
        conversation: String,
        event: u64,
        field: String,
        offset: u64,
    },

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

    Identity,
    GetConfig,
    SetConfig {
        toml: String,
        expected_revision: String,
    },
    Shutdown,
    ForceShutdown,
    Conversations,
    Start {
        #[serde(default)]
        project: Option<String>,
    },
    Send {
        conversation: String,
        text: String,
        #[serde(default)]
        to: Option<String>,
    },
    Accept {
        conversation: String,
        plan: String,
        #[serde(default)]
        scope_hash: Option<String>,
    },
    Reject {
        conversation: String,
        plan: String,
        #[serde(default)]
        reason: Option<String>,
        #[serde(default)]
        scope_hash: Option<String>,
    },
    Resume {
        conversation: String,
        plan: String,
        #[serde(default)]
        scope_hash: Option<String>,
    },
    ReviewComments {
        conversation: String,
        plan: String,
    },
    Recover {
        conversation: String,
        plan: String,
        task: String,
        action: crate::lean::RecoveryAction,
        #[serde(default)]
        consent_files: Option<Vec<String>>,
        freshness_token: String,
        #[serde(default)]
        message: Option<String>,
    },
    RestartOrchestrator {
        conversation: String,
    },
    Cancel {
        conversation: String,
        #[serde(default)]
        scope_hash: Option<String>,
    },
    Pause {
        conversation: String,
        paused: bool,
    },
    Mode {
        conversation: String,
        autonomous: bool,
    },
    Plans {
        conversation: String,
        #[serde(default)]
        offset: Option<usize>,
        #[serde(default)]
        limit: Option<usize>,
    },
    Plan {
        conversation: String,
        plan: String,
    },
    Comments {
        conversation: String,
        #[serde(default)]
        plan: Option<String>,
        #[serde(default)]
        status: Option<String>,
    },
    Comment {
        conversation: String,
        #[serde(default)]
        plan: Option<String>,
        target: String,
        text: String,
    },
    CommentStatus {
        conversation: String,
        id: u64,
        status: String,
    },
    Diff {
        conversation: String,
        plan: String,
        id: String,
    },
    Review {
        conversation: String,
        plan: String,
        id: String,
        #[serde(default)]
        scope_hash: Option<String>,
    },
    TaskChecks {
        #[serde(default)]
        conversation: Option<String>,
        plan: String,
        id: String,
    },
    Task {
        #[serde(default)]
        conversation: Option<String>,
        plan: String,
        id: String,
    },
    Events {
        conversation: String,
        after: u64,
        wait_ms: u64,
        #[serde(default)]
        limit: Option<usize>,
    },
    Sessions {
        conversation: String,
    },
    History {
        conversation: String,
        #[serde(default)]
        after: u64,
        #[serde(default)]
        limit: Option<usize>,
    },
    Mail {
        conversation: String,
    },
    TrustStatus,
    Trust {
        #[serde(default = "super::wire_defaults::default_true")]
        trusted: bool,
    },
}
