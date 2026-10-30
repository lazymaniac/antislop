use super::*;

pub(super) fn apply(core: &Arc<Core>, work: &mut RecoveryWork) -> Result<()> {
    require(
        work.task
            .verdict
            .as_ref()
            .is_some_and(|v| v["approved"] == true),
        "independent review must approve the current result before integration",
    )?;
    let (base, change) = integration_commits(work)?;
    let all_changed = git::changed_files(&work.project, &base, &change)?;
    validate_consent(work, &all_changed)?;
    let (notice, card) = integrate_changes(work, &base, &change)?;
    work.integrated_files = work.req.consent_files.take().unwrap_or(all_changed.clone());
    mark_remaining_changes(work, &change, &all_changed)?;
    cleanup_approved(core, work)?;
    finish_integration(core, work, &notice, card)?;
    work.notice = Some(notice);
    Ok(())
}

fn finish_integration(
    core: &Arc<Core>,
    work: &mut RecoveryWork,
    notice: &str,
    card: (Outcome, Option<String>),
) -> Result<()> {
    land(
        &mut work.task,
        card.0,
        "recovery",
        notice,
        card.1.as_deref(),
        None,
        &work.integrated_files,
    );
    note(&mut work.task, notice);
    persist_integration(core, work, notice)?;
    if work.task.status == status::APPROVED {
        core.unblock_dependent_tasks(&work.req.conversation, &work.req.plan, &work.task.id)?;
    }
    core.maybe_continue_plan(&work.req.conversation, &work.req.plan)?;
    Ok(())
}

fn integration_commits(work: &RecoveryWork) -> Result<(String, String)> {
    let change = work
        .task
        .result
        .clone()
        .ok_or_else(|| Error::Invalid("task has no result commit to integrate".into()))?;
    let base = work
        .task
        .base
        .clone()
        .ok_or_else(|| Error::Invalid("task has no base commit".into()))?;
    Ok((base, change))
}

fn validate_consent(work: &RecoveryWork, changed: &[String]) -> Result<()> {
    let Some(consent) = &work.req.consent_files else {
        return Ok(());
    };
    for file in consent {
        validate_consent_file(work, file, changed)?;
    }
    Ok(())
}

fn validate_consent_file(work: &RecoveryWork, file: &str, changed: &[String]) -> Result<()> {
    require(
        changed.contains(&file.to_string()),
        &format!("consent file '{file}' is not in changed files"),
    )?;
    let in_scope = work.spec.scope.is_empty()
        || work
            .spec
            .scope
            .iter()
            .any(|s| s == "." || s.is_empty() || file.starts_with(s) || s == file);
    require(
        in_scope,
        &format!("consent file '{file}' is outside task scope"),
    )
}

fn integrate_changes(
    work: &mut RecoveryWork,
    base: &str,
    change: &str,
) -> Result<(String, (Outcome, Option<String>))> {
    let result = git::integrate_scoped(
        &work.project,
        base,
        change,
        work.req.consent_files.as_deref(),
        work.config.policy.local_commits,
    )?;
    integration_result(work, result)
}

fn integration_result(
    work: &mut RecoveryWork,
    result: git::Integration,
) -> Result<(String, (Outcome, Option<String>))> {
    match result {
        git::Integration::Committed(commit) => {
            work.task.status = status::APPROVED.into();
            Ok((
                format!("committed as {commit}"),
                (Outcome::Committed, Some(commit)),
            ))
        }
        git::Integration::Applied => {
            work.task.status = status::APPROVED.into();
            Ok((
                "applied to the working tree, not committed".into(),
                (Outcome::Applied, None),
            ))
        }
        git::Integration::Conflict(files) => Err(Error::Invalid(format!(
            "conflicts remain in {}",
            files.join(", ")
        ))),
        git::Integration::UserEdits(files) => Err(Error::Invalid(format!(
            "uncommitted edits remain in {}",
            files.join(", ")
        ))),
    }
}

fn mark_remaining_changes(work: &mut RecoveryWork, change: &str, all: &[String]) -> Result<()> {
    if work.integrated_files.len() >= all.len() {
        return Ok(());
    }
    let snapshot = git::snapshot(&work.project, "Check remaining task changes")?;
    if git::matching_paths(&work.project, &snapshot, change, all)?.len() != all.len() {
        work.task.status = status::NEEDS_USER.into();
    }
    Ok(())
}

fn cleanup_approved(core: &Core, work: &RecoveryWork) -> Result<()> {
    if work.task.status != status::APPROVED {
        return Ok(());
    }
    let path = worktree_path(&core.state, &work.task)?;
    if path.exists() {
        git::remove_worktree(
            &work.project,
            &path,
            &worktree_branch(&core.state, &work.task)?,
        )?;
    }
    Ok(())
}

fn persist_integration(core: &Core, work: &RecoveryWork, notice: &str) -> Result<()> {
    core.store.update_task(&work.task)?;
    core.announce_task(&work.req.conversation, &work.task, None);
    integration_event(core, work, notice)?;
    Ok(())
}

fn integration_event(core: &Core, work: &RecoveryWork, notice: &str) -> Result<()> {
    let payload = json!({ "actor": work.actor, "task": work.task.id,
        "action": tasks::action::INTEGRATE_WITH_CONSENT, "status": work.task.status,
        "files": event_files(&work.integrated_files),
        "files_more": work.integrated_files.len().saturating_sub(tasks::FILE_LIMIT),
        "detail": tasks::bound(notice, tasks::DETAIL_LIMIT) });
    events::append(core, work, &payload)
}
