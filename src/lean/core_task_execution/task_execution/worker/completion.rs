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
        turn_result::TurnResult::Blocked {
            report,
            resume_condition,
        } => {
            save_checkpoint(task, spec, worktree)?;
            task.report = Some(format!("{report}\n\nResume when: {resume_condition}"));
            core.stop(
                conversation,
                task,
                status::COORDINATING,
                Some(&resume_condition),
            )?;
            Ok(None)
        }
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
        task.result = Some(commit_worker_files(
            path,
            spec,
            &format!("{} checkpoint", spec.title),
            false,
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
    let result = commit_worker_files(path, spec, &format!("{} (round {round})", spec.title), true)?;
    task.result = Some(result.clone());
    let stat = git::diff_stat(path, base, &result)?;
    let files = git::changed_files(path, base, &result)?;
    let _effect = tasks::bound_cut(summary, tasks::EFFECT_LIMIT);
    spec.validate_change_paths(&files)?;
    Ok(Some(tasks::changes_text(base, &result, &stat)))
}

fn commit_worker_files(
    path: &Path,
    spec: &TaskContract,
    message: &str,
    require_all: bool,
) -> Result<String> {
    if spec.change_kind() == tasks::ChangeKind::Artifact {
        git::commit_artifacts(path, &spec.outputs, message, require_all)
    } else {
        git::commit_all(path, message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ignored_outputs_survive_checkpoint_review_delivery_and_snapshot() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        std::fs::create_dir(&project).unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "-q"])
                .current_dir(&project)
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(project.join(".gitignore"), ".antislop/\n").unwrap();
        let base = git::commit_all(&project, "base").unwrap();
        let index = std::fs::read(project.join(".git/index")).unwrap();
        let store = store::Store::open(&directory.path().join("core.db")).unwrap();
        let conversation = store
            .create_conversation(project.to_str().unwrap())
            .unwrap();
        let outputs = [
            ".antislop/continuation-plan-review.md".to_owned(),
            ".antislop/appendix.md".to_owned(),
        ];
        let spec: TaskContract = serde_json::from_value(json!({"id":"report","title":"Report","goal":"Deliver analysis","kind":"analysis","change_kind":"artifact","outputs":outputs,"delivery":"untracked"})).unwrap();
        let mut task = store
            .add_plan(&conversation.id, &[serde_json::to_value(&spec).unwrap()])
            .unwrap()
            .remove(0);
        let worktree = directory.path().join("worker");
        git::add_worktree(&project, &worktree, "antislop/report", &base).unwrap();
        task.base = Some(base.clone());
        std::fs::create_dir(worktree.join(".antislop")).unwrap();
        std::fs::write(worktree.join(&outputs[0]), "all sixteen findings").unwrap();
        std::fs::write(
            worktree.join(".antislop/runtime.db"),
            "unrelated runtime state",
        )
        .unwrap();
        save_checkpoint(&mut task, &spec, Some(&worktree)).unwrap();
        assert_eq!(
            git::changed_files(&worktree, &base, task.result.as_ref().unwrap()).unwrap(),
            [outputs[0].clone()]
        );
        let error = commit_round(&mut task, &spec, Some(&worktree), 1, "Report", "Summary")
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("artifact output missing: .antislop/appendix.md"),
            "{error}"
        );
        std::fs::write(worktree.join(&outputs[1]), "supporting evidence").unwrap();
        commit_round(&mut task, &spec, Some(&worktree), 1, "Report", "Summary").unwrap();
        let result = task.result.as_ref().unwrap();
        let mut files = git::changed_files(&worktree, &base, result).unwrap();
        files.sort();
        let mut expected = outputs.to_vec();
        expected.sort();
        assert_eq!(files, expected);
        let review = directory.path().join("review");
        git::add_worktree(&project, &review, "antislop/review", result).unwrap();
        assert_eq!(
            std::fs::read_to_string(review.join(&outputs[0])).unwrap(),
            "all sixteen findings"
        );
        assert!(!review.join(".antislop/runtime.db").exists());
        assert_eq!(
            git::deliver_untracked(&project, &base, result).unwrap(),
            git::Integration::Applied
        );
        let current =
            git::snapshot_with_artifacts(&project, &outputs, "Check delivered artifacts").unwrap();
        assert!(
            git::changed_files(&project, result, &current)
                .unwrap()
                .is_empty()
        );
        std::fs::write(project.join(&outputs[0]), "edited by user").unwrap();
        let changed =
            git::snapshot_with_artifacts(&project, &outputs, "Check changed artifact").unwrap();
        assert_eq!(
            git::changed_files(&project, result, &changed).unwrap(),
            [outputs[0].clone()]
        );
        assert_eq!(git::head(&project).unwrap(), base);
        assert_eq!(std::fs::read(project.join(".git/index")).unwrap(), index);
    }
}
