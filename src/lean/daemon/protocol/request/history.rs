#[derive(Debug, Clone, PartialEq)]
pub enum HistoryRequest {
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
        references: Option<bool>,
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
    Events {
        conversation: String,
        after: u64,
        wait_ms: u64,
    },
    EventsPaged {
        conversation: String,
        after: u64,
        wait_ms: u64,
        limit: Option<usize>,
    },
    History {
        conversation: String,
        after: u64,
        limit: Option<usize>,
    },
    Mail {
        conversation: String,
    },
}
