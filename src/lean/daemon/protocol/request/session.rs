use serde::{Deserialize, Serialize};

/// A user-selected next step for a completed session. The orchestrator owns
/// the work; this request only hands the selection back to that conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionActionKind {
    CreatePr,
    MergeWithMain,
    ContinueWork,
}

/// Outcomes an orchestrator may report for a selected session action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionActionResult {
    Queued {
        message: u64,
    },
    Succeeded {
        #[serde(deserialize_with = "nonempty_text")]
        summary: String,
    },
    Failed {
        #[serde(deserialize_with = "nonempty_text")]
        details: String,
    },
    MissingInputs {
        #[serde(deserialize_with = "nonempty_inputs")]
        inputs: Vec<String>,
    },
}

fn nonempty_text<'de, D>(deserializer: D) -> std::result::Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let text = String::deserialize(deserializer)?;
    if text.trim().is_empty() {
        return Err(serde::de::Error::custom("text must not be empty"));
    }
    Ok(text)
}

fn nonempty_inputs<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let inputs = Vec::<String>::deserialize(deserializer)?;
    if inputs.is_empty() || inputs.iter().any(|input| input.trim().is_empty()) {
        return Err(serde::de::Error::custom(
            "missing-input results require nonempty input names",
        ));
    }
    Ok(inputs)
}
