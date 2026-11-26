//! Session-bound MCP commands. Waiting never owns a store or scheduler lock.
use super::tasks::PlannedTask;
use super::turn_result::{TurnResult, required_text};
use super::*;
use rusqlite::OptionalExtension;
use serde_json::Value;

mod context;
mod controls;
mod dependencies;
mod evidence;
mod planning;
mod questions;
mod references;
mod results;
mod session_actions;
pub(crate) mod turn_inputs;
mod validations;

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
            "read_plan_context" | "read_task_contract" => {
                self.read_plan_context(conversation, role, turn, tool, args)
            }
            "prepare_validation"
            | "complete_validation"
            | "invalidate_validation"
            | "read_validations" => self.validation_tool(conversation, turn, tool, args),
            "consult_planning" => self.consult_planning(conversation, role, turn, args),
            "read_planning_result" => self.read_planning_result(conversation, role, turn, args),
            "read_audit" => self.read_audit(conversation, role, turn, args),
            "get_images" => self.conversation_images(conversation, turn, args),
            "record_evidence" | "read_evidence" => {
                self.evidence_tool(conversation, role, turn, tool, args)
            }
            "read_dependency_result" => self.read_dependency_result(conversation, role, turn, args),
            "ask" | "wait_for_answer" => self.question_tool(conversation, role, turn, tool, args),
            "complete" | "select" | "review" | "report_blocker" if task_role(role).is_some() => {
                self.task_result_tool(conversation, role, turn, tool, args)
            }
            "review" | "finish_turn" => self.finish_tool(conversation, role, turn, tool, args),
            "ask_user" | "submit_requirements" => {
                self.interview_tool(conversation, role, turn, tool, args)
            }
            "answer_agent" | "defer_to_user" | "decide_approval" | "retry_task" | "resume_task" => {
                self.orchestrator_tool(conversation, role, turn, tool, args)
            }
            "create_session_pr" => self.orchestrator_tool(conversation, role, turn, tool, args),
            "merge_session_with_main" => {
                require(
                    matches!(role, ORCHESTRATOR | COMMENT_ORCHESTRATOR),
                    "only the orchestrator may merge a completed session",
                )?;
                self.merge_session_with_main_tool(conversation, turn)
            }
            "submit_plan" => self.submit_plan_tool(conversation, role, turn, args),
            "revise_plan" | "request_replan" => {
                self.revision_tool(conversation, role, turn, tool, args)
            }
            _ => Err(Error::Invalid(format!("unknown agent tool {tool}"))),
        }
    }

    fn conversation_images(&self, conversation: &str, turn: &str, args: &Value) -> Result<Value> {
        let provider: String = self.store.db().query_row(
            "SELECT provider FROM turns WHERE id=?1 AND conversation_id=?2",
            rusqlite::params![turn, conversation],
            |row| row.get(0),
        )?;
        crate::require(
            agent::supports_mcp_images(&provider),
            &format!(
                "image retrieval is not supported by the configured {provider} provider; the retained images remain available to a compatible provider"
            ),
        )?;
        let images = self.store.linked_images(conversation)?;
        let offset = usize::try_from(args["offset"].as_u64().unwrap_or(0))
            .map_err(|_| Error::Invalid("image offset exceeds the platform range".into()))?;
        crate::require(offset <= images.len(), "image offset is out of range")?;
        let mut content = vec![];
        if let Some((message, reference)) = images.get(offset) {
            let bytes = self.store.read_image(reference)?;
            content.push(serde_json::json!({"type":"text","text":format!("Image {}/{} attached to message {message}: {}. {}", offset + 1, images.len(), reference.name, if offset + 1 < images.len() { format!("Call get_images with offset {} for the next image.", offset + 1) } else { "All conversation images have been retrieved.".into() })}));
            content.push(serde_json::json!({"type":"image","data":base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes),"mimeType":reference.mime_type}));
        } else {
            content.push(serde_json::json!({"type":"text","text":"No images are attached to messages in this conversation."}));
        }
        Ok(serde_json::json!({"_mcp_content":content}))
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
        let evidence = self.completion_evidence(conversation, turn, args, &result)?;
        self.record_task_result_with_evidence(conversation, role, turn, result, &evidence)
    }
}

#[cfg(test)]
pub(in crate::lean) mod tests;
