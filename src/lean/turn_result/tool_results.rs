use super::review::task_tool_result;
use super::types::{Alternative, Selection, TurnResult};
use crate::lean::tasks::PlannedTask;
use crate::{Result, require};
use serde::Deserialize;

/// What a reviewer is shown of a completed turn: the prose report plus the
/// selection the runtime recorded. The selection is the work product itself;
/// a reviewer handed only the prose cannot judge it and rejects the task for
/// payload it never saw.
pub fn report_with_deliverables(report: &str, selection: Option<&Selection>) -> String {
    let mut text = if let Some(selection) = selection {
        if report == selection.rationale {
            String::new()
        } else if let Some(evidence) = report.strip_prefix(&selection.rationale).filter(|tail| {
            tail.starts_with("\n\nMCP execution evidence (read_evidence; immutable records):")
        }) {
            evidence.to_owned()
        } else {
            report.to_owned()
        }
    } else {
        report.to_owned()
    };
    if let Some(selection) = selection {
        text.push_str("\n\nRecorded selection:\n");
        text.push_str(&crate::lean::prompt_context::markdown(selection));
    }
    text
}

impl Selection {
    pub fn validate(&self) -> Result<()> {
        require(
            (1..=4).contains(&self.alternatives.len()),
            "alternatives: compare 1–4 variants; justify a single feasible approach in rationale",
        )?;
        let mut ids = std::collections::BTreeSet::new();
        for alternative in &self.alternatives {
            require(
                !alternative.id.trim().is_empty() && ids.insert(&alternative.id),
                "alternatives: each needs its own id",
            )?;
            require(
                !alternative.approach.trim().is_empty() && !alternative.tradeoffs.trim().is_empty(),
                &format!(
                    "alternatives ({}): approach and tradeoffs are required",
                    alternative.id
                ),
            )?;
        }
        require(
            self.alternatives.iter().any(|a| a.id == self.selected),
            "selected: name one of the alternatives' ids",
        )?;
        if !self.ranking.is_empty()
            || !self.decisive_criterion.is_empty()
            || self.ranking_override.is_some()
        {
            let ranked: std::collections::BTreeSet<_> = self.ranking.iter().collect();
            require(
                ranked == ids && self.ranking.len() == ids.len(),
                "ranking: list every alternative ID exactly once, strongest first",
            )?;
            require(
                !self.decisive_criterion.trim().is_empty(),
                "decisive_criterion: name the criterion that determined the choice",
            )?;
            if self.ranking.first() != Some(&self.selected) {
                require(
                    self.ranking_override
                        .as_deref()
                        .is_some_and(|reason| !reason.trim().is_empty()),
                    "selected differs from the first ranked alternative; supply ranking_override explaining why, or correct the ranking or selection",
                )?;
            } else {
                require(
                    self.ranking_override.is_none(),
                    "ranking_override applies only when selected differs from first place",
                )?;
            }
        }
        require(!self.rationale.trim().is_empty(), "rationale: required")
    }

    /// The chosen alternative.
    pub fn chosen(&self) -> Option<&Alternative> {
        self.alternatives.iter().find(|a| a.id == self.selected)
    }
}

/// The result a terminal tool call records, from its arguments alone.
/// `wait_for_answer` needs the turn's open question and is built by the
/// caller.
pub fn tool_result(tool: &str, args: &serde_json::Value) -> Result<TurnResult> {
    match tool {
        "finish_turn" => report_tool(args),
        "submit_plan" => plan_tool(args),
        "ask_user" => question_tool(args),
        "submit_requirements" => Ok(TurnResult::Requirements {
            title: args["title"].as_str().unwrap_or_default().trim().to_owned(),
            markdown: required_text(args, "markdown")?,
        }),
        _ => task_tool_result(tool, args),
    }
}

pub(crate) fn required_text(args: &serde_json::Value, field: &str) -> Result<String> {
    args[field]
        .as_str()
        .filter(|text| !text.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| crate::Error::Invalid(format!("{field}: required")))
}

fn report_tool(args: &serde_json::Value) -> Result<TurnResult> {
    Ok(TurnResult::Report {
        text: required_text(args, "text")?,
        addressed: addressed(args)?,
    })
}

fn plan_tool(args: &serde_json::Value) -> Result<TurnResult> {
    let tasks = serde_json::from_value(args["tasks"].clone())
        .map_err(|error| crate::Error::Invalid(format!("tasks: {error}")))?;
    Ok(TurnResult::Plan {
        scope: if args["scope"].is_null() {
            None
        } else {
            Some(required_text(args, "scope")?)
        },
        document: required_text(args, "document")?,
        tasks,
        keep: kept(args)?,
        addressed: addressed(args)?,
    })
}

fn question_tool(args: &serde_json::Value) -> Result<TurnResult> {
    let options = serde_json::from_value(args["options"].clone())
        .map_err(|error| crate::Error::Invalid(format!("options: {error}")))?;
    let mut question = crate::lean::interview::Question {
        id: String::new(),
        context: required_text(args, "context")?,
        question: required_text(args, "question")?,
        options,
    };
    question.id = crate::hash(serde_json::to_vec(&question)?)[..12].to_owned();
    question.validate()?;
    Ok(TurnResult::Question { question })
}

/// The comment ids an `addressed` argument lists.
fn addressed(args: &serde_json::Value) -> Result<Option<Vec<u64>>> {
    #[derive(Deserialize)]
    struct Addressed {
        id: u64,
    }
    if args["addressed"].is_null() {
        return Ok(None);
    }
    let items: Vec<Addressed> = serde_json::from_value(args["addressed"].clone())
        .map_err(|error| crate::Error::Invalid(format!("addressed: {error}")))?;
    Ok(Some(items.into_iter().map(|item| item.id).collect()))
}

/// The tasks a plan submitted and the ids of the approved tasks it keeps.
pub type SubmittedPlan = (Vec<PlannedTask>, Vec<String>);

fn kept(args: &serde_json::Value) -> Result<Vec<String>> {
    if args["keep"].is_null() {
        return Ok(Vec::new());
    }
    serde_json::from_value(args["keep"].clone())
        .map_err(|error| crate::Error::Invalid(format!("keep: {error}")))
}
