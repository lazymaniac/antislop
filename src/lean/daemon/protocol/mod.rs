//! Local daemon protocol: grouped application requests, a flat tagged wire
//! model and a separate versioned envelope. See README.md for compatibility
//! rules and the independent operation inventory.

mod envelope;
mod operations;
mod request;
mod wire_serde_impl;
pub(crate) use envelope::RequestEnvelope;
#[cfg(test)]
use operations::WireRequest;
pub use request::{
    ControlRequest, ConversationRequest, HistoryRequest, Request, SessionActionKind,
    SessionActionResult, SessionRequest, TerminalRequest,
};

#[cfg(test)]
mod tests;
