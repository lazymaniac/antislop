use super::*;

pub(super) fn approve(
    core: &Arc<Core>,
    work: &mut RecoveryWork,
    review: &ManualReview,
    fixed: &[String],
) -> Result<()> {
    work.task.result = Some(review.snapshot.clone());
    work.task.status = status::APPROVED.into();
    let (outcome, commit) = commit_manual_fixes(work, fixed)?;
    let notice = approval_notice(&review.snapshot, commit.as_deref());
    land(
        &mut work.task,
        outcome,
        "recovery",
        &notice,
        commit.as_deref(),
        None,
        fixed,
    );
    note(&mut work.task, notice.clone());
    persist_manual(core, work, &notice, &review.snapshot, None)?;
    core.queue_worktree_cleanup(&work.task.id);
    core.unblock_dependent_tasks(&work.req.conversation, &work.req.plan, &work.task.id)?;
    core.maybe_continue_plan(&work.req.conversation, &work.req.plan)?;
    work.notice = Some(notice);
    Ok(())
}

fn commit_manual_fixes(work: &RecoveryWork, fixed: &[String]) -> Result<(Outcome, Option<String>)> {
    if work.config.policy.local_commits
        && work.spec.delivery != Some(tasks::DeliveryMode::Untracked)
        && !fixed.is_empty()
    {
        Ok((
            Outcome::Committed,
            Some(git::commit_paths(&work.project, fixed, &work.spec.title)?),
        ))
    } else {
        Ok((Outcome::Applied, None))
    }
}

fn approval_notice(snapshot: &str, commit: Option<&str>) -> String {
    match commit {
        Some(commit) => format!("manual fixes reviewed and committed as {commit}"),
        None => format!("manual fixes reviewed and applied on snapshot {snapshot}"),
    }
}

pub(super) fn reject(
    core: &Core,
    work: &mut RecoveryWork,
    review: &ManualReview,
    fixed: &[String],
) -> Result<()> {
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

fn persist_manual(
    core: &Core,
    work: &RecoveryWork,
    notice: &str,
    snapshot: &str,
    findings: Option<&str>,
) -> Result<()> {
    core.store.update_task(&work.task)?;
    core.announce_task(&work.req.conversation, &work.task, findings);
    manual_event(core, work, notice, snapshot, findings)?;
    Ok(())
}

fn manual_event(
    core: &Core,
    work: &RecoveryWork,
    notice: &str,
    snapshot: &str,
    findings: Option<&str>,
) -> Result<()> {
    let mut payload = json!({ "actor": work.actor, "task": work.task.id,
        "action": tasks::action::REVIEW_MANUAL_SNAPSHOT, "status": work.task.status,
        "detail": tasks::bound(notice, tasks::DETAIL_LIMIT) });
    if findings.is_some() {
        payload["findings"] = json!(work.task.verdict.as_ref().map(|v| &v["findings"]));
    } else {
        payload["result"] = json!(snapshot);
    }
    super::super::events::append(core, work, &payload)
}
