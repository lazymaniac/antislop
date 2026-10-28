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

pub fn parse(answer: &str) -> Option<Result<Question>> {
    let mut lines = answer.lines();
    let mut fenced = false;
    while let Some(line) = lines.next() {
        if !fenced && line.trim() == "```question" {
            return Some(parse_question_block(&mut lines));
        }
        if line.trim().starts_with("```") {
            fenced = !fenced;
        }
    }
    None
}

fn parse_question_block<'a>(lines: &mut impl Iterator<Item = &'a str>) -> Result<Question> {
    let mut body = Vec::new();
    for line in lines {
        if line.trim() == "```" {
            return validate_question(&body.join("\n"));
        }
        body.push(line);
    }
    Err(crate::Error::Invalid(
        "incomplete interview question block".into(),
    ))
}

fn validate_question(body: &str) -> Result<Question> {
    let question: Question = serde_json::from_str(body).map_err(|error| {
        crate::Error::Invalid(format!(
            "question block is not valid: {error}; the body must be one JSON object {{\"id\",\"context\",\"question\",\"options\":[{{\"label\",\"description\"}}]}}"
        ))
    })?;
    question.validate()?;
    Ok(question)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_need_context_options_and_a_complete_block() {
        let question = serde_json::json!({"id":"scope","context":"The scope affects delivery.","question":"Which scope?","options":[{"label":"Small","description":"Earlier delivery"},{"label":"Full"}]});
        let block = format!("```question\n{question}\n```");
        let parsed = parse(&block).unwrap().unwrap();
        assert_eq!(parsed.options[0].label, "Small");
        assert!(parsed.options[1].description.is_empty());
        assert!(parse("```question\n{}").unwrap().is_err());
        let strings =
            serde_json::json!({"id":"s","context":"c","question":"q","options":["Keep","Switch"]});
        let parsed = parse(&format!("```question\n{strings}\n```"))
            .unwrap()
            .unwrap();
        assert_eq!(parsed.options[1].label, "Switch");
        let error = parse("```question\nnot json\n```").unwrap().unwrap_err();
        assert!(error.to_string().contains("one JSON object"), "{error}");
        let mut invalid = question;
        invalid["options"] = serde_json::json!([]);
        assert!(
            parse(&format!("```question\n{invalid}\n```"))
                .unwrap()
                .is_err()
        );
    }
}
