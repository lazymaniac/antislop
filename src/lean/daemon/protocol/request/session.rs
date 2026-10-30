#[derive(Debug, Clone, PartialEq)]
pub enum SessionRequest {
    RunContext {
        conversation: String,
        turn: String,
    },
    SessionEvents {
        conversation: String,
        turn: String,
        after: u64,
        limit: Option<usize>,
        include_messages: Option<bool>,
    },
    SessionContent {
        conversation: String,
        event: u64,
        field: String,
        offset: u64,
    },
    Sessions {
        conversation: String,
    },
}
