//! Bounded JSON transport with duplicate-key rejection and chunked messages.

mod frame;
mod json;

pub use frame::{MAX_FRAME, read, write};
pub use json::decode;

#[cfg(test)]
#[path = "transport/tests.rs"]
mod tests;
