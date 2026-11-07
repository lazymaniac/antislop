use super::super::super::*;

pub(in crate::lean) struct WorkerCompletion {
    pub report: String,
    pub effect: (String, bool),
    pub commit_message: String,
}

pub(in crate::lean) fn complete_report(
    core: &Core,
    conversation: &str,
    task: &mut Task,
    spec: &TaskContract,
    worktree: Option<&Path>,
    raw: String,
) -> Result<Option<WorkerCompletion>> {
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
) -> Result<Option<WorkerCompletion>> {
    match result {
        turn_result::TurnResult::Completed {
            report,
            summary,
            commit_message,
            selection,
        } => Ok(Some(WorkerCompletion {
            report: turn_result::report_with_deliverables(&report, selection.as_ref()),
            effect: tasks::bound_cut(&summary, tasks::EFFECT_LIMIT),
            commit_message,
        })),
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
            if spec.change_kind() == tasks::ChangeKind::Artifact {
                git::CommitPurpose::ArtifactCheckpoint
            } else {
                git::CommitPurpose::Checkpoint
            },
            false,
        )?);
    }
    Ok(())
}

pub(in crate::lean) fn commit_round(
    task: &mut Task,
    spec: &TaskContract,
    worktree: Option<&Path>,
    _round: u32,
    report: &str,
    summary: &str,
    commit_message: &str,
) -> Result<Option<String>> {
    task.report = Some(report.to_owned());
    let (Some(path), Some(base)) = (worktree, task.base.as_ref()) else {
        return Ok(None);
    };
    turn_result::validate_worker_commit_subject(commit_message)?;
    let result = if spec.change_kind() == tasks::ChangeKind::Artifact {
        git::commit_artifacts_with_message(path, &spec.outputs, commit_message, true)?
    } else {
        git::commit_all_with_message(path, commit_message)?
    };
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
    purpose: git::CommitPurpose,
    require_all: bool,
) -> Result<String> {
    if spec.change_kind() == tasks::ChangeKind::Artifact {
        git::commit_artifacts(path, &spec.outputs, purpose, require_all)
    } else {
        git::commit_all(path, purpose)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_completion_subjects_reach_distinct_task_result_commits() {
        let directory = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/antislop.toml"
        )))
        .unwrap();
        let core = Core::open(&directory.path().join("state"), config).unwrap();
        let cases = [
            (
                "feature",
                "feat(api): add pagination cursor",
                "Add cursor pagination",
                r#"
pub fn page_after<'a>(items: &'a [String], cursor: Option<&str>, limit: usize) -> Vec<&'a str> {
    let start = match cursor {
        Some(value) => match items.iter().position(|item| item == value) {
            Some(index) => index + 1,
            None => return Vec::new(),
        },
        None => 0,
    };
    items.iter().skip(start).take(limit).map(String::as_str).collect()
}

#[cfg(test)]
mod tests {
    use super::page_after;

    #[test]
    fn returns_the_page_after_its_cursor() {
        let items = ["a", "b", "c", "d"].map(str::to_owned);
        assert_eq!(page_after(&items, Some("b"), 2), ["c", "d"]);
    }

