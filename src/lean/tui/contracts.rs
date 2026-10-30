use super::*;

pub enum Update {
    NativeInputRejected {
        conversation: String,
        role: String,
        input: crate::lean::terminal::Input,
        error: String,
    },
    PlanHistory {
        conversation: String,
        generation: u64,
        offset: usize,
        reply: std::result::Result<crate::lean::store::PlansPage, String>,
    },
    InterviewResult {
        conversation: String,
        question: u64,
        result: std::result::Result<(), String>,
    },
    AgentMessage {
        conversation: String,
        recipient: String,
        operation: String,
        text: String,
        result: std::result::Result<Value, String>,
    },
    Ledger {
        conversation: String,
        request_id: u64,
        kind: String,
        reply: std::result::Result<Value, String>,
    },
    Config {
        project: String,
        save: bool,
        reply: std::result::Result<Value, String>,
    },
    Events {
        conversation: String,
        generation: u64,
        events: Vec<Value>,
        idle: bool,
        autonomous: bool,
    },
    Conversation(String),
    Notice(String),
    /// A project was opened: switch to it, to `conversation` if one was named.
    ProjectReady {
        conn: Box<workspace::ProjectConn>,
        conversation: Option<String>,
    },
    /// The directory must be trusted before it is opened.
    ProjectTrust(std::path::PathBuf),
    /// The conversations of a connected project, as it last listed them.
    ProjectStatus {
        path: std::path::PathBuf,
        conversations: Vec<Value>,
    },
    /// A task's change, for the diff view of `task`.
    Diff {
        conversation: String,
        plan: Option<String>,
        generation: u64,
        request_id: u64,
        task: String,
        reply: std::result::Result<Value, String>,
    },
    /// A task as the daemon stores it, for the details of `key`. `round`
    /// is the task's round when the fetch was issued; a reply for a round
    /// that already moved on is dropped instead of shown stale.
    Task {
        conversation: String,
        plan: Option<String>,
        generation: u64,
        request_id: u64,
        key: String,
        round: u64,
        task: Value,
        documents: std::collections::HashMap<String, document::Document>,
    },
    TaskFailed {
        conversation: String,
        generation: u64,
        plan: String,
        request_id: u64,
        key: String,
        error: String,
    },
    SendResult {
        conversation: String,
        generation: u64,
        request_id: u64,
        sent_text: String,
        result: std::result::Result<(), String>,
    },
    CommentResult {
        conversation: String,
        plan: Option<String>,
        generation: u64,
        request_id: u64,
        editor: Editor,
        result: std::result::Result<(), String>,
    },
}

/// A comment being written on one element.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editor {
    pub target: String,
    pub input: InputBuffer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Chat,
    Graph,
}

/// Full preserved state of a conversation across switches.
#[derive(Debug, Clone, PartialEq)]
pub struct ConversationState {
    pub input: InputBuffer,
    pub editor: Option<Editor>,
    pub sending: bool,
    pub pending_send: Option<String>,
    pub send_request_id: u64,
    pub failed_send: Option<String>,
    pub sending_comment: bool,
    pub pending_comment: Option<Editor>,
    pub comment_request_id: u64,
    pub failed_comment: Option<Editor>,
    pub comment_drafts: std::collections::HashMap<String, String>,
    pub comment_draft_cursors: std::collections::HashMap<String, (usize, Option<u16>)>,
    pub selected: String,
    pub filter: Filter,
    pub offset: (u16, u16),
    pub details: Option<Details>,
    pub diff: Option<DiffView>,
    pub plan_view: Option<PlanView>,
    pub ledger_view: Option<ledger::LedgerView>,
    pub scroll: usize,
    pub task_request_id: u64,
    pub diff_request_id: u64,
    pub focus: Focus,
    pub graph_filter: Option<String>,
    pub graph_collapsed: std::collections::HashSet<String>,
    pub graph_grouping: graph::GroupingMode,
    pub confirmation: Option<ConfirmationCard>,
    pub details_scroll_memory: std::collections::HashMap<(String, usize), usize>,
    pub details_query_memory: std::collections::HashMap<(String, usize), String>,
    pub graph_viewport: Option<(u16, u16)>,
}
