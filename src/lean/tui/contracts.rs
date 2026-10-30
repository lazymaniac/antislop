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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Focus {
    #[default]
    Chat,
    Graph,
}

/// Full preserved state of a conversation across switches.
#[derive(Debug, Clone, PartialEq, Default)]
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
    pub relations: Relations,
    pub offset: (u16, u16),
    pub details: Option<Details>,
    pub diff: Option<DiffView>,
    pub plan_view: Option<PlanView>,
    pub ledger_view: Option<ledger::LedgerView>,
    pub scroll: usize,
    pub task_request_id: u64,
    pub diff_request_id: u64,
    pub focus: Focus,
    pub graph_scope: Scope,
    pub graph_collapsed: std::collections::HashSet<String>,
    pub graph_grouping: graph::GroupingMode,
    pub confirmation: Option<ConfirmationCard>,
    pub details_scroll_memory: std::collections::HashMap<(String, usize), usize>,
    pub details_query_memory: std::collections::HashMap<(String, usize), String>,
    pub graph_viewport: Option<(u16, u16)>,
}

impl ConversationState {
    pub(super) fn capture(app: &App) -> Self {
        let mut state = Self::default();
        state.capture_composer(app);
        state.capture_navigation(app);
        state.capture_overlays(app);
        state.capture_requests(app);
        state
    }

    fn capture_composer(&mut self, app: &App) {
        self.input = app.input.clone();
        self.editor = app.editor.clone();
        self.sending = app.sending;
        self.pending_send = app.pending_send.clone();
        self.send_request_id = app.send_request_id;
        self.failed_send = app.failed_send.clone();
        self.sending_comment = app.sending_comment;
        self.pending_comment = app.pending_comment.clone();
        self.comment_request_id = app.comment_request_id;
        self.failed_comment = app.failed_comment.clone();
        self.comment_drafts = app.comment_drafts.clone();
        self.comment_draft_cursors = app.comment_draft_cursors.clone();
    }

    fn capture_navigation(&mut self, app: &App) {
        self.selected = app.selected.clone();
        self.relations = app.relations;
        self.offset = app.offset;
        self.scroll = app.scroll;
        self.focus = app.focus;
        self.graph_scope = app.graph_scope.clone();
        self.graph_collapsed = app.graph_collapsed.clone();
        self.graph_grouping = app.graph_grouping;
        self.graph_viewport = app.graph_viewport;
    }

    fn capture_overlays(&mut self, app: &App) {
        self.details = app.details.clone();
        self.diff = app.diff.clone();
        self.plan_view = app.plan_view.clone();
        self.ledger_view = app.ledger_view.clone();
        self.confirmation = app.confirmation.clone();
    }

    fn capture_requests(&mut self, app: &App) {
        self.task_request_id = app.task_request_id;
        self.diff_request_id = app.diff_request_id;
        self.details_scroll_memory = app.details_scroll_memory.clone();
        self.details_query_memory = app.details_query_memory.clone();
    }

    pub(super) fn restore(&self, app: &mut App) {
        self.restore_composer(app);
        self.restore_navigation(app);
        self.restore_overlays(app);
        self.restore_requests(app);
    }

    fn restore_composer(&self, app: &mut App) {
        app.input = self.input.clone();
        app.editor = self.editor.clone();
        app.sending = self.sending;
        app.pending_send = self.pending_send.clone();
        app.send_request_id = self.send_request_id;
        app.failed_send = self.failed_send.clone();
        app.sending_comment = self.sending_comment;
        app.pending_comment = self.pending_comment.clone();
        app.comment_request_id = self.comment_request_id;
        app.failed_comment = self.failed_comment.clone();
        app.comment_drafts = self.comment_drafts.clone();
        app.comment_draft_cursors = self.comment_draft_cursors.clone();
    }

    fn restore_navigation(&self, app: &mut App) {
        app.selected = self.selected.clone();
        app.relations = self.relations;
        app.offset = self.offset;
        app.scroll = self.scroll;
        app.focus = self.focus;
        app.graph_scope = self.graph_scope.clone();
        app.graph_collapsed = self.graph_collapsed.clone();
        app.graph_grouping = self.graph_grouping;
        app.graph_viewport = self.graph_viewport;
    }

    fn restore_overlays(&self, app: &mut App) {
        app.details = self.details.clone();
        app.diff = self.diff.clone();
        app.plan_view = self.plan_view.clone();
        app.ledger_view = self.ledger_view.clone();
        app.confirmation = self.confirmation.clone();
    }

    fn restore_requests(&self, app: &mut App) {
        app.task_request_id = self.task_request_id;
        app.diff_request_id = self.diff_request_id;
        app.details_scroll_memory = self.details_scroll_memory.clone();
        app.details_query_memory = self.details_query_memory.clone();
    }
}
