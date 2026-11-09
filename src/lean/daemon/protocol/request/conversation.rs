#[derive(Debug, Clone, PartialEq)]
pub enum ConversationRequest {
    Recover(crate::lean::TaskRecoveryRequest),
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
    Send {
        conversation: String,
        operation_id: Option<String>,
        text: String,
        to: Option<String>,
    },
    /// Rejects a proposed plan and asks the orchestrator to revise it.
    Reject {
        conversation: String,
        operation_id: Option<String>,
        plan: String,
        reason: Option<String>,
        revision_token: Option<String>,
    },
    /// Runs again what the plan left unfinished, through the same sessions.
    Resume {
        conversation: String,
        operation_id: Option<String>,
        plan: String,
        revision_token: Option<String>,
    },
    /// This frontend steers the conversation from now on; others watch.
    TakeControl {
        conversation: String,
    },
    RestartOrchestrator {
        conversation: String,
        operation_id: Option<String>,
    },
    Cancel {
        conversation: String,
        operation_id: Option<String>,
        revision_token: Option<String>,
    },
    /// Stops starting new plan work, or continues it.
    Pause {
        conversation: String,
        paused: bool,
    },
    /// Sets who decides the approval dialogs of the conversation's agents:
    /// `supervised` (the user) or `autonomous` (the orchestrator).
    SetApprovalMode {
        conversation: String,
        mode: String,
    },
    /// The user's decision on an agent CLI's approval dialog.
    AnswerApproval {
        conversation: String,
        operation_id: Option<String>,
        request: String,
        choice: String,
    },
    /// The user finished commenting on the plan: the orchestrator answers
    /// the draft comments together.
    ReviewComments {
        conversation: String,
        operation_id: Option<String>,
        plan: String,
    },
    /// Paginated read of all stored plans for a conversation.
    Plans {
        conversation: String,
        offset: Option<usize>,
        limit: Option<usize>,
    },
    /// Detailed history of a single plan.
    Plan {
        conversation: String,
        plan: String,
    },
    /// Comments of a conversation, optionally filtered by plan and/or status.
    Comments {
        conversation: String,
        plan: Option<String>,
        status: Option<String>,
    },
    /// The user's comment on one element of the team panel or a diff.
    /// The user's comment scoped to a specific plan.
    /// Resolves, reopens (`open`) or deletes a comment.
    CommentStatus {
        conversation: String,
        id: u64,
        status: String,
    },
    /// A writer's change as files and numbered lines.
    Diff {
        conversation: String,
        plan: String,
        id: String,
    },
    /// Reviews a writer's change now; findings go to the orchestrator.
    Review {
        conversation: String,
        plan: String,
        id: String,
        revision_token: Option<String>,
    },
    /// One task of a plan with its latest report and verdict.
    /// Inspector snapshot and checks without a Git freshness fingerprint.
    TaskChecks {
        conversation: Option<String>,
        plan: String,
        id: String,
    },
    Task {
        conversation: Option<String>,
        plan: String,
        id: String,
    },
}

impl ConversationRequest {
    /// The conversation, idempotency id and kind of a state-changing request
    /// the client may safely retry.
    pub(in crate::lean::daemon) fn operation(&self) -> Option<(&str, &str, &'static str)> {
        let (conversation, operation_id, kind) = match self {
            Self::Send {
                conversation,
                operation_id,
                ..
            } => (conversation, operation_id, "send"),
            Self::Reject {
                conversation,
                operation_id,
                ..
            } => (conversation, operation_id, "reject"),
            Self::Resume {
                conversation,
                operation_id,
                ..
            } => (conversation, operation_id, "resume"),
            Self::ReviewComments {
                conversation,
                operation_id,
                ..
            } => (conversation, operation_id, "review_comments"),
            Self::RestartOrchestrator {
                conversation,
                operation_id,
            } => (conversation, operation_id, "restart_orchestrator"),
            Self::Cancel {
                conversation,
                operation_id,
                ..
            } => (conversation, operation_id, "cancel"),
            Self::AnswerApproval {
                conversation,
                operation_id,
                ..
            } => (conversation, operation_id, "answer_approval"),
            _ => return None,
        };
        Some((conversation, operation_id.as_deref()?, kind))
    }
}
