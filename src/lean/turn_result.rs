//! Typed task results. Prose is evidence, never a control protocol.
use super::tasks::{PlannedTask, TaskContract, Verdict};
use crate::{Result, require};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
// Not `deny_unknown_fields`: stored results from before a field was dropped
// (a completed turn's `outputs`) must still load. Agents never write this
// type directly; each MCP tool has its own strict schema.
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TurnResult {
    Report {
        text: String,
        /// The open comments the orchestrator says this answer settles.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        addressed: Option<Vec<u64>>,
    },
    /// A task graph from `submit_plan`, already checked in the turn.
    Plan {
        document: String,
        tasks: Vec<PlannedTask>,
        /// Approved tasks of the plan that ran, kept with their results.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        keep: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        addressed: Option<Vec<u64>>,
    },
    /// A question for the user from `ask_user`, its id given by the runtime.
    Question {
        question: super::interview::Question,
    },
    /// The interview's requirements summary from `submit_requirements`.
    Requirements {
        markdown: String,
    },
    Completed {
        summary: String,
        report: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selection: Option<Selection>,
    },
    Review {
        report: String,
        verdict: Verdict,
    },
    NeedsInput {
        request: String,
        checkpoint: String,
    },
    /// The requirements `submit_ledger` extracted, ranges and run filled in
    /// by the runtime.
    Ledger {
        extraction: super::ledger::Extraction,
    },
    /// The audit of `submit_audit`; it approves unless a finding blocks.
    Audit {
        findings: Vec<serde_json::Value>,
        evidence: Vec<super::ledger::AuditEvidence>,
    },
    /// The assessments of `submit_audit_review`, by the evidence ids the
    /// runtime gave the audit's proposals.
    AuditReview {
        note: String,
        assessments: Vec<super::ledger::AuditAssessment>,
    },
}

/// The approach an options task chose and what it was weighed against.
/// Stored selections may carry fields of the earlier, longer form; they are
/// ignored. The `select` tool's schema is strict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    pub alternatives: Vec<Alternative>,
    pub selected: String,
    pub rationale: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Alternative {
    pub id: String,
    pub approach: String,
    pub tradeoffs: String,
}

/// Severities that keep a task from being approved.
const BLOCKING: [&str; 3] = ["critical", "high", "medium"];

/// What a verifier is shown of a completed turn: the prose report plus the
/// selection the runtime recorded. The selection is the work product itself;
/// a verifier handed only the prose cannot judge it and rejects the task for
/// payload it never saw.
pub fn report_with_deliverables(report: &str, selection: Option<&Selection>) -> String {
    let mut text = report.to_owned();
    if let Some(selection) = selection
        && let Ok(json) = serde_json::to_string_pretty(selection)
    {
        text.push_str(&format!(
            "\n\nRecorded structured selection (accepted by the runtime; this is the deliverable):\n```json\n{json}\n```"
        ));
    }
    text
}

impl Selection {
    pub fn validate(&self) -> Result<()> {
        require(
            (2..=4).contains(&self.alternatives.len()),
            "alternatives: compare 2–4 variants",
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
        require(!self.rationale.trim().is_empty(), "rationale: required")
    }

    /// The chosen alternative.
    pub fn chosen(&self) -> Option<&Alternative> {
        self.alternatives.iter().find(|a| a.id == self.selected)
    }
}

/// One problem a reviewer found, as the `review` tool takes it. A task
/// verifier names the check it belongs to; a plan, audit or code reviewer has
/// no checks.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Finding {
    #[serde(default)]
    check: Option<String>,
    severity: String,
    issue: String,
    #[serde(rename = "where")]
    location: String,
}

