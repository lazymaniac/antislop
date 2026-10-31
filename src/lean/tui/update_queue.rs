//! Bounded FIFO delivery for background TUI updates.
mod queue;

pub use queue::{QueueClosed, QueueEmpty, UpdateReceiver, UpdateSender, update_channel};