    #[test]
    fn starts_at_the_beginning_without_a_cursor() {
        let items = ["a", "b", "c"].map(str::to_owned);
        assert_eq!(page_after(&items, None, 2), ["a", "b"]);
    }
}
"#,
            ),
            (
                "repair",
                "fix(parser): reject unterminated strings",
                "Reject unterminated strings",
                r#"
#[derive(Debug, PartialEq)]
pub enum ParseError { ExpectedQuotedString, UnterminatedString }

pub fn parse_quoted(input: &str) -> Result<&str, ParseError> {
    if !input.starts_with('"') {
        return Err(ParseError::ExpectedQuotedString);
    }
    if input.len() < 2 || !input.ends_with('"') {
        return Err(ParseError::UnterminatedString);
    }
    Ok(&input[1..input.len() - 1])
}

#[cfg(test)]
mod tests {
    use super::{parse_quoted, ParseError};

    #[test]
    fn accepts_a_terminated_string() {
        assert_eq!(parse_quoted("\"hello\""), Ok("hello"));
    }

    #[test]
    fn rejects_an_unterminated_string() {
        assert_eq!(parse_quoted("\"hello"), Err(ParseError::UnterminatedString));
    }
}
"#,
            ),
        ];
        let mut subjects = Vec::new();
        for (id, subject, summary, source) in cases {
            let project = directory.path().join(id);
            std::fs::create_dir(&project).unwrap();
            assert!(
                std::process::Command::new("git")
                    .args(["init", "-q"])
                    .current_dir(&project)
                    .status()
                    .unwrap()
                    .success()
            );
            std::fs::write(project.join("lib.rs"), "// base\n").unwrap();
            let base = git::commit_all(&project, git::CommitPurpose::TestFixture).unwrap();
            let conversation = core
                .store
                .create_conversation(project.to_str().unwrap())
                .unwrap();
            let spec: TaskContract = serde_json::from_value(json!({"id":id,"title":summary,"goal":summary,"change_kind":"code","kind":"implementation"})).unwrap();
            let mut task = core
                .store
                .add_plan(&conversation.id, &[serde_json::to_value(&spec).unwrap()])
                .unwrap()
                .remove(0);
            task.base = Some(base.clone());
            core.store.update_task(&task).unwrap();
            let worktree = directory.path().join(format!("{id}-worker"));
            git::add_worktree(&project, &worktree, &format!("antislop/{id}"), &base).unwrap();
            std::fs::write(worktree.join("lib.rs"), source).unwrap();
            run_behavior_tests(&worktree);

            let result = turn_result::task_tool_result(
                "complete",
                &json!({
                    "summary":summary,
                    "report":format!("Implemented: {summary}"),
                    "commit_message":subject,
                }),
            )
            .unwrap();
            result.validate_task(&spec, false).unwrap();
            let raw = result.encode().unwrap();
            let completed = complete_report(
                &core,
                &conversation.id,
                &mut task,
                &spec,
                Some(&worktree),
                raw,
            )
            .unwrap()
            .unwrap();
            commit_round(
                &mut task,
                &spec,
                Some(&worktree),
                1,
                &completed.report,
                &completed.effect.0,
                &completed.commit_message,
            )
            .unwrap();
            let actual = git::ok(
                &worktree,
                &["show", "-s", "--format=%B", task.result.as_ref().unwrap()],
            )
            .unwrap();
            assert_eq!(actual, subject);
            assert_eq!(actual.lines().count(), 1);
            assert!(actual.starts_with(if id == "feature" { "feat(" } else { "fix(" }));
            subjects.push(actual);
        }
        assert_ne!(
            subjects[0], subjects[1],
            "different task changes must not reuse a fixed subject"
        );
    }

    fn run_behavior_tests(worktree: &Path) {
        let output = tempfile::tempdir().unwrap();
        let test_binary = output.path().join("behavior-tests");
        let compile = std::process::Command::new("rustc")
            .args(["--edition=2024", "--test", "lib.rs", "-o"])
            .arg(&test_binary)
            .current_dir(worktree)
            .output()
            .unwrap();
        assert!(
            compile.status.success(),
            "rustc failed: {}",
            String::from_utf8_lossy(&compile.stderr)
        );
        let tests = std::process::Command::new(&test_binary).output().unwrap();
        assert!(
            tests.status.success(),
            "behavior tests failed: {}{}",
            String::from_utf8_lossy(&tests.stdout),
            String::from_utf8_lossy(&tests.stderr)
        );
        assert!(
            String::from_utf8_lossy(&tests.stdout).contains("2 passed; 0 failed"),
            "expected both behavior tests to run: {}",
            String::from_utf8_lossy(&tests.stdout)
        );
    }

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
        let base = git::commit_all(&project, git::CommitPurpose::TestFixture).unwrap();
        let index = std::fs::read(project.join(".git/index")).unwrap();
        let store = store::Store::open(&directory.path().join("core.db")).unwrap();
        let conversation = store
            .create_conversation(project.to_str().unwrap())
            .unwrap();
        let outputs = [
            ".antislop/continuation-plan-review.md".to_owned(),
            ".antislop/appendix.md".to_owned(),
        ];
        let spec: TaskContract = serde_json::from_value(json!({"id":"report","title":"Przygotuj raport i napraw błędne załączniki","goal":"Dostarcz analizę zgodnie z ustaleniami rozmowy","kind":"analysis","change_kind":"artifact","outputs":outputs,"delivery":"untracked"})).unwrap();
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
        let error = commit_round(
            &mut task,
            &spec,
            Some(&worktree),
            1,
            "Raport z rozmowy: napraw załączniki",
            "Podsumowanie ustaleń",
            "docs: napraw załączniki i podsumuj ustalenia",
        )
        .unwrap_err()
        .to_string();
        assert!(
            error.contains("artifact output missing: .antislop/appendix.md"),
            "{error}"
        );
        std::fs::write(worktree.join(&outputs[1]), "supporting evidence").unwrap();
        commit_round(
            &mut task,
            &spec,
            Some(&worktree),
            1,
            "Raport z rozmowy: napraw załączniki",
            "Podsumowanie ustaleń",
            "docs: napraw załączniki i podsumuj ustalenia",
        )
        .unwrap();
        let result = task.result.as_ref().unwrap();
        assert_eq!(
            git::ok(&worktree, &["show", "-s", "--format=%B", result]).unwrap(),
            "docs: napraw załączniki i podsumuj ustalenia"
        );
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
            git::snapshot_with_artifacts(&project, &outputs, git::CommitPurpose::TestFixture)
                .unwrap();
        assert!(
            git::changed_files(&project, result, &current)
                .unwrap()
                .is_empty()
        );
        std::fs::write(project.join(&outputs[0]), "edited by user").unwrap();
        let changed =
            git::snapshot_with_artifacts(&project, &outputs, git::CommitPurpose::TestFixture)
                .unwrap();
        assert_eq!(
            git::changed_files(&project, result, &changed).unwrap(),
            [outputs[0].clone()]
        );
        assert_eq!(git::head(&project).unwrap(), base);
        assert_eq!(std::fs::read(project.join(".git/index")).unwrap(), index);
    }
}
