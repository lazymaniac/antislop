use super::ConversationRequest;

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
            Self::RevisePlan {
                conversation,
                operation_id,
                ..
            } => (conversation, operation_id, "revise_plan"),
            _ => return None,
        };
        Some((conversation, operation_id.as_deref()?, kind))
    }
}
