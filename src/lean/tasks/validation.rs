use super::*;

pub fn parse_verdict(answer: &str) -> Verdict {
    let mut verdict = fenced(answer, "verdict")
        .and_then(|(_, body)| serde_json::from_str::<Verdict>(body).ok())
        .unwrap_or_else(|| Verdict {
            approved: false,
            findings: vec![Value::String(answer.trim().to_owned())],
            coverage: serde_json::Map::new(),
        });
    let blocking = verdict.findings.iter().any(|finding| {
        matches!(
            finding["severity"]
                .as_str()
                .map(str::to_ascii_lowercase)
                .as_deref(),
            Some("high" | "medium" | "critical")
        )
    });
    verdict.approved &= !blocking;
    verdict
}

/// What every verification checks whatever the kind of work (design §7.6,
/// V02 and V04–V07; the runtime pins the basis and keeps verifiers
/// independent, V01 and V08, and the coverage itself is V03).
pub const COMMON_CHECKS: &str = "every requirement of the goal accounted for, \
     with each qualifier, exception and prohibition; the key claims \
     checked against actual files and recorded worker evidence without executing commands; a \
     counterexample or a failing premise looked for, not only the expected \
     path; nothing beyond the agreed scope and no hidden change of it; \
     earlier findings resolved on the current version, with evidence; \
     the touched code is left in a better, cleaner state than it was found";

/// What a task's verifier must account for in its verdict, with ids: the
/// risks its task names (`R1`…), the common checks (`V1`…), its kind's
/// checklist (`C1`…) and the share of the design's risk catalog (`R001`…)
/// that belongs to its kind.
pub fn obligations(task: &TaskSpec) -> Vec<(String, String)> {
    let risks = task
        .verifier
        .risks
        .iter()
        .enumerate()
        .map(|(index, risk)| (format!("R{}", index + 1), risk.clone()));
    let numbered = |prefix: &'static str, list: &'static str| {
        list.split("; ")
            .enumerate()
            .map(move |(index, check)| (format!("{prefix}{}", index + 1), check.to_owned()))
    };
    task.resolves
        .iter()
        .map(|blocked| ("B1".into(), crate::lean::blocker::obligation(blocked)))
        .chain(risks)
        .chain(numbered("V", COMMON_CHECKS))
        .chain(numbered("C", checklist(task.kind)))
        .chain(crate::lean::risks::for_kind(task.kind))
        .collect()
}

pub(super) fn obligations_text(obligations: &[(String, String)]) -> String {
    obligations
        .iter()
        .map(|(id, text)| format!("- {id}: Assess from files and recorded evidence only (do not execute or edit): {text}"))
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
    read_only: bool,
) {
    for (id, text) in obligations {
        let entry = &verdict
            .coverage
            .get(id.as_str())
            .cloned()
            .unwrap_or_default();
        let status = entry["status"].as_str().map(str::to_ascii_lowercase);
        let unsettled = match status.as_deref() {
            Some("issue") if read_only && id.starts_with('R') => continue,
            Some("issue") => "",
            Some("blocked") => "could not be settled: ",
            _ => continue,
        };
        verdict.approved = false;
        verdict.findings.push(serde_json::json!({
            "severity": "medium",
            "issue": format!(
                "{unsettled}{text}: {}",
                entry["note"].as_str().unwrap_or_default()
            ),
            "where": id,
        }));
    }
}

pub fn coverage_followup(missing: &[&(String, String)]) -> String {
    let missing: Vec<(String, String)> = missing.iter().map(|item| (*item).clone()).collect();
    format!(
        "Your verdict does not account for these obligations:\n{}\n\n\
         Give your complete verdict again, with every obligation in \
         \"coverage\". {VERDICT_FORMAT} {COVERAGE_FORMAT}",
        obligations_text(&missing)
    )
}

/// One progress line of a verifier: it started on an obligation (`status`
/// is `None`) or settled it.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckUpdate {
    pub id: String,
    pub status: Option<String>,
    pub note: String,
}

/// Reads `CHECKING <id>` and `CHECK <id> <status>: <evidence>`, however the
/// agent decorated the line (list marks, bold, backticks).
pub fn parse_progress(line: &str) -> Option<CheckUpdate> {
    let line = line.trim_matches(|c: char| c.is_whitespace() || "-*•`>_".contains(c));
    let (word, rest) = line.split_once(char::is_whitespace)?;
    let word = word.trim_matches(|c: char| !c.is_alphanumeric());
    let rest = rest.trim_start();
    parse_progress_command(word, rest)
}

fn parse_progress_command(word: &str, rest: &str) -> Option<CheckUpdate> {
    let (id, rest) = match rest.split_once(char::is_whitespace) {
        Some((id, rest)) => (id, rest.trim()),
        None => (rest, ""),
    };
    let id = id.trim_matches(|c: char| !c.is_alphanumeric());
    if !valid_check_id(id) {
        return None;
    }
    match word {
        "CHECKING" => Some(CheckUpdate {
            id: id.to_owned(),
            status: None,
            note: String::new(),
        }),
        "CHECK" => {
            let (status, note) = rest.split_once(':').unwrap_or((rest, ""));
            let status = status.trim().trim_matches('*').to_ascii_lowercase();
            matches!(status.as_str(), "ok" | "issue" | "blocked" | "n/a").then(|| CheckUpdate {
                id: id.to_owned(),
                status: Some(status),
                note: bound(note.trim().trim_matches('*').trim(), DETAIL_LIMIT),
            })
        }
        _ => None,
    }
}

fn valid_check_id(id: &str) -> bool {
    let mut chars = id.chars();
    id.len() >= 2
        && matches!(chars.next(), Some('R' | 'V' | 'C'))
        && chars.all(|ch| ch.is_ascii_digit())
}

/// Splits streamed text into lines and reads the progress lines among them.
#[derive(Debug, Default)]
pub struct Progress {
    partial: String,
}

impl Progress {
    pub fn push(&mut self, delta: &str) -> Vec<CheckUpdate> {
        self.partial.push_str(delta);
        let mut updates = Vec::new();
        while let Some(end) = self.partial.find('\n') {
            let line: String = self.partial.drain(..=end).collect();
            updates.extend(parse_progress(&line));
        }
        // A line that never ends must not grow without bound.
        if self.partial.len() > 4 * DETAIL_LIMIT {
            self.partial.clear();
        }
        updates
    }

    /// The last line, which may have no newline after it.
    pub fn finish(&mut self) -> Vec<CheckUpdate> {
        parse_progress(&std::mem::take(&mut self.partial))
            .into_iter()
            .collect()
    }
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
