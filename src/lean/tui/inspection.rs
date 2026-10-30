//! History retrieval runs on the existing task-fetch thread, never while drawing.
use super::*;
mod history;
mod session;
pub(super) use history::message_document;
pub(super) use history::task_snapshot;
pub(super) use session::run_snapshot;

pub(super) struct Snapshot {
    pub task: Value,
    pub documents: std::collections::HashMap<String, document::Document>,
}
