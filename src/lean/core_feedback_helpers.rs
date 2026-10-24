use super::*;

/// A review's outcome as an event payload.
pub(super) fn review_json(review: &std::result::Result<Verdict, String>) -> serde_json::Value {
    match review {
        Ok(verdict) => json!({"approved": verdict.approved, "findings": verdict.findings}),
        Err(error) => json!({"approved": false, "error": error}),
    }
}

pub(super) fn note(task: &mut Task, text: impl AsRef<str>) {
    let report = task.report.take().unwrap_or_default();
    task.report = Some(
        format!("{report}\n\n[recovery] {}", text.as_ref())
            .trim_start()
            .to_owned(),
    );
}

/// File names an event may carry: at most [`tasks::FILE_LIMIT`] of them,
/// the same cut the card's integration makes, so a wide change never
/// ships an unbounded list in a live event while the whole set stays
/// readable in git and in the task's detail screens.
pub(super) fn event_files(files: &[String]) -> Vec<String> {
    files.iter().take(tasks::FILE_LIMIT).cloned().collect()
}

/// Attaches one unambiguous integration outcome to the task's card feedback.
/// Feedback from another round is stale and dropped; the kept part is the
/// current round's effect and coverage. The detail keeps the raw reason,
/// bounded so no event ever carries a full report.
pub(super) fn land(
    task: &mut Task,
    outcome: Outcome,
    source: &str,
    detail: &str,
    commit: Option<&str>,
    on_branch: Option<&str>,
    files: &[String],
) {
    let mut feedback = current_feedback(task);
    feedback["integration"] = integration_record(outcome, source, detail, commit, on_branch, files);
    task.feedback = Some(feedback);
}

fn current_feedback(task: &mut Task) -> serde_json::Value {
    let round = u64::from(task.round);
    task.feedback
        .take()
        .filter(|feedback| tasks::feedback_is_current(feedback, round))
        .unwrap_or_else(|| json!({"round": task.round}))
}

fn integration_record(
    outcome: Outcome,
    source: &str,
    detail: &str,
    commit: Option<&str>,
    on_branch: Option<&str>,
    files: &[String],
) -> serde_json::Value {
    let mut integration = json!({
        "outcome": outcome.name(),
        "source": source,
        "detail": tasks::bound(detail, tasks::DETAIL_LIMIT),
    });
    add_commit(&mut integration, commit);
    add_branch(&mut integration, on_branch);
    add_files(&mut integration, files);
    integration
}

fn add_commit(integration: &mut serde_json::Value, commit: Option<&str>) {
    if let Some(commit) = commit {
        integration["commit"] = json!(commit);
    }
}

fn add_branch(integration: &mut serde_json::Value, branch: Option<&str>) {
    if let Some(branch) = branch {
        integration["branch"] = json!(branch);
    }
}

fn add_files(integration: &mut serde_json::Value, files: &[String]) {
    if files.is_empty() {
        return;
    }
    let shown = files.len().min(tasks::FILE_LIMIT);
    integration["files"] = json!(&files[..shown]);
    if files.len() > shown {
        integration["files_more"] = json!(files.len() - shown);
    }
}
