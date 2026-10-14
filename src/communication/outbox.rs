//! A common, provider-independent message channel in completed structured output.
//! Runtime binds authorship to the observed turn; models cannot supply a sender.
use super::MessageKind;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Target {
    Agent { agent_id: String },
    Broadcast,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Message {
    pub target: Target,
    pub kind: MessageKind,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    pub output_hash: String,
    pub message_ids: Vec<String>,
    pub rejection: Option<String>,
}

pub(crate) fn schema() -> Value {
    json!({"type":"array","items":{"type":"object","additionalProperties":false,
        "required":["target","kind","text"],"properties":{
            "target":{"anyOf":[
                {"type":"object","additionalProperties":false,"required":["type","agent_id"],
                    "properties":{"type":{"const":"agent"},"agent_id":{"type":"string"}}},
                {"type":"object","additionalProperties":false,"required":["type"],
                    "properties":{"type":{"const":"broadcast"}}}]},
            "kind":{"type":"string","enum":["question","answer","handoff","finding","decision","status"]},
            "text":{"type":"string"}}}})
}
