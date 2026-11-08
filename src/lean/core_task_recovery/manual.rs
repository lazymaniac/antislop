use super::super::core_task_execution::reviewer_setup;
use super::*;

mod finish;

struct ManualReview {
    snapshot: String,
    base: String,
    stat: String,
    verdict: Verdict,
}

pub(super) fn apply(core: &Arc<Core>, work: &mut RecoveryWork) -> Result<()> {
    let review = prepare_review(core, work)?;
    update_review_state(work, &review)?;
    let fixed =
        git::changed_files(&work.project, &review.base, &review.snapshot).unwrap_or_default();
    work.task.feedback = Some(feedback(&review, &work.spec, &fixed, work.task.round));
    if review.verdict.approved {
        finish::approve(core, work, &review, &fixed)
    } else {
        finish::reject(core, work, &review, &fixed)
    }
}

fn prepare_review(core: &Core, work: &mut RecoveryWork) -> Result<ManualReview> {
    let snapshot = git::snapshot(&work.project, git::CommitPurpose::ConflictSnapshot)?;
    let base = work
        .task
        .base
        .clone()
        .unwrap_or_else(|| git::head(&work.project).unwrap_or_default());
    let stat = git::diff_stat(&work.project, &base, &snapshot)?;
    let changes = tasks::changes_text(&base, &snapshot, &stat);
    work.task.round += 1;
    work.task.verdict = None;
    work.task.base = Some(base.clone());
    work.task.result = Some(snapshot.clone());
    core.set_status(&work.req.conversation, &mut work.task, status::REVIEWING)?;
    let verdict = run_reviewer(core, work, &snapshot, &changes)?;
    let obligations = tasks::obligations(&work.spec);
    let mut verdict = verdict;
    tasks::settle(
        &mut verdict,
        &obligations,
        !work.spec.change_kind.has_delivery(),
    );
    Ok(ManualReview {
        snapshot,
        base,
        stat,
        verdict,
    })
}

fn run_reviewer(
    core: &Core,
    work: &mut RecoveryWork,
    snapshot: &str,
    changes: &str,
) -> Result<Verdict> {
    let (setup, project) = reviewer_setup(core, work, snapshot)?;
    let upstream = core.upstream(&work.task.plan_id, &work.spec)?;
    let cancel = AtomicBool::new(false);
    let answer = core.turn(setup, &cancel, |fresh, _| {
        reviewer_prompt(
            &project,
            &work.spec,
            work.task.report.as_deref().unwrap_or(""),
            changes,
            fresh,
            &upstream,
        )
    })?;
    decode_answer(core, work, &answer, &cancel)
}

fn reviewer_setup<'a>(
    core: &Core,
    work: &'a RecoveryWork,
    snapshot: &str,
) -> Result<(TurnSetup<'a>, String)> {
    let reviewer = format!("reviewer:{}", work.task.id);
    let reviewdir = super::super::core_path_helpers::worktree_path(&core.state, &work.task)?;
    let branch = super::super::core_path_helpers::branch(&work.task)?;
    if reviewdir.exists() {
        require(
            git::worktree_branch(&reviewdir)? == branch,
            "manual recovery task worktree is attached to a different branch; preserved",
        )?;
    } else {
        let reference = format!("refs/heads/{branch}");
        let start = if git::git(
            &work.project,
            &["show-ref", "--verify", "--quiet", &reference],
        )?
        .status
        .success()
        {
            git::ok(&work.project, &["rev-parse", &reference])?
        } else {
            snapshot.to_owned()
        };
        git::add_worktree(&work.project, &reviewdir, &branch, &start)?;
    }
    let base = work
        .task
        .base
        .as_deref()
        .ok_or_else(|| Error::Invalid("manual recovery has no review base".into()))?;
    let changed = git::changed_files(&work.project, base, snapshot)?;
    if !changed.is_empty() {
        match git::integrate_scoped(&reviewdir, base, snapshot, Some(&changed), false)? {
            git::Integration::Applied | git::Integration::Committed(_) => {}
            git::Integration::Conflict(conflicts) => {
                let mergeable = changed
                    .iter()
                    .filter(|file| !conflicts.contains(file))
                    .cloned()
                    .collect::<Vec<_>>();
                if !mergeable.is_empty() {
                    match git::integrate_scoped(
                        &reviewdir,
                        base,
                        snapshot,
                        Some(&mergeable),
                        false,
                    )? {
                        git::Integration::Applied | git::Integration::Committed(_) => {}
                        git::Integration::Conflict(files) | git::Integration::UserEdits(files) => {
                            return Err(snapshot_conflict(&files));
                        }
                    }
                }
                restore_snapshot_conflicts(&reviewdir, snapshot, &conflicts)?;
            }
            git::Integration::UserEdits(files) => {
                return Err(snapshot_conflict(&files));
            }
        }
    }
    let project = reviewdir.to_string_lossy().into_owned();
    let setup = reviewer_setup::build(
        &work.req.conversation,
        &work.task,
        reviewer,
        work.config.resolve(work.spec.kind, RoleKind::Reviewer),
        &reviewdir,
        vec![],
    );
    Ok((setup, project))
}

