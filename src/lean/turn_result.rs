//! Typed task results. Prose is evidence, never a control protocol.
mod review;
mod tool_results;
mod types;
mod validation;

pub use review::task_tool_result;
pub(crate) use tool_results::required_text;
pub use tool_results::{SubmittedPlan, report_with_deliverables, tool_result};
pub use types::{Alternative, Selection, TurnResult};
pub(crate) use validation::validate_worker_commit_subject;

#[cfg(test)]
include!("turn_result/tests.rs");
