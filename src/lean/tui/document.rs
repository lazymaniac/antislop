//! Disk-backed inspection documents with asynchronous viewport indexing.

mod builder;
mod chunk;
mod layout;
mod search;
#[cfg(test)]
mod tests;
mod wrap;

pub use builder::{Builder, Document, Viewport};

pub(super) const CHUNK: usize = 64 * 1024;
pub(super) const INDEX_BYTES: u64 = 17;
