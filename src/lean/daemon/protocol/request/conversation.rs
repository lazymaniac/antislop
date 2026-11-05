#[derive(Debug, Clone, PartialEq)]
pub enum ConversationRequest {
    Recover(crate::lean::TaskRecoveryRequest),
    Comment {
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
    Send {
        conversation: String,
        text: String,
        to: Option<String>,
    },
    /// Rejects a proposed plan and asks the orchestrator to revise it.
    Reject {
        conversation: String,
        plan: String,
        reason: Option<String>,
        revision_token: Option<String>,
    },
    /// Runs again what the plan left unfinished, through the same sessions.
    Resume {
        conversation: String,
        plan: String,
        revision_token: Option<String>,
    },
    RestartOrchestrator {
        conversation: String,
    },
    Cancel {
        conversation: String,
        revision_token: Option<String>,
    },
    /// Stops starting new plan work, or continues it.
    Pause {
        conversation: String,
        paused: bool,
    },
    /// The user finished commenting on the plan: the orchestrator answers
    /// the draft comments together.
    ReviewComments {
        conversation: String,
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
