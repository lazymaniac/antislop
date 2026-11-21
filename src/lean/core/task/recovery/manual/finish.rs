use super::*;

/// Record a positive review and leave integration pending explicit consent.
pub(super) fn approve(
    core: &Arc<Core>,
    work: &mut RecoveryWork,
    review: &ManualReview,
    fixed: &[String],
) -> Result<()> {
    super::await_consent(core, work, review, fixed)
}

pub(super) fn reject(
    core: &Core,
    work: &mut RecoveryWork,
    review: &ManualReview,
    fixed: &[String],
) -> Result<()> {
    super::super::ensure_current(core, work)?;
    work.task.status = status::NEEDS_USER.into();
    let findings = tasks::findings_text(&review.verdict.findings);
    let notice = format!("manual fix review found open issues:\n{findings}");
    append_findings(&mut work.task, &findings);
    land(
        &mut work.task,
        Outcome::Committed,
        "recovery",
        &notice,
        Some(&review.snapshot),
        None,
        fixed,
    );
    persist_manual(core, work, &notice, &review.snapshot, Some(&findings))?;
    work.notice = Some(notice);
    Ok(())
}

fn append_findings(task: &mut Task, findings: &str) {
    task.report = Some(format!(
        "{}\n\n[manual fix review]\n{findings}",
        task.report.as_deref().unwrap_or("")
    ));
}
