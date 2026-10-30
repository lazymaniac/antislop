use super::*;

pub(super) fn results(details: &mut Details, view: &View, records: &mut Vec<Rc<DetailRecord>>) {
    let task = details
        .task_id()
        .and_then(|id| view.tasks.iter().find(|task| task.id == id));
    match (&details.task, task, details.key.starts_with("verifier:")) {
        (_, None, _) => records.push(record(vec![(dim(), "Task workers and verifiers have results; plan verifier and auditor results are in Overview. [no historical result data]".into())], " ")),
        (None, Some(_), _) => records.push(record(vec![(dim(), "Loading…".into())], " ")),
        (Some(stored), Some(_), true) => records.push(verifier_result(&stored["verdict"])),
        (Some(stored), Some(_), false) => worker_results(stored, records),
    }
}

fn verifier_result(verdict: &Value) -> Rc<DetailRecord> {
    if verdict.is_null() {
        return record(vec![(dim(), "Verifier has not evaluated the task yet (verdict will appear after verification). [no historical verdict data]".into())], " ");
    }
    let mut lines = crate::lean::cli::review_lines("verdict", verdict)
        .into_iter()
        .map(plain)
        .collect::<Vec<_>>();
    append_findings(&mut lines, verdict);
    record(lines, " ")
}

fn append_findings(lines: &mut Vec<(Style, String)>, verdict: &Value) {
    match verdict["findings"].as_array() {
        Some(findings) if findings.is_empty() => lines.push(plain(
            "Findings: no issues (verifier approved the task without notes).".into(),
        )),
        Some(findings) => {
            lines.push(label("Verifier findings"));
            lines.extend(findings.iter().map(finding_line));
        }
        None => lines.push((dim(), "Findings: [no historical findings data]".into())),
    }
}

fn finding_line(finding: &Value) -> (Style, String) {
    let severity = finding["severity"].as_str().unwrap_or("note");
    let issue = finding["issue"].as_str().unwrap_or_default();
    let location = finding["where"]
        .as_str()
        .map(|where_| format!(" ({where_})"))
        .unwrap_or_default();
    plain(format!("• [{severity}] {issue}{location}"))
}

fn worker_results(stored: &Value, records: &mut Vec<Rc<DetailRecord>>) {
    records.push(artifact_record(stored));
    records.push(report_record(stored));
    records.push(evidence_record(stored));
}

fn artifact_record(stored: &Value) -> Rc<DetailRecord> {
    let mut lines = vec![label("Artifacts and changes")];
    if let (Some(base), Some(result)) = (stored["base"].as_str(), stored["result"].as_str()) {
        lines.push(plain(format!(
            "Git changes: {}..{}",
            short_hash(base),
            short_hash(result)
        )));
    } else {
        lines.push((
            dim(),
            "Artifacts: no recorded git changes [historical data unavailable]".into(),
        ));
    }
    record(lines, " ")
}

fn short_hash(hash: &str) -> &str {
    &hash[..hash.len().min(10)]
}

fn report_record(stored: &Value) -> Rc<DetailRecord> {
    let report = stored["report"].as_str().map_or_else(
        || (dim(), "Worker has not submitted a report yet (work in progress or waiting to start) [no historical report data].".into()),
        |report| plain(report.to_owned()),
    );
    record(vec![label("Report"), report], " ")
}

fn evidence_record(stored: &Value) -> Rc<DetailRecord> {
    let mut lines = vec![label("Evidence and findings")];
    match stored.get("verdict").filter(|verdict| !verdict.is_null()) {
        Some(verdict) => append_evidence_findings(&mut lines, verdict),
        None => lines.push((
            dim(),
            "Evidence: [no attached evidence in history / awaiting verification]".into(),
        )),
    }
    record(lines, " ")
}

fn append_evidence_findings(lines: &mut Vec<(Style, String)>, verdict: &Value) {
    if let Some(findings) = verdict["findings"].as_array() {
        if findings.is_empty() {
            lines.push(plain("Findings: no issues.".into()));
        } else {
            lines.extend(findings.iter().map(|finding| {
                plain(format!(
                    "• [{}] {}",
                    finding["severity"].as_str().unwrap_or("note"),
                    finding["issue"].as_str().unwrap_or_default()
                ))
            }));
        }
    }
}

fn plain(text: String) -> (Style, String) {
    (Style::default(), text)
}
fn label(text: &str) -> (Style, String) {
    (
        Style::default().add_modifier(Modifier::BOLD),
        text.to_owned(),
    )
}
