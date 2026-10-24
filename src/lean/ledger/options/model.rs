use super::super::Run;
use crate::{Result, require};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Alternative {
    pub id: String,
    pub approach: String,
    pub evaluations: Vec<String>,
    pub risks: String,
    pub validation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SolutionChoice {
    pub run_id: String,
    pub spec_revision: i64,
    pub criteria: Vec<String>,
    pub alternatives: Vec<Alternative>,
    pub selected: String,
    #[serde(deserialize_with = "deserialize_rationale")]
    pub rationale: String,
    #[serde(default)]
    pub review_findings: Vec<String>,
}
fn deserialize_rationale<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<String, D::Error> {
    fn text(value: serde_json::Value) -> Option<String> {
        match value {
            serde_json::Value::String(text) => Some(text),
            serde_json::Value::Array(values) => Some(
                values
                    .into_iter()
                    .map(text)
                    .collect::<Option<Vec<_>>>()?
                    .join("\n"),
            ),
            serde_json::Value::Object(fields) => Some(
                fields
                    .into_iter()
                    .map(|(key, value)| Some(format!("{key}: {}", text(value)?)))
                    .collect::<Option<Vec<_>>>()?
                    .join("\n"),
            ),
            _ => None,
        }
    }
    let value = serde_json::Value::deserialize(deserializer)?;
    text(value).ok_or_else(|| {
        serde::de::Error::custom("rationale must contain text or named text sections")
    })
}

impl SolutionChoice {
    pub(super) fn validate(&self, run: &Run) -> Result<()> {
        self.validate_revision(run)?;
        self.validate_alternatives()?;
        self.validate_selection()
    }

    fn validate_revision(&self, run: &Run) -> Result<()> {
        require(
            self.run_id == run.id && self.spec_revision == run.spec_revision,
            "options refer to a different requirements revision",
        )
    }

    fn validate_alternatives(&self) -> Result<()> {
        require(
            (2..=4).contains(&self.alternatives.len()),
            "compare two to four real implementation alternatives",
        )?;
        require(
            !self.criteria.is_empty() && self.criteria.iter().all(|s| !s.trim().is_empty()),
            "options need explicit shared selection criteria",
        )?;
        require(
            !self.rationale.trim().is_empty(),
            "solution choice needs an evidence-based rationale",
        )?;
        self.validate_alternative_details()
    }

    fn validate_alternative_details(&self) -> Result<()> {
        let mut ids = HashSet::new();
        for option in &self.alternatives {
            validate_alternative(option, self.criteria.len(), &mut ids)?;
        }
        Ok(())
    }

    fn validate_selection(&self) -> Result<()> {
        require(
            self.alternatives
                .iter()
                .any(|option| option.id == self.selected),
            "selected solution must be one of the compared alternatives",
        )
    }
}

fn validate_alternative<'a>(
    option: &'a Alternative,
    criteria_count: usize,
    ids: &mut HashSet<&'a str>,
) -> Result<()> {
    require(
        !option.id.trim().is_empty() && ids.insert(option.id.as_str()),
        "alternative IDs must be nonempty and unique",
    )?;
    require(
        [&option.approach, &option.risks, &option.validation]
            .iter()
            .all(|text| !text.trim().is_empty()),
        "each alternative needs an approach, risks and validation",
    )?;
    require(
        option.evaluations.len() == criteria_count
            && option
                .evaluations
                .iter()
                .all(|text| !text.trim().is_empty()),
        "evaluate every alternative against every shared criterion",
    )
}
