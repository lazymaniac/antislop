//! Structured questions from the read-only orchestrator. Answers remain user
//! messages, so the same transcript and requirements sources cover both UIs.
use crate::{Result, require};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub label: String,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub id: String,
    pub context: String,
    pub question: String,
    pub options: Vec<Choice>,
}

impl Question {
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
            "{}\n\n{}\n\n{}\n\nChoose an option or write your own answer in the interview card. /interview reopens it.",
            self.context, self.question, options
        )
    }
}

pub fn parse(answer: &str) -> Option<Result<Question>> {
    let mut lines = answer.lines();
    let mut fenced = false;
    while let Some(line) = lines.next() {
        if !fenced && line.trim() == "```question" {
            let mut body = Vec::new();
            for line in lines.by_ref() {
                if line.trim() == "```" {
                    return Some((|| {
                        let question: Question = serde_json::from_str(&body.join("\n"))?;
                        require(
                            !question.id.trim().is_empty(),
                            "interview question needs an id",
                        )?;
                        require(
                            !question.context.trim().is_empty()
                                && !question.question.trim().is_empty(),
                            "interview question needs context and a question",
                        )?;
                        require(
                            (2..=6).contains(&question.options.len()),
                            "interview question needs 2–6 options",
                        )?;
                        require(
                            question
                                .options
                                .iter()
                                .all(|option| !option.label.trim().is_empty()),
                            "interview options need labels",
                        )?;
                        Ok(question)
                    })());
                }
                body.push(line);
            }
            return Some(Err(crate::Error::Invalid(
                "incomplete interview question block".into(),
            )));
        }
        if line.trim().starts_with("```") {
            fenced = !fenced;
        }
    }
    None
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
        let mut invalid = question;
        invalid["options"] = serde_json::json!([]);
        assert!(
            parse(&format!("```question\n{invalid}\n```"))
                .unwrap()
                .is_err()
        );
    }
}
