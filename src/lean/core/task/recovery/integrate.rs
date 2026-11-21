use super::*;

pub(super) fn apply(core: &Arc<Core>, work: &mut RecoveryWork) -> Result<()> {
    require(
        work.task
            .verdict
            .as_ref()
            .is_some_and(|v| v["approved"] == true),
        "independent review must approve the current result before integration",
    )?;
    if manual_snapshot_current(work) {
        return integrate_manual_snapshot(core, work);
    }
    let (base, change) = integration_commits(work)?;
    let integration_project = integration_project(core, work)?;
    let all_changed = git::changed_files(&integration_project, &base, &change)?;
    validate_consent(work, &all_changed)?;
    if work.spec.delivery == Some(tasks::DeliveryMode::Untracked)
        && let Some(consent) = &work.req.consent_files
    {
        require(
            consent.iter().collect::<std::collections::BTreeSet<_>>()
                == all_changed
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>(),
            "untracked artifact recovery requires consent for every changed output",
        )?;
    }
    let (notice, card) = integrate_changes(core, work, &integration_project, &base, &change)?;
    work.integrated_files = work.req.consent_files.take().unwrap_or(all_changed.clone());
    mark_remaining_changes(work, &integration_project, &change, &all_changed)?;
    finish_integration(core, work, &notice, card)?;
    core.queue_worktree_cleanup(&work.task.id);
    work.notice = Some(notice);
    Ok(())
}

fn manual_snapshot_current(work: &RecoveryWork) -> bool {
    let Some(result) = work.task.result.as_deref() else {
        return false;
    };
    work.task.feedback.as_ref().is_some_and(|feedback| {
        feedback["manual_snapshot"]["result"] == result
            && feedback["manual_snapshot"]["round"] == work.task.round
            && work.task.verdict.as_ref().is_some_and(|verdict| {
                verdict["result"] == result && verdict["round"] == work.task.round
            })
    })
}

fn integrate_manual_snapshot(core: &Arc<Core>, work: &mut RecoveryWork) -> Result<()> {
    let (base, result) = integration_commits(work)?;
    let all_changed = git::changed_files(&work.project, &base, &result)?;
    let consent = work
        .req
        .consent_files
        .as_deref()
        .ok_or_else(|| Error::Invalid("explicit file consent is required".into()))?;
    require(!consent.is_empty(), "choose at least one file to integrate")?;
    require(
        consent
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            == consent.len(),
        "file consent contains duplicate paths",
    )?;
    validate_consent(work, &all_changed)?;
    work.spec.validate_change_paths(&all_changed)?;
    if work.spec.delivery == Some(tasks::DeliveryMode::Untracked) {
        require(
            consent.iter().collect::<std::collections::BTreeSet<_>>()
                == all_changed
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>(),
            "untracked artifact recovery requires consent for every changed output",
        )?;
    }

    // A manual recovery snapshot describes edits already in the project's
    // working tree. Confirm each authorized path still has the reviewed tree
    // entry, then commit those paths in place; applying the snapshot as a Git
    // change would replay the same edits and could overwrite newer user work.
    let mut completed = work
        .task
        .feedback
        .as_ref()
        .and_then(|feedback| feedback["manual_snapshot"]["integrated_files"].as_array())
        .map(|files| {
            files
                .iter()
                .filter_map(|file| file.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for file in consent {
        if !completed.contains(file) {
            completed.push(file.clone());
        }
    }
    let current = git::snapshot(&work.project, git::CommitPurpose::WorkspaceSnapshot)?;
    require(
        git::matching_paths(&work.project, &result, &current, &completed)?.len() == completed.len(),
        "manual recovery files changed after review; review the current working tree again",
    )?;
    let (notice, card) = if work.config.policy.local_commits && !consent.is_empty() {
        let commit = git::commit_paths(&work.project, consent, git::CommitPurpose::ManualRecovery)?;
        (
            format!("integrated selected manual files as {commit}"),
            (Outcome::Committed, Some(commit)),
        )
    } else {
        (
            "selected manual files remain applied in the project working tree".into(),
            (Outcome::Applied, None),
        )
    };
    if let Some(feedback) = work.task.feedback.as_mut()
        && let Some(manual) = feedback["manual_snapshot"].as_object_mut()
    {
        manual.insert("integrated_files".into(), json!(completed));
    }
    work.integrated_files = consent.to_vec();
    if all_changed.iter().all(|file| completed.contains(file)) {
        work.task.status = status::APPROVED.into();
    } else {
        work.task.status = status::NEEDS_USER.into();
    }
    finish_integration(core, work, &notice, card)?;
    if work.task.status == status::APPROVED {
        core.queue_worktree_cleanup(&work.task.id);
    }
    work.notice = Some(notice);
    Ok(())
}

fn integration_project(core: &Core, work: &RecoveryWork) -> Result<PathBuf> {
    if work.spec.delivery == Some(tasks::DeliveryMode::Untracked) {
        Ok(work.project.clone())
    } else {
        core.session_worktree(&work.req.conversation, &work.project)
    }
}

fn finish_integration(
    core: &Arc<Core>,
    work: &mut RecoveryWork,
    notice: &str,
    card: (Outcome, Option<String>),
) -> Result<()> {
    let graph = core.locks.graph();
    let integration = core.locks.integration();
    super::ensure_current(core, work)?;
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
    drop(integration);
    drop(graph);
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
        validate_consent_file(file, changed)?;
    }
    Ok(())
}

fn validate_consent_file(file: &str, changed: &[String]) -> Result<()> {
    require(
        changed.contains(&file.to_string()),
        &format!("consent file '{file}' is not in changed files"),
    )?;
    Ok(())
}

fn integrate_changes(
    core: &Core,
    work: &mut RecoveryWork,
    integration_project: &Path,
    base: &str,
    change: &str,
) -> Result<(String, (Outcome, Option<String>))> {
    work.spec
        .validate_change_paths(&git::changed_files(integration_project, base, change)?)?;
    if work.spec.delivery == Some(tasks::DeliveryMode::Untracked) {
        let result = core.deliver_artifact_change(
            &work.task,
            &work.spec,
            &work.project,
            base,
            change,
            &work.cancel,
        )?;
        return integration_result(work, result);
    }
    let result = git::integrate_scoped(
        integration_project,
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

fn mark_remaining_changes(
    work: &mut RecoveryWork,
    integration_project: &Path,
    change: &str,
    all: &[String],
) -> Result<()> {
    if work.integrated_files.len() >= all.len() {
        return Ok(());
    }
    let snapshot = git::snapshot(integration_project, git::CommitPurpose::WorkspaceSnapshot)?;
    if git::matching_paths(integration_project, &snapshot, change, all)?.len() != all.len() {
        work.task.status = status::NEEDS_USER.into();
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
