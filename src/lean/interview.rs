//! Structured questions from the read-only orchestrator. Answers remain user
//! messages, so the same transcript and requirements sources cover both UIs.
use crate::{Result, require};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "RawChoice")]
pub struct Choice {
    pub label: String,
    #[serde(default)]
    pub description: String,
}

/// A bare string is a choice with only a label; agents write options both ways.
#[derive(Deserialize)]
#[serde(untagged)]
enum RawChoice {
    Label(String),
    Full {
        label: String,
        #[serde(default)]
        description: String,
    },
}

impl From<RawChoice> for Choice {
    fn from(raw: RawChoice) -> Self {
        match raw {
            RawChoice::Label(label) => Self {
                label,
                description: String::new(),
            },
            RawChoice::Full { label, description } => Self { label, description },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub id: String,
    pub context: String,
    pub question: String,
    pub options: Vec<Choice>,
}

impl Question {
    pub fn validate(&self) -> Result<()> {
        require(!self.id.trim().is_empty(), "interview question needs an id")?;
        require(
            !self.context.trim().is_empty() && !self.question.trim().is_empty(),
            "interview question needs context and a question",
        )?;
        require(
            (2..=6).contains(&self.options.len()),
            "interview question needs 2–6 options",
        )?;
        require(
            self.options
                .iter()
                .all(|option| !option.label.trim().is_empty()),
            "interview options need labels",
        )?;
        require(
            self.id.len() <= 128 && !self.id.chars().any(char::is_control),
            "question id must be at most 128 bytes without control characters",
        )?;
        require(
            serde_json::to_vec(self)?.len() <= 16384,
            "question exceeds 16 KiB",
        )?;
        Ok(())
    }

    pub fn transcript(&self) -> String {
        let options = self
            .options
            .iter()
            .enumerate()
            .map(|(index, choice)| {
                format!("{}. {} — {}", index + 1, choice.label, choice.description)
            })
            .collect::<Vec<_>>()
            .join("\n");
        format!(
            "{}\n\n{}\n\n{}\n\nChoose an option or write your own answer in the question card. /question reopens it.",
            self.context, self.question, options
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_need_context_and_two_to_six_labelled_options() {
        let question: Question = serde_json::from_value(serde_json::json!({"id":"scope","context":"The scope affects delivery.","question":"Which scope?","options":[{"label":"Small","description":"Earlier delivery"},"Full"]})).unwrap();
        question.validate().unwrap();
        assert!(question.options[1].description.is_empty());
        let mut few = question.clone();
        few.options.truncate(1);
        assert!(few.validate().is_err());
        let mut unlabelled = question;
        unlabelled.options[0].label = " ".into();
        assert!(unlabelled.validate().is_err());
    }
}