fn snapshot_conflict(files: &[String]) -> Error {
    Error::Invalid(format!(
        "manual recovery snapshot conflicts with uncommitted task edits in {}",
        files.join(", ")
    ))
}

// Keep each ls-tree invocation comfortably below common ARG_MAX limits. The
// restore pathspecs travel on stdin, so their total size does not consume argv.
const SNAPSHOT_LOOKUP_ARG_BUDGET: usize = 64 * 1024;

fn restore_snapshot_conflicts(project: &Path, snapshot: &str, files: &[String]) -> Result<()> {
    restore_snapshot_conflicts_counted(project, snapshot, files).map(|_| ())
}

fn restore_snapshot_conflicts_counted(
    project: &Path,
    snapshot: &str,
    files: &[String],
) -> Result<usize> {
    // Validate every path before querying or changing the working tree. In
    // particular, a later unsafe path must not follow earlier restores.
    for file in files {
        let relative = Path::new(file);
        require(
            !file.is_empty()
                && relative
                    .components()
                    .all(|part| matches!(part, std::path::Component::Normal(_))),
            "invalid path in manual recovery snapshot",
        )?;
        ensure_safe_parent(project, relative)?;
    }

    let mut present = std::collections::HashSet::new();
    let mut process_count = 0;
    let lookup_batches = snapshot_lookup_batches(project, files, snapshot);
    let oversized_path = files.iter().any(|file| {
        file.len() + 1 + snapshot_lookup_fixed_arg_bytes(project, snapshot)
            > SNAPSHOT_LOOKUP_ARG_BUDGET
    });
    let requested_paths: std::collections::HashSet<&str> =
        files.iter().map(String::as_str).collect();
    let batches: Vec<Vec<&str>> = if oversized_path {
        // A single very deep path can exceed the conservative argv budget.
        // In that rare case list the tree once and filter by exact path bytes.
        vec![Vec::new()]
    } else {
        lookup_batches
            .iter()
            .map(|batch| batch.iter().map(String::as_str).collect())
            .collect()
    };
    for batch in batches {
        let mut args = vec!["--literal-pathspecs", "ls-tree", "-r", "-z", snapshot];
        if !oversized_path {
            args.push("--");
            args.extend(batch);
        }
        let tree = git::git(project, &args)?;
        process_count += 1;
        require(
            tree.status.success(),
            "could not inspect manual recovery snapshot",
        )?;
        for record in tree
            .stdout
            .split(|byte| *byte == 0)
            .filter(|row| !row.is_empty())
        {
            let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
                continue;
            };
            if let Ok(path) = std::str::from_utf8(&record[tab + 1..])
                && (!oversized_path || requested_paths.contains(path))
            {
                present.insert(path.to_owned());
            }
        }
    }

    let mut restore_paths = Vec::new();
    for file in files {
        if present.contains(file) {
            restore_paths.push(file);
        } else {
            let destination = project.join(file);
            match std::fs::remove_file(destination) {
                Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
                    return Err(error.into());
                }
                _ => {}
            }
        }
    }

    if !restore_paths.is_empty() {
        let source = format!("--source={snapshot}");
        let mut pathspecs = Vec::new();
        for path in restore_paths {
            pathspecs.extend_from_slice(path.as_bytes());
            pathspecs.push(0);
        }
        git::ok_with_input(
            project,
            &[
                "--literal-pathspecs",
                "restore",
                &source,
                "--worktree",
                "--pathspec-from-file=-",
                "--pathspec-file-nul",
            ],
            &pathspecs,
        )?;
        process_count += 1;
    }
    Ok(process_count)
}

