//! Session-bound MCP commands. Waiting never owns a store or scheduler lock.
use super::tasks::PlannedTask;
use super::turn_result::{TurnResult, required_text};
use super::*;
use rusqlite::OptionalExtension;
use serde_json::Value;

mod controls;
mod questions;
mod results;

/// The tasks of `submit_plan` or `revise_plan`, with an error that names the
/// task and field the agent must fix.
fn planned_tasks(value: &Value) -> Result<Vec<PlannedTask>> {
    let items = value
        .as_array()
        .ok_or_else(|| Error::Invalid("tasks: expected an array of tasks".into()))?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            serde_json::from_value(item.clone()).map_err(|error| {
                let id = item["id"].as_str().unwrap_or("?");
                Error::Invalid(format!("tasks[{index}] ({id}): {error}"))
            })
        })
        .collect()
}

fn revision_request_key(args: &Value) -> Result<String> {
    let mut payload = args.clone();
    if let Some(fields) = payload.as_object_mut() {
        fields.remove("turn");
    }
    Ok(crate::hash(serde_json::to_vec(&payload)?))
}

impl Core {
    pub(super) fn agent_tool(
        self: &Arc<Self>,
        conversation: &str,
        role: &str,
        turn: &str,
        tool: &str,
        args: &Value,
    ) -> Result<Value> {
        self.require_open_agent_turn(conversation, role, turn)?;
        match tool {
            "ask" | "wait_for_answer" => self.question_tool(conversation, role, turn, tool, args),
            "complete" | "select" | "review" if task_role(role).is_some() => {
                self.task_result_tool(conversation, role, turn, tool, args)
            }
            "review" | "finish_turn" => self.finish_tool(conversation, role, turn, tool, args),
            "ask_user" | "submit_requirements" => {
                self.interview_tool(conversation, role, turn, tool, args)
            }
            "answer_agent" | "defer_to_user" | "retry_task" => {
                self.orchestrator_tool(conversation, role, tool, args)
            }
            "submit_plan" => self.submit_plan_tool(conversation, role, turn, args),
            "submit_ledger" | "submit_audit" | "submit_audit_review" => {
                self.audit_tool(conversation, role, turn, tool, args)
            }
            "revise_plan" | "request_replan" => self.revision_tool(conversation, role, tool, args),
            _ => Err(Error::Invalid(format!("unknown agent tool {tool}"))),
        }
    }

    fn task_result_tool(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        tool: &str,
        args: &Value,
    ) -> Result<Value> {
        let result = turn_result::task_tool_result(tool, args)?;
        self.record_task_result(conversation, role, turn, result)
    }
}

#[cfg(test)]
mod tests;
