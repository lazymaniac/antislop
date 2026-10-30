#[derive(Debug, Clone, PartialEq)]
pub enum ConversationRequest {
    Recover(crate::lean::TaskRecoveryRequest),
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
    Send {
        conversation: String,
        text: String,
    },
    SendAddressed {
        conversation: String,
        text: String,
        to: Option<String>,
    },
    /// Starts exactly the plan the user saw.
    Accept {
        conversation: String,
        plan: String,
    },
    AcceptScoped {
        conversation: String,
        plan: String,
        scope_hash: Option<String>,
    },
    /// Rejects a proposed plan and asks the orchestrator to revise it.
    Reject {
        conversation: String,
        plan: String,
        reason: Option<String>,
    },
    RejectScoped {
        conversation: String,
        plan: String,
        reason: Option<String>,
        scope_hash: Option<String>,
    },
    /// Runs again what the plan left unfinished, through the same sessions.
    Resume {
        conversation: String,
        plan: String,
    },
    ResumeScoped {
        conversation: String,
        plan: String,
        scope_hash: Option<String>,
    },
    RestartOrchestrator {
        conversation: String,
    },
    Cancel {
        conversation: String,
    },
    CancelScoped {
        conversation: String,
        scope_hash: Option<String>,
    },
    /// Stops starting new plan work, or continues it.
    Pause {
        conversation: String,
        paused: bool,
    },
    /// Runs the conversation on by itself, or under the user, from now on.
    Mode {
        conversation: String,
        autonomous: bool,
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
    Comment {
        conversation: String,
        target: String,
        text: String,
    },
    /// The user's comment scoped to a specific plan.
    CommentScoped {
        conversation: String,
        plan: Option<String>,
        target: String,
        text: String,
    },
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
    },
    ReviewScoped {
        conversation: String,
        plan: String,
        id: String,
        scope_hash: Option<String>,
    },
    /// One task of a plan with its latest report and verdict.
    Task {
        plan: String,
        id: String,
    },
    /// Inspector snapshot and checks without a Git freshness fingerprint.
    TaskChecks {
        conversation: Option<String>,
        plan: String,
        id: String,
    },
    TaskScoped {
        conversation: Option<String>,
        plan: String,
        id: String,
    },
}