/// The result a terminal tool call records, from its arguments alone.
/// `wait_for_answer` needs the turn's open question and is built by the
/// caller.
pub fn tool_result(tool: &str, args: &serde_json::Value) -> Result<TurnResult> {
    let text = |field: &str| {
        args[field]
            .as_str()
            .filter(|text| !text.trim().is_empty())
            .map(str::to_owned)
            .ok_or_else(|| crate::Error::Invalid(format!("{field}: required")))
    };
    match tool {
        "finish_turn" => Ok(TurnResult::Report {
            text: text("text")?,
            addressed: addressed(args)?,
        }),
        "submit_plan" => Ok(TurnResult::Plan {
            document: text("document")?,
            tasks: serde_json::from_value(args["tasks"].clone())
                .map_err(|error| crate::Error::Invalid(format!("tasks: {error}")))?,
            keep: kept(args)?,
            addressed: addressed(args)?,
        }),
        "ask_user" => {
            let options = serde_json::from_value(args["options"].clone())
                .map_err(|error| crate::Error::Invalid(format!("options: {error}")))?;
            let mut question = super::interview::Question {
                id: String::new(),
                context: text("context")?,
                question: text("question")?,
                options,
            };
            // The same question asked again is the same question.
            question.id = crate::hash(serde_json::to_vec(&question)?)[..12].to_owned();
            question.validate()?;
            Ok(TurnResult::Question { question })
        }
        "submit_requirements" => Ok(TurnResult::Requirements {
            markdown: text("markdown")?,
        }),
        _ => task_tool_result(tool, args),
    }
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

/// The result of a task tool call.
pub fn task_tool_result(tool: &str, args: &serde_json::Value) -> Result<TurnResult> {
    let text = |field: &str| args[field].as_str().unwrap_or_default().to_owned();
    match tool {
        "complete" => Ok(TurnResult::Completed {
            summary: text("summary"),
            report: text("report"),
            selection: None,
        }),
        "select" => {
            let selection = Selection {
                alternatives: serde_json::from_value(args["alternatives"].clone())
                    .map_err(|error| crate::Error::Invalid(format!("alternatives: {error}")))?,
                selected: text("selected"),
                rationale: text("rationale"),
            };
            selection.validate()?;
            let chosen = selection.chosen().map_or("", |a| a.approach.as_str());
            Ok(TurnResult::Completed {
                summary: format!("Selected {}: {chosen}", selection.selected),
                report: selection.rationale.clone(),
                selection: Some(selection),
            })
        }
        "review" => review(text("report"), args),
        other => Err(crate::Error::Invalid(format!(
            "{other} is not a task result"
        ))),
    }
}

/// A verifier's review with the verdict the runtime derives from it: approved
/// when every check is ok or n/a and no finding blocks; each finding gets an
/// id stable across rounds, from its check and location.
fn review(report: String, args: &serde_json::Value) -> Result<TurnResult> {
    let checks = match &args["checks"] {
        serde_json::Value::Null => serde_json::Map::new(),
        serde_json::Value::Object(checks) => checks.clone(),
        _ => {
            return Err(crate::Error::Invalid(
                "checks: expected an object by check id".into(),
            ));
        }
    };
    for (id, check) in &checks {
        require(
            matches!(
                check["status"].as_str(),
                Some("ok" | "issue" | "blocked" | "n/a")
            ) && check["note"].as_str().is_some_and(|n| !n.trim().is_empty()),
            &format!("checks.{id}: give status (ok, issue, blocked or n/a) and a note"),
        )?;
    }
    let findings: Vec<Finding> = match &args["findings"] {
        serde_json::Value::Null => Vec::new(),
        value => serde_json::from_value(value.clone())
            .map_err(|error| crate::Error::Invalid(format!("findings: {error}")))?,
    };
    let mut ids = std::collections::BTreeSet::new();
    let mut recorded = Vec::new();
    for finding in &findings {
        require(
            matches!(
                finding.severity.as_str(),
                "critical" | "high" | "medium" | "low"
            ),
            "findings.severity: critical, high, medium or low",
        )?;
        let base = format!(
            "{}:{}",
            finding.check.as_deref().unwrap_or("finding"),
            &crate::hash(finding.location.trim().as_bytes())[..8]
        );
        let mut id = base.clone();
        let mut n = 2;
        while !ids.insert(id.clone()) {
            id = format!("{base}-{n}");
            n += 1;
        }
        let mut entry = serde_json::json!({
            "id": id, "severity": finding.severity,
            "issue": finding.issue, "where": finding.location,
        });
        if let Some(check) = &finding.check {
            entry["check"] = check.clone().into();
        }
        recorded.push(entry);
    }
    let resolved: Vec<String> = match &args["resolved"] {
        serde_json::Value::Null => Vec::new(),
        value => serde_json::from_value(value.clone())
            .map_err(|error| crate::Error::Invalid(format!("resolved: {error}")))?,
    };
    let approved = checks
        .values()
        .all(|check| matches!(check["status"].as_str(), Some("ok" | "n/a")))
        && !findings
            .iter()
            .any(|f| BLOCKING.contains(&f.severity.as_str()));
    Ok(TurnResult::Review {
        report,
        verdict: Verdict {
            approved,
            findings: recorded,
            coverage: checks,
            resolved,
        },
    })
}

impl TurnResult {
    /// How the gateway hands a result over: a plain report as its bare
    /// text, every other result as JSON.
    pub fn encode(&self) -> Result<String> {
        Ok(match self {
            Self::Report {
                text,
                addressed: None,
            } => text.clone(),
            other => serde_json::to_string(other)?,
        })
    }

    /// What a role without a task contract ended its turn with; the inverse
    /// of [`Self::encode`].
    pub fn decode(text: &str) -> Self {
        serde_json::from_str::<Self>(text)
            .ok()
            .filter(|result| {
                !matches!(
                    result,
                    Self::Report {
                        addressed: None,
                        ..
                    }
                )
            })
            .unwrap_or_else(|| Self::Report {
                text: text.to_owned(),
                addressed: None,
            })
    }

    /// The words of a planning answer and the plan it submitted, if any: its
    /// tasks and the approved tasks it keeps.
    pub fn plan_answer(text: &str) -> (String, Option<SubmittedPlan>) {
        match Self::decode(text) {
            Self::Plan {
                document,
                tasks,
                keep,
                ..
            } => (document, Some((tasks, keep))),
            other => (other.text(), None),
        }
    }

    /// The verdict of a plan, audit or code review; `None` when the reviewer
    /// ended its turn without one.
    pub fn review_verdict(text: &str) -> Option<Verdict> {
        match Self::decode(text) {
            Self::Review { verdict, .. } => Some(verdict),
            _ => None,
        }
    }

    /// The comments this answer says it settled.
    pub fn addressed(&self) -> Option<&[u64]> {
        match self {
            Self::Report { addressed, .. } | Self::Plan { addressed, .. } => addressed.as_deref(),
            _ => None,
        }
    }

    pub fn validate_task(&self, spec: &TaskContract, verifier: bool) -> Result<()> {
        require(
            serde_json::to_vec(self)?.len() <= 256 * 1024,
            "result exceeds 256 KiB; keep the report compact and reference task artifacts",
        )?;
        match (self, verifier) {
            (
                Self::Completed {
                    summary,
                    report,
                    selection,
                },
                false,
            ) => {
                require(
                    !summary.trim().is_empty() && !report.trim().is_empty(),
                    "summary and report: required",
                )?;
                match (spec.kind == crate::config::WorkKind::Options, selection) {
                    (true, Some(selection)) => selection.validate(),
                    (true, None) => Err(crate::Error::Invalid(
                        "an options task ends with select, naming the alternatives and the chosen one".into(),
                    )),
                    (false, Some(_)) => Err(crate::Error::Invalid(
                        "select is only for options tasks; end this task with complete".into(),
                    )),
                    (false, None) => Ok(()),
                }
            }
            (Self::Review { report, verdict }, true) => {
                require(!report.trim().is_empty(), "report: required")?;
                let obligations = super::tasks::obligations(spec);
                require(
                    verdict
                        .coverage
                        .keys()
                        .all(|id| obligations.iter().any(|(assigned, _)| assigned == id)),
                    "checks: use only the check ids this prompt lists",
                )?;
                let mut finding_ids = std::collections::BTreeSet::new();
                for finding in &verdict.findings {
                    let id = finding["id"]
                        .as_str()
                        .filter(|id| !id.is_empty())
                        .ok_or_else(|| {
                            crate::Error::Invalid("findings: stable id required".into())
                        })?;
                    require(
                        matches!(
                            finding["severity"].as_str(),
                            Some("critical" | "high" | "medium" | "low")
                        ) && finding["issue"]
                            .as_str()
                            .is_some_and(|s| !s.trim().is_empty())
                            && finding["where"]
                                .as_str()
                                .is_some_and(|s| !s.trim().is_empty()),
                        "findings: severity, issue and where are required",
                    )?;
                    require(finding_ids.insert(id), "findings: duplicate id")?;
                    require(
                        obligations.iter().any(|(id, _)| finding["check"] == *id),
                        &format!(
                            "findings.check: {} is not one of the check ids this prompt lists; name the check each finding belongs to",
                            finding["check"]
                        ),
                    )?;
                }
                require(
                    super::tasks::unaccounted(verdict, &obligations).is_empty(),
                    &format!(
                        "checks: give a status and note for every listed check; missing: {}",
                        super::tasks::unaccounted(verdict, &obligations)
                            .iter()
                            .map(|(id, _)| id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                )?;
                require(
                    !verdict.approved
                        || !verdict.findings.iter().any(|f| {
                            matches!(f["severity"].as_str(), Some("high" | "medium" | "critical"))
                        }),
                    "verdict: approval contradicts blocking findings",
                )?;
                require(
                    !verdict.approved
                        || verdict.coverage.values().all(|assessment| {
                            matches!(assessment["status"].as_str(), Some("ok" | "n/a"))
                        }),
                    "verdict: approval contradicts unsettled checks",
                )
            }
            (
                Self::NeedsInput {
                    request,
                    checkpoint,
                },
                _,
            ) => require(
                !request.trim().is_empty() && !checkpoint.trim().is_empty(),
                "ask a question with ask before waiting for its answer",
            ),
            (_, true) => Err(crate::Error::Invalid(
                "a verifier ends its turn with review or wait_for_answer".into(),
            )),
            _ => Err(crate::Error::Invalid(
                "a worker ends its turn with complete (select for an options task) or wait_for_answer".into(),
            )),
        }
    }

    /// The words of this result as the conversation shows them.
    pub fn text(&self) -> String {
        match self {
            Self::Report { text, .. }
            | Self::Plan { document: text, .. }
            | Self::Requirements { markdown: text } => text.clone(),
            Self::Completed { report, .. } | Self::Review { report, .. } => report.clone(),
            Self::NeedsInput { checkpoint, .. } => checkpoint.clone(),
            Self::Question { question } => question.transcript(),
            Self::Ledger { extraction } => {
                format!("Extracted {} requirement(s).", extraction.entries.len())
            }
            Self::Audit { findings, evidence } => format!(
                "Audit: {} finding(s), {} piece(s) of evidence.",
                findings.len(),
                evidence.len()
            ),
            Self::AuditReview { note, .. } => note.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn task() -> TaskContract {
        serde_json::from_value(
            json!({"id":"local","title":"API","goal":"Define API","kind":"implementation"}),
        )
        .unwrap()
    }

    #[test]
    fn a_verifier_is_shown_the_recorded_selection() {
        let selection = Selection {
            alternatives: vec![],
            selected: "A".into(),
            rationale: "cheapest".into(),
        };
        let shown = report_with_deliverables("prose", Some(&selection));
        assert!(shown.starts_with("prose"));
        assert!(shown.contains("\"selected\": \"A\"") && shown.contains("cheapest"));
        assert_eq!(report_with_deliverables("prose", None), "prose");
    }

    #[test]
    fn completion_requires_a_summary_and_the_worker_role() {
        let result: TurnResult =
            serde_json::from_value(json!({"type":"completed","summary":"API","report":"Defined"}))
                .unwrap();
        result.validate_task(&task(), false).unwrap();
        assert!(result.validate_task(&task(), true).is_err());
        let result: TurnResult =
            serde_json::from_value(json!({"type":"completed","summary":" ","report":"Defined"}))
                .unwrap();
        assert!(result.validate_task(&task(), false).is_err());
        assert!(serde_json::from_value::<TurnResult>(json!({"text":"done"})).is_err());
    }

    #[test]
    fn a_stored_completion_with_legacy_outputs_still_loads() {
        let result: TurnResult = serde_json::from_value(
            json!({"type":"completed","summary":"API","report":"Defined","outputs":{"api":"x"}}),
        )
        .unwrap();
        assert_eq!(result.text(), "Defined");
    }

    #[test]
    fn review_must_account_for_each_selected_local_risk() {
        let result: TurnResult = serde_json::from_value(
            json!({"type":"review","report":"Reviewed","verdict":{"approved":true,"findings":[]}}),
        )
        .unwrap();
        assert!(result.validate_task(&task(), true).is_err());
    }

    #[test]
    fn a_selection_names_one_of_its_distinct_alternatives() {
        let args = json!({"selected": "A", "rationale": "Preserves contracts", "alternatives": [
            {"id": "A", "approach": "Reuse", "tradeoffs": "Less flexibility"},
            {"id": "B", "approach": "Replace", "tradeoffs": "Changes contracts"}]});
        let TurnResult::Completed {
            summary, selection, ..
        } = task_tool_result("select", &args).unwrap()
        else {
            panic!("a selection completes the task");
        };
        assert_eq!(summary, "Selected A: Reuse");
        assert!(selection.is_some());
        let mut unknown = args.clone();
        unknown["selected"] = json!("C");
        assert!(task_tool_result("select", &unknown).is_err());
        let mut duplicate = args.clone();
        duplicate["alternatives"][1]["id"] = json!("A");
        assert!(task_tool_result("select", &duplicate).is_err());
        let mut single = args;
        single["alternatives"].as_array_mut().unwrap().pop();
        assert!(task_tool_result("select", &single).is_err());
    }

    #[test]
    fn a_stored_selection_of_the_longer_form_still_loads() {
        let stored = json!({"criteria": ["c"], "alternatives": [
            {"id": "A", "approach": "a", "evaluations": ["e"], "eligible": true, "tradeoffs": "t"}],
            "selected": "A", "rationale": "r", "reconsider_when": "w"});
        let selection: Selection = serde_json::from_value(stored).unwrap();
        assert_eq!(selection.chosen().unwrap().approach, "a");
    }

    #[test]
    fn the_runtime_derives_the_verdict_and_stable_finding_ids() {
        let ok = json!({"status": "ok", "note": "checked"});
        let args = json!({"report": "Reviewed", "checks": {"goal": ok}, "findings": []});
        let TurnResult::Review { verdict, .. } = task_tool_result("review", &args).unwrap() else {
            panic!("a review");
        };
        assert!(verdict.approved);
        assert!(verdict.resolved.is_empty());
        let mut fixed = args.clone();
        fixed["resolved"] = json!(["goal:1234abcd"]);
        let TurnResult::Review { verdict, .. } = task_tool_result("review", &fixed).unwrap() else {
            panic!("a review");
        };
        assert_eq!(verdict.resolved, ["goal:1234abcd"]);
        let finding =
            json!({"check": "goal", "severity": "medium", "issue": "wrong", "where": "src/a.rs:1"});
        let mut blocked = args.clone();
        blocked["findings"] = json!([finding.clone(), finding]);
        let TurnResult::Review { verdict, .. } = task_tool_result("review", &blocked).unwrap()
        else {
            panic!("a review");
        };
        assert!(!verdict.approved);
        let ids: Vec<_> = verdict.findings.iter().map(|f| f["id"].clone()).collect();
        assert!(ids[0].as_str().unwrap().starts_with("goal:"));
        assert_ne!(ids[0], ids[1]);
        let again = task_tool_result("review", &blocked).unwrap();
        let TurnResult::Review { verdict: same, .. } = again else {
            panic!("a review")
        };
        assert_eq!(
            same.findings[0]["id"], ids[0],
            "the same finding keeps its id"
        );
        let mut open = args;
        open["checks"]["goal"] = json!({"status": "blocked", "note": "no data"});
        let TurnResult::Review { verdict, .. } = task_tool_result("review", &open).unwrap() else {
            panic!("a review");
        };
        assert!(!verdict.approved, "an unsettled check does not approve");
    }

    #[test]
    fn the_runtime_names_a_question_by_its_content() {
        let args = json!({"context": "Scope decides the work.", "question": "Which scope?",
            "options": [{"label": "Small", "description": "Sooner"}, {"label": "Full"}]});
        let TurnResult::Question { question } = tool_result("ask_user", &args).unwrap() else {
            panic!("a question");
        };
        let TurnResult::Question { question: again } = tool_result("ask_user", &args).unwrap()
        else {
            panic!("a question");
        };
        assert_eq!(question.id, again.id, "the same question keeps its id");
        let mut other = args.clone();
        other["question"] = json!("Which other scope?");
        let TurnResult::Question { question: other } = tool_result("ask_user", &other).unwrap()
        else {
            panic!("a question");
        };
        assert_ne!(question.id, other.id);
        let mut single = args;
        single["options"].as_array_mut().unwrap().pop();
        assert!(tool_result("ask_user", &single).is_err());
    }

    #[test]
    fn a_plain_review_is_approved_unless_a_finding_blocks() {
        let low = json!({"severity": "low", "issue": "naming", "where": "t2"});
        let args = json!({"report": "Checked the plan", "findings": [low]});
        let TurnResult::Review { verdict, .. } = tool_result("review", &args).unwrap() else {
            panic!("a review");
        };
        assert!(verdict.approved && verdict.coverage.is_empty());
        assert!(
            verdict.findings[0]["id"]
                .as_str()
                .unwrap()
                .starts_with("finding:")
        );
        let mut blocked = args;
        blocked["findings"][0]["severity"] = json!("high");
        let encoded = tool_result("review", &blocked).unwrap().encode().unwrap();
        assert!(!TurnResult::review_verdict(&encoded).unwrap().approved);
        assert!(TurnResult::review_verdict("Plan review: approved.").is_none());
    }

    #[test]
    fn an_answer_carries_the_comments_it_settled_and_a_plain_one_stays_text() {
        let plain = tool_result("finish_turn", &json!({"text": "Done"})).unwrap();
        assert_eq!(plain.encode().unwrap(), "Done");
        let settled = tool_result(
            "finish_turn",
            &json!({"text": "Revised", "addressed": [{"id": 3, "how": "split t1"}]}),
        )
        .unwrap();
        let decoded = TurnResult::decode(&settled.encode().unwrap());
        assert_eq!(decoded.addressed(), Some(&[3][..]));
        assert_eq!(decoded.text(), "Revised");
        assert!(tool_result("finish_turn", &json!({"text": " "})).is_err());
    }
}
