use super::*;

/// Plan and revision reviews that rejected this many times in a row stop
/// blocking on findings that are only medium or low: they travel with the plan
/// as notes. Without this, a reviewer finding something new each round keeps a
/// sound plan from ever running.
pub const TOLERATED_PLAN_REJECTIONS: usize = 2;

/// Whether every finding of a rejection is a structured medium or low one. An
/// unstructured rejection or a high or critical finding is never minor.
pub fn only_minor_findings(verdict: &Verdict) -> bool {
    !verdict.approved
        && !verdict.findings.is_empty()
        && verdict.findings.iter().all(|finding| {
            matches!(
                finding["severity"]
                    .as_str()
                    .map(str::to_ascii_lowercase)
                    .as_deref(),
                Some("medium" | "low")
            )
        })
}

/// The id of the obligation to meet the task's own goal.
pub const GOAL_CHECK: &str = "goal";

/// What a task's verifier must account for in its verdict, with ids: the
/// task's goal (`goal`), each acceptance criterion assigned to it (by its
/// ledger id) and its kind's share of the risk catalog (`R001`…). Nothing
/// else: no generated risks and no checks the runtime adds on its own.
pub fn obligations(task: &TaskContract) -> Vec<(String, String)> {
    std::iter::once((GOAL_CHECK.to_owned(), format!("The goal is met: {}", task.goal)))
        .chain(task.criterion_ids.iter().map(|id|(id.clone(),format!("Evaluate assigned criterion {id} against this task's local deliverable; for options assess the proposed approach, not future implementation"))))
        .chain(crate::lean::risks::for_kind(task.kind))
        .collect()
}

pub(super) fn obligations_text(obligations: &[(String, String)]) -> String {
    obligations
        .iter()
        .map(|(id, text)| format!("- {id}: {text}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The obligations a verdict does not account for: missing, with another
/// status than ok, issue, blocked or n/a, or without a note on what was
/// checked or why it does not apply. One reading with the card's counters.
pub fn unaccounted<'a>(
    verdict: &Verdict,
    obligations: &'a [(String, String)],
) -> Vec<&'a (String, String)> {
    obligations
        .iter()
        .filter(|(id, _)| {
            let entry = verdict
                .coverage
                .get(id.as_str())
                .cloned()
                .unwrap_or_default();
            counted_status(&entry) == "missing"
        })
        .collect()
}

/// A verdict counts only with every obligation accounted for; one marked
/// `issue` or `blocked` (it could not be settled; missing data is not `ok`)
/// rejects it, and its note reaches the worker as a finding. For a read-only
/// task (`read_only`) an `issue` on a risk (`R…`) describes the state of the
/// code the report analyses, which the worker cannot change; it stays in the
/// verdict's coverage and does not reject the report.
pub fn settle(verdict: &mut Verdict, obligations: &[(String, String)], read_only: bool) {
    let missing = unaccounted(verdict, obligations);
    append_missing_finding(verdict, &missing);
    append_unsettled_findings(verdict, obligations, read_only);
}

fn append_missing_finding(verdict: &mut Verdict, missing: &[&(String, String)]) {
    if missing.is_empty() {
        return;
    }
    verdict.approved = false;
    verdict.findings.push(serde_json::json!({
        "severity": "high",
        "issue": format!(
            "the verifier's verdict does not account for {}",
            missing
                .iter()
                .map(|(id, text)| format!("{id} ({text})"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        "where": "verdict",
    }));
}

fn append_unsettled_findings(
    verdict: &mut Verdict,
    obligations: &[(String, String)],
    _read_only: bool,
) {
    for (id, text) in obligations {
        let entry = &verdict
            .coverage
            .get(id.as_str())
            .cloned()
            .unwrap_or_default();
        let status = entry["status"].as_str().map(str::to_ascii_lowercase);
        let unsettled = match status.as_deref() {
            Some("issue") => "",
            Some("blocked") => "could not be settled: ",
            _ => continue,
        };
        verdict.approved = false;
        if verdict.findings.iter().any(|f| {
            f["check"] == *id
                && matches!(f["severity"].as_str(), Some("critical" | "high" | "medium"))
        }) {
            continue;
        }
        verdict.findings.push(serde_json::json!({
            "id": format!("check:{id}"),
            "check": id,
            "severity": "medium",
            "issue": format!(
                "{unsettled}{text}: {}",
                entry["note"].as_str().unwrap_or_default()
            ),
            "where": id,
        }));
    }
}

/// One progress line of a verifier: it started on an obligation (`status`
/// is `None`) or settled it.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckUpdate {
    pub id: String,
    pub status: Option<String>,
    pub note: String,
}

/// The user's comments on checks, for the verifier: `(check id, text)`.
pub fn check_comments_text(comments: &[(String, String)]) -> String {
    if comments.is_empty() {
        return String::new();
    }
    let list: Vec<String> = comments
        .iter()
        .map(|(id, text)| format!("- on {id}: {text}"))
        .collect();
    format!(
        "The user commented on your checks. Take each into account and answer it in \
         that check's evidence:\n{}\n\n",
        list.join("\n")
    )
}
