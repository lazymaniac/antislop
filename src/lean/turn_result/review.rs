use super::types::{Selection, TurnResult};
use crate::lean::tasks::Verdict;
use crate::{Result, require};
use serde::Deserialize;
use serde_json;

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

pub fn task_tool_result(tool: &str, args: &serde_json::Value) -> Result<TurnResult> {
    match tool {
        "complete" => Ok(TurnResult::Completed {
            summary: optional_text(args, "summary"),
            report: optional_text(args, "report"),
            selection: None,
        }),
        "select" => selection_tool(args),
        "review" => review(optional_text(args, "report"), args),
        other => Err(crate::Error::Invalid(format!(
            "{other} is not a task result"
        ))),
    }
}

fn optional_text(args: &serde_json::Value, field: &str) -> String {
    args[field].as_str().unwrap_or_default().to_owned()
}

fn selection_tool(args: &serde_json::Value) -> Result<TurnResult> {
    let selection = Selection {
        alternatives: serde_json::from_value(args["alternatives"].clone())
            .map_err(|error| crate::Error::Invalid(format!("alternatives: {error}")))?,
        selected: optional_text(args, "selected"),
        rationale: optional_text(args, "rationale"),
    };
    selection.validate()?;
    let chosen = selection.chosen().map_or("", |item| item.approach.as_str());
    Ok(TurnResult::Completed {
        summary: format!("Selected {}: {chosen}", selection.selected),
        report: selection.rationale.clone(),
        selection: Some(selection),
    })
}

/// A verifier's review with the verdict the runtime derives from it: approved
/// when every check is ok or n/a and no finding blocks; each finding gets an
/// id stable across rounds, from its check and location.
fn review(report: String, args: &serde_json::Value) -> Result<TurnResult> {
    let checks = review_checks(args)?;
    let findings = review_findings(args)?;
    let recorded = record_findings(&findings)?;
    let resolved = resolved_items(args)?;
    let approved = review_approved(&checks, &findings);
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

fn resolved_items(args: &serde_json::Value) -> Result<Vec<String>> {
    if args["resolved"].is_null() {
        return Ok(Vec::new());
    }
    serde_json::from_value(args["resolved"].clone())
        .map_err(|error| crate::Error::Invalid(format!("resolved: {error}")))
}

fn review_checks(args: &serde_json::Value) -> Result<serde_json::Map<String, serde_json::Value>> {
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
        validate_check(id, check)?;
    }
    Ok(checks)
}

fn validate_check(id: &str, check: &serde_json::Value) -> Result<()> {
    require(
        matches!(
            check["status"].as_str(),
            Some("ok" | "issue" | "blocked" | "n/a")
        ) && check["note"]
            .as_str()
            .is_some_and(|note| !note.trim().is_empty()),
        &format!("checks.{id}: give status (ok, issue, blocked or n/a) and a note"),
    )
}

fn review_findings(args: &serde_json::Value) -> Result<Vec<Finding>> {
    let findings: Vec<Finding> = match &args["findings"] {
        serde_json::Value::Null => Vec::new(),
        value => serde_json::from_value(value.clone())
            .map_err(|error| crate::Error::Invalid(format!("findings: {error}")))?,
    };
    Ok(findings)
}

fn record_findings(findings: &[Finding]) -> Result<Vec<serde_json::Value>> {
    let mut ids = std::collections::BTreeSet::new();
    findings
        .iter()
        .map(|finding| record_finding(finding, &mut ids))
        .collect()
}

fn record_finding(
    finding: &Finding,
    ids: &mut std::collections::BTreeSet<String>,
) -> Result<serde_json::Value> {
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
    let id = unique_finding_id(&base, ids);
    let mut entry = serde_json::json!({
        "id": id, "severity": finding.severity,
        "issue": finding.issue, "where": finding.location,
    });
    if let Some(check) = &finding.check {
        entry["check"] = check.clone().into();
    }
    Ok(entry)
}

fn unique_finding_id(base: &str, ids: &mut std::collections::BTreeSet<String>) -> String {
    let mut id = base.to_owned();
    let mut suffix = 2;
    while !ids.insert(id.clone()) {
        id = format!("{base}-{suffix}");
        suffix += 1;
    }
    id
}

fn review_approved(
    checks: &serde_json::Map<String, serde_json::Value>,
    findings: &[Finding],
) -> bool {
    checks
        .values()
        .all(|check| matches!(check["status"].as_str(), Some("ok" | "n/a")))
        && !findings
            .iter()
            .any(|finding| ["critical", "high", "medium"].contains(&finding.severity.as_str()))
}
