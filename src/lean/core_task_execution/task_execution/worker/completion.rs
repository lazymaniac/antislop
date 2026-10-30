use super::super::super::*;

pub(in crate::lean) fn complete_report(
    core: &Core,
    conversation: &str,
    task: &mut Task,
    spec: &TaskContract,
    worktree: Option<&Path>,
    raw: String,
) -> Result<Option<(String, (String, bool))>> {
    task.report = Some(raw.clone());
    core.store.update_task(task)?;
    let result: turn_result::TurnResult = serde_json::from_str(&raw)?;
    finish_report(core, conversation, task, spec, worktree, result)
}

fn finish_report(
    core: &Core,
    conversation: &str,
    task: &mut Task,
    spec: &TaskContract,
    worktree: Option<&Path>,
    result: turn_result::TurnResult,
) -> Result<Option<(String, (String, bool))>> {
    match result {
        turn_result::TurnResult::Completed {
            report,
            summary,
            selection,
        } => Ok(Some((
            turn_result::report_with_deliverables(&report, selection.as_ref()),
            tasks::bound_cut(&summary, tasks::EFFECT_LIMIT),
        ))),
        turn_result::TurnResult::NeedsInput {
            request,
            checkpoint,
        } => {
            save_checkpoint(task, spec, worktree)?;
            task.report = Some(checkpoint);
            core.wait_for_question(conversation, task, &request, false)?;
            Ok(None)
        }
        _ => Err(Error::Invalid("worker did not submit a completion".into())),
    }
}

fn save_checkpoint(task: &mut Task, spec: &TaskContract, worktree: Option<&Path>) -> Result<()> {
    if let Some(path) = worktree {
        task.result = Some(git::commit_all(
            path,
            &format!("{} checkpoint", spec.title),
        )?);
    }
    Ok(())
}

pub(in crate::lean) fn commit_round(
    task: &mut Task,
    spec: &TaskContract,
    worktree: Option<&Path>,
    round: u32,
    report: &str,
    summary: &str,
) -> Result<Option<String>> {
    task.report = Some(report.to_owned());
    let (Some(path), Some(base)) = (worktree, task.base.as_ref()) else {
        return Ok(None);
    };
    let result = git::commit_all(path, &format!("{} (round {round})", spec.title))?;
    task.result = Some(result.clone());
    let stat = git::diff_stat(path, base, &result)?;
    let files = git::changed_files(path, base, &result)?;
    let _effect = tasks::bound_cut(summary, tasks::EFFECT_LIMIT);
    let _ = files;
    Ok(Some(tasks::changes_text(base, &result, &stat)))
}
