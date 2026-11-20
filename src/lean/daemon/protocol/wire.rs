use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum WireRequest {
    AddSource {
        conversation: String,
        text: String,
    },
    GetSource {
        conversation: String,
        source_id: String,
    },

    ActivateRuntime {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        request_id: Option<String>,
        executable: String,
        path: Option<String>,
        conversation: Option<String>,
        task: Option<String>,
    },
    RuntimeActivationStatus,
    RuntimeActivationOperation {
        request_id: String,
    },
    CancelRuntimeActivation {
        request_id: String,
    },
    /// Trusted-client operational information, never an execution grant.
    ClassifyRuntimeFailure {
        conversation: String,
        message: u64,
        reason: String,
    },
    RuntimeNotice {
        conversation: String,
        text: String,
    },
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
    SessionAction {
        conversation: String,
        action: crate::lean::daemon::SessionActionKind,
    },
    Comment {
        conversation: String,
        operation_id: Option<String>,
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
    ResultDocuments {
        conversation: String,
    },
    ResultDocumentContent {
        conversation: String,
        id: String,
        offset: u64,
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
        #[serde(default)]
        operation_id: Option<String>,
        text: String,
        #[serde(default)]
        to: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        images: Vec<String>,
    },
    Reject {
        conversation: String,
        #[serde(default)]
        operation_id: Option<String>,
        plan: String,
        #[serde(default)]
        reason: Option<String>,
        #[serde(default)]
        revision_token: Option<String>,
    },
    Resume {
        conversation: String,
        #[serde(default)]
        operation_id: Option<String>,
        plan: String,
        #[serde(default)]
        revision_token: Option<String>,
    },
    ReviewComments {
        conversation: String,
        #[serde(default)]
        operation_id: Option<String>,
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
    TakeControl {
        conversation: String,
    },
    RestartOrchestrator {
        conversation: String,
        #[serde(default)]
        operation_id: Option<String>,
    },
    Cancel {
        conversation: String,
        #[serde(default)]
        operation_id: Option<String>,
        #[serde(default)]
        revision_token: Option<String>,
    },
    Pause {
        conversation: String,
        paused: bool,
    },
    SetApprovalMode {
        conversation: String,
        mode: String,
    },
    AnswerApproval {
        conversation: String,
        #[serde(default)]
        operation_id: Option<String>,
        request: String,
        choice: String,
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
        revision_token: Option<String>,
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
    RevisePlan {
        conversation: String,
        #[serde(default)]
        operation_id: Option<String>,
        revision: crate::lean::store::PlanRevision,
        #[serde(default)]
        request_ai_review: bool,
    },
    SelectTeam {
        conversation: String,
        team: String,
        #[serde(default)]
        reason: String,
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
    DeleteSession {
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
}
