//! The loopback MCP gateway exposed to interactive agents.
mod dispatch;
mod gateway;
mod schema;
mod tools;
mod transport;

pub(crate) use gateway::Finish;
pub(crate) use gateway::Gateway;
pub(crate) use gateway::{CANCELLED, Handler, SERVER, TOOL, TOOLS};
pub(crate) use tools::allowed_tools;
#[cfg(test)]
use tools::tool_list;
pub use transport::bridge;

#[cfg(test)]
use crate::lean::native::invalid;
#[cfg(test)]
use dispatch::{call, dispatch};
#[cfg(test)]
use gateway::{Shared, State, is_terminal};
#[cfg(test)]
use serde_json::{Value, json};
#[cfg(test)]
use std::sync::atomic::AtomicBool;
#[cfg(test)]
use std::sync::{Arc, Condvar, Mutex};
#[cfg(test)]
use std::time::{Duration, Instant};
#[cfg(test)]
use transport::forward;
#[cfg(test)]
include!("mcp/tests.rs");