fn snapshot_lookup_fixed_arg_bytes(project: &Path, snapshot: &str) -> usize {
    [
        "git".len() + 1,
        "-C".len() + 1,
        project.as_os_str().as_encoded_bytes().len() + 1,
        "--literal-pathspecs".len() + 1,
        "ls-tree".len() + 1,
        "-r".len() + 1,
        "-z".len() + 1,
        snapshot.len() + 1,
        "--".len() + 1,
    ]
    .into_iter()
    .sum()
}

fn snapshot_lookup_batches<'a>(
    project: &Path,
    files: &'a [String],
    snapshot: &str,
) -> Vec<&'a [String]> {
    let fixed_args_bytes = snapshot_lookup_fixed_arg_bytes(project, snapshot);
    let mut batches = Vec::new();
    let mut start = 0;
    let mut bytes = fixed_args_bytes;
    for (index, file) in files.iter().enumerate() {
        let cost = file.len() + 1;
        if index > start && bytes + cost > SNAPSHOT_LOOKUP_ARG_BUDGET {
            batches.push(&files[start..index]);
            start = index;
            bytes = fixed_args_bytes;
        }
        bytes += cost;
    }
    if start < files.len() {
        batches.push(&files[start..]);
    }
    batches
}

fn ensure_safe_parent(project: &Path, relative: &Path) -> Result<()> {
    let mut parent = project.to_path_buf();
    let parent_count = relative.components().count().saturating_sub(1);
    for component in relative.components().take(parent_count) {
        parent.push(component);
        match std::fs::symlink_metadata(&parent) {
            Ok(metadata) => require(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "manual recovery snapshot path crosses a non-directory or symlink",
            )?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn reviewer_prompt(
    project: &str,
    spec: &TaskContract,
    report: &str,
    changes: &str,
    fresh: bool,
    upstream: &str,
) -> String {
    tasks::reviewer_turn_prompt(project, spec, report, Some(changes), fresh, upstream, &[])
}

fn decode_answer(
    core: &Core,
    work: &mut RecoveryWork,
    answer: &Option<String>,
    cancel: &AtomicBool,
) -> Result<Verdict> {
    let Some(answer) = answer else {
        return failed_reviewer(core, work, cancel);
    };
    match serde_json::from_str::<turn_result::TurnResult>(answer)? {
        turn_result::TurnResult::Review { verdict, .. } => Ok(verdict),
        turn_result::TurnResult::NeedsInput {
            request,
            checkpoint,
        } => awaiting_answer(core, work, &request, checkpoint),
        _ => Err(Error::Invalid(
            "manual snapshot requires an independent review".into(),
        )),
    }
}

fn failed_reviewer(core: &Core, work: &mut RecoveryWork, cancel: &AtomicBool) -> Result<Verdict> {
    let reviewer = format!("reviewer:{}", work.task.id);
    core.stop_task(&work.req.conversation, &mut work.task, &reviewer, cancel)?;
    Err(Error::Invalid(
        "manual snapshot reviewer failed; inspect its raw turn error".into(),
    ))
}

fn awaiting_answer(
    core: &Core,
    work: &mut RecoveryWork,
    request: &str,
    checkpoint: String,
) -> Result<Verdict> {
    work.task.report = Some(checkpoint);
    core.wait_for_question(&work.req.conversation, &mut work.task, request, true)?;
    Err(Error::Invalid(
        "manual snapshot awaits orchestrator answer".into(),
    ))
}

fn update_review_state(work: &mut RecoveryWork, review: &ManualReview) -> Result<()> {
    let mut value = serde_json::to_value(&review.verdict)?;
    if let Some(object) = value.as_object_mut() {
        object.insert("round".into(), json!(work.task.round));
        object.insert("result".into(), json!(review.snapshot));
    }
    work.task.verdict = Some(value);
    Ok(())
}

fn feedback(
    review: &ManualReview,
    spec: &TaskContract,
    fixed: &[String],
    round: u32,
) -> serde_json::Value {
    let (effect, effect_cut) = tasks::effect(&review.stat, fixed);
    let obligations = tasks::obligations(spec);
    json!({ "round": round, "effect": effect, "effect_cut": effect_cut,
        "coverage": tasks::coverage(&review.verdict, &obligations) })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn run_git(project: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(project)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn grouped_snapshot_restore_handles_many_conflicts_deletions_and_literal_names() {
        let dir = tempfile::tempdir().unwrap();
        run_git(dir.path(), &["init", "-q"]);
        run_git(dir.path(), &["config", "user.name", "Test"]);
        run_git(
            dir.path(),
            &["config", "user.email", "test@example.invalid"],
        );

        let mut files = vec![
            "with\ttab.txt".to_owned(),
            "with\nnewline.txt".to_owned(),
            "literal[*]?name.txt".to_owned(),
            "directory/deep.txt".to_owned(),
        ];
        // Push the lookup beyond one argv budget so the test covers bounded
        // query chunks while still exercising a single streamed restore.
        for index in 0..2300 {
            files.push(format!("many/conflict-{index:04}-long-name.txt"));
        }
        for file in &files {
            let path = dir.path().join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, format!("snapshot: {file}\n")).unwrap();
        }
        run_git(dir.path(), &["add", "--all"]);
        run_git(dir.path(), &["commit", "-qm", "snapshot"]);
        let snapshot = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_owned();

        for file in &files {
            std::fs::write(dir.path().join(file), "conflicting edit\n").unwrap();
        }
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(outside.path(), dir.path().join("linked")).unwrap();
            let error = restore_snapshot_conflicts(
                dir.path(),
                &snapshot,
                &[files[0].clone(), "linked/file".into()],
            )
            .unwrap_err();
            assert!(error.to_string().contains("symlink"));
            assert_eq!(
                std::fs::read_to_string(dir.path().join(&files[0])).unwrap(),
                "conflicting edit\n"
            );
            std::fs::remove_file(dir.path().join("linked")).unwrap();
        }
        std::fs::write(dir.path().join("unrelated-user-edit.txt"), "keep me\n").unwrap();
        let deleted = "removed\tby-snapshot.txt";
        std::fs::write(dir.path().join(deleted), "remove me\n").unwrap();
        let mut conflicts = files.clone();
        conflicts.push(deleted.to_owned());

        let launches =
            restore_snapshot_conflicts_counted(dir.path(), &snapshot, &conflicts).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("with\ttab.txt")).unwrap(),
            "snapshot: with\ttab.txt\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("with\nnewline.txt")).unwrap(),
            "snapshot: with\nnewline.txt\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("literal[*]?name.txt")).unwrap(),
            "snapshot: literal[*]?name.txt\n"
        );
        assert!(!dir.path().join(deleted).exists());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("unrelated-user-edit.txt")).unwrap(),
            "keep me\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("many/conflict-2299-long-name.txt")).unwrap(),
            "snapshot: many/conflict-2299-long-name.txt\n"
        );
        let lookup_batches = snapshot_lookup_batches(dir.path(), &conflicts, &snapshot);
        assert!(
            lookup_batches.len() > 1,
            "test must exercise bounded lookup chunks"
        );
        assert_eq!(launches, lookup_batches.len() + 1);
        assert_eq!(launches, 3, "two ls-tree batches and one restore process");
        // The former loop launched one ls-tree for each requested path, then
        // one restore for each path present in the snapshot. The absent
        // deletion therefore contributes only its lookup process.
        let old_per_path_launches = conflicts.len() + files.len();
        assert_eq!(files.len(), 2_304);
        assert_eq!(conflicts.len(), 2_305);
        assert_eq!(old_per_path_launches, 4_609);
        assert!(launches < old_per_path_launches);
        println!(
            "grouped recovery: {} Git launches for {} paths ({} present); old per-path loop: {}",
            launches,
            conflicts.len(),
            files.len(),
            old_per_path_launches
        );
    }

    #[test]
    fn grouped_snapshot_restore_keeps_parent_symlink_protection() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(outside.path(), dir.path().join("linked")).unwrap();
            let err = restore_snapshot_conflicts(dir.path(), "HEAD", &["linked/file".into()])
                .unwrap_err();
            assert!(err.to_string().contains("symlink"));
        }
    }
}
