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
    restore_snapshot_conflicts_with_budget(project, snapshot, files, SNAPSHOT_LOOKUP_ARG_BUDGET)
        .map(|(launches, _)| launches)
}

fn restore_snapshot_conflicts_with_budget(
    project: &Path,
    snapshot: &str,
    files: &[String],
    lookup_arg_budget: usize,
) -> Result<(usize, usize)> {
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
    }

    let mut present = std::collections::HashSet::new();
    let mut process_count = 0;
    let oversized_path = files.iter().any(|file| {
        file.len() + 1 + snapshot_lookup_fixed_arg_bytes(project, snapshot) > lookup_arg_budget
    });
    let mut max_lookup_output_bytes = 0;
    if oversized_path {
        // Query each exact component in its parent tree. This avoids a
        // recursive scan and retains only a single ls-tree result at a time.
        let mut entries = std::collections::HashMap::new();
        for file in files {
            if snapshot_contains_path(
                project,
                snapshot,
                file,
                &mut entries,
                &mut process_count,
                &mut max_lookup_output_bytes,
            )? {
                present.insert(file.clone());
            }
        }
    } else {
        let lookup_batches = snapshot_lookup_batches(project, files, snapshot, lookup_arg_budget);
        for batch in lookup_batches {
            let mut args = vec!["--literal-pathspecs", "ls-tree", "-r", "-z", snapshot];
            args.push("--");
            args.extend(batch.iter().map(String::as_str));
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
                if let Ok(path) = std::str::from_utf8(&record[tab + 1..]) {
                    present.insert(path.to_owned());
                }
            }
        }
    }

    let mut restore_paths = Vec::new();
    let mut delete_paths = Vec::new();
    for file in files {
        if present.contains(file) {
            restore_paths.push(file);
        } else {
            delete_paths.push(file);
        }
    }

    process_count += apply_snapshot_paths(
        project,
        snapshot,
        &restore_paths,
        &delete_paths,
        || {},
        || {},
    )?;
    Ok((process_count, max_lookup_output_bytes))
}

fn snapshot_contains_path(
    project: &Path,
    snapshot: &str,
    path: &str,
    entries: &mut std::collections::HashMap<(String, String), Option<git::TreeEntry>>,
    process_count: &mut usize,
    max_output_bytes: &mut usize,
) -> Result<bool> {
    let mut tree = snapshot.to_owned();
    let mut components = Path::new(path).components().peekable();
    while let Some(std::path::Component::Normal(component)) = components.next() {
        let component = component.to_str().ok_or_else(|| {
            Error::Invalid("manual recovery snapshot path is not valid UTF-8".into())
        })?;
        let key = (tree.clone(), component.to_owned());
        let entry = if let Some(entry) = entries.get(&key) {
            entry.clone()
        } else {
            let entry = git::tree_entry(project, &tree, component)?;
            *process_count += 1;
            if let Some(entry) = &entry {
                *max_output_bytes = (*max_output_bytes).max(entry.stdout_bytes);
            }
            entries.insert(key, entry.clone());
            entry
        };
        let Some(entry) = entry else {
            return Ok(false);
        };
        if components.peek().is_some() {
            if !entry.is_tree {
                return Ok(false);
            }
            tree = entry.object;
        } else {
            return Ok(true);
        }
    }
    Ok(false)
}

fn apply_snapshot_paths(
    project: &Path,
    snapshot: &str,
    restore_paths: &[&String],
    delete_paths: &[&String],
    before_open: impl FnOnce(),
    before_operation: impl FnOnce(),
) -> Result<usize> {
    struct Group {
        parent: git::Parent,
        relative_parent: PathBuf,
        restore: Vec<String>,
        delete: Vec<String>,
    }
    let mut paths = std::collections::BTreeMap::<PathBuf, (Vec<String>, Vec<String>)>::new();
    for (items, restoring) in [(restore_paths, true), (delete_paths, false)] {
        for file in items {
            let key = Path::new(file.as_str())
                .parent()
                .unwrap_or(Path::new("."))
                .to_path_buf();
            let group = paths.entry(key).or_default();
            if restoring {
                group.0.push(file.as_str().to_owned());
            } else {
                group.1.push(file.as_str().to_owned());
            }
        }
    }
    before_open();
    let mut groups = Vec::new();
    for (relative_parent, (restore, delete)) in paths {
        let needs_directory = !restore.is_empty();
        let probe = restore.first().or_else(|| delete.first()).unwrap();
        let parent_path = if relative_parent.as_os_str().is_empty() {
            "."
        } else {
            relative_parent
                .to_str()
                .ok_or_else(|| Error::Invalid("recovery parent path is not UTF-8".into()))?
        };
        let file_name = Path::new(probe).file_name().unwrap().to_string_lossy();
        let opened = git::Parent::recovery_at(
            project,
            &format!("{parent_path}/{file_name}"),
            needs_directory,
        );
        let parent = match opened {
            Ok((parent, _)) => parent,
            Err(Error::Io(error))
                if !needs_directory && error.kind() == std::io::ErrorKind::NotFound =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        groups.push(Group {
            parent,
            relative_parent,
            restore: restore
                .iter()
                .map(|p| {
                    Path::new(p)
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect(),
            delete: delete
                .iter()
                .map(|p| {
                    Path::new(p)
                        .file_name()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect(),
        });
    }
    // Materialize tracked snapshot entries through Git's checkout-index so
    // attributes, filters, symlink entries, and executable modes use Git's
    // checkout rules. Selected entries then replace destination directory
    // entries with renameat (including symlinks); absent snapshot paths are
    // removed with unlinkat. A destination directory collision remains an
    // error. The directory fd pins the opened inode, not its pathname. A same-user
    // process can move that inode elsewhere while recovery runs; this code
    // does not claim checkout containment against that concurrent rename.
    before_operation();
    let staging = if restore_paths.is_empty() {
        None
    } else {
        Some(materialize_snapshot(project, snapshot, restore_paths)?)
    };
    let launches = if staging.is_some() { 2 } else { 0 };
    for group in groups {
        for name in &group.delete {
            match group.parent.unlink(name) {
                Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
                Ok(()) => {}
            }
        }
        if !group.restore.is_empty() {
            let staging = staging.as_ref().expect("restore paths have staging");
            for name in &group.restore {
                let path = group.relative_parent.join(name);
                let path = path
                    .to_str()
                    .ok_or_else(|| Error::Invalid("recovery path is not UTF-8".into()))?;
                let (source_parent, source_name) =
                    git::Parent::recovery_at(&staging.path().join("tree"), path, false)?;
                source_parent.rename(&source_name, &group.parent, name)?;
            }
        }
    }
    Ok(launches)
}

fn materialize_snapshot(
    project: &Path,
    snapshot: &str,
    paths: &[&String],
) -> Result<tempfile::TempDir> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    // Keep staging on the checkout filesystem: publication uses renameat,
    // which cannot cross devices. tempdir_in creates a private directory and
    // does not follow TMPDIR, which may point at another mount.
    let staging = tempfile::Builder::new()
        .prefix("antislop-recovery-")
        .tempdir_in(project)?;
    let stage_root = staging.path().join("tree");
    std::fs::create_dir(&stage_root)?;
    let index = staging.path().join("index");
    let run = |args: &[&str], input_paths: Option<&[&String]>| -> Result<()> {
        let mut child = Command::new("git")
            .arg("-C")
            .arg(project)
            .args(args)
            .env("GIT_INDEX_FILE", &index)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        if let Some(paths) = input_paths {
            let mut stdin = child.stdin.take().expect("piped git stdin");
            for path in paths {
                stdin.write_all(path.as_bytes())?;
                stdin.write_all(&[0])?;
            }
        }
        let output = child.wait_with_output()?;
        require(
            output.status.success(),
            &format!(
                "git {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        )
    };
    run(&["read-tree", snapshot], None)?;
    let prefix = format!("--prefix={}/", stage_root.display());
    run(
        &["checkout-index", "--force", "--stdin", "-z", &prefix],
        Some(paths),
    )?;
    Ok(staging)
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
    lookup_arg_budget: usize,
) -> Vec<&'a [String]> {
    let fixed_args_bytes = snapshot_lookup_fixed_arg_bytes(project, snapshot);
    let mut batches = Vec::new();
    let mut start = 0;
    let mut bytes = fixed_args_bytes;
    for (index, file) in files.iter().enumerate() {
        let cost = file.len() + 1;
        if index > start && bytes + cost > lookup_arg_budget {
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
            .args(["-c", "core.hooksPath=/dev/null"])
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
        let lookup_batches = snapshot_lookup_batches(
            dir.path(),
            &conflicts,
            &snapshot,
            SNAPSHOT_LOOKUP_ARG_BUDGET,
        );
        assert!(
            lookup_batches.len() > 1,
            "test must exercise bounded lookup chunks"
        );
        assert_eq!(launches, lookup_batches.len() + 2);
        assert_eq!(
            launches, 4,
            "two ls-tree batches and two temporary-index checkout processes"
        );
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
    fn oversized_snapshot_lookup_walks_exact_tree_entries_with_bounded_output() {
        let dir = tempfile::tempdir().unwrap();
        run_git(dir.path(), &["init", "-q"]);
        run_git(dir.path(), &["config", "user.name", "Test"]);
        run_git(
            dir.path(),
            &["config", "user.email", "test@example.invalid"],
        );

        for index in 0..1500 {
            std::fs::write(
                dir.path().join(format!("root-entry-{index:04}.txt")),
                "unrelated snapshot entry\n",
            )
            .unwrap();
        }
        let exact_paths = [
            "deep/nested/with\ttab.txt",
            "deep/nested/with\nnewline.txt",
            "literal[*]?name.txt",
        ];
        for path in exact_paths {
            let path = dir.path().join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "snapshot value\n").unwrap();
        }
        run_git(dir.path(), &["add", "--all"]);
        run_git(dir.path(), &["commit", "-qm", "snapshot"]);
        let snapshot = git::ok(dir.path(), &["rev-parse", "HEAD"]).unwrap();

        for path in exact_paths {
            std::fs::write(dir.path().join(path), "user edit\n").unwrap();
        }
        let deleted = "deep/nested/deleted\tfile.txt";
        std::fs::write(dir.path().join(deleted), "user only\n").unwrap();
        let requests = [
            exact_paths[0].to_owned(),
            exact_paths[1].to_owned(),
            exact_paths[2].to_owned(),
            deleted.to_owned(),
        ];
        let (launches, max_entry_output) =
            restore_snapshot_conflicts_with_budget(dir.path(), &snapshot, &requests, 1).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.path().join(exact_paths[0])).unwrap(),
            "snapshot value\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join(exact_paths[1])).unwrap(),
            "snapshot value\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join(exact_paths[2])).unwrap(),
            "snapshot value\n"
        );
        assert!(!dir.path().join(deleted).exists());
        assert!(max_entry_output > 0);
        assert!(
            max_entry_output < 128,
            "one exact entry only: {max_entry_output}"
        );
        assert_eq!(
            launches, 8,
            "shared prefixes should be queried once before the two staging commands"
        );
        println!(
            "oversized recovery: {launches} git launches, max exact-entry output {max_entry_output} bytes, 1,503 tree entries"
        );
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_staging_uses_checkout_filesystem_for_descriptor_publication() {
        use std::os::unix::fs::MetadataExt;

        let dir = tempfile::tempdir().unwrap();
        run_git(dir.path(), &["init", "-q"]);
        run_git(dir.path(), &["config", "user.name", "Test"]);
        run_git(
            dir.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        std::fs::write(dir.path().join("restore.txt"), "snapshot value\n").unwrap();
        run_git(dir.path(), &["add", "--all"]);
        run_git(dir.path(), &["commit", "-qm", "snapshot"]);
        let snapshot = git::ok(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        std::fs::write(dir.path().join("restore.txt"), "user edit\n").unwrap();

        let path = "restore.txt".to_owned();
        let staging = materialize_snapshot(dir.path(), &snapshot, &[&path]).unwrap();
        assert_eq!(
            staging.path().parent(),
            Some(dir.path()),
            "staging must be created under the checkout, independent of TMPDIR"
        );
        assert_eq!(
            staging.path().metadata().unwrap().dev(),
            dir.path().metadata().unwrap().dev(),
            "renameat publication requires staging and checkout on one filesystem"
        );

        let (source_parent, source_name) =
            git::Parent::recovery_at(&staging.path().join("tree"), "restore.txt", false).unwrap();
        let (destination_parent, _) =
            git::Parent::recovery_at(dir.path(), "restore.txt", false).unwrap();
        source_parent
            .rename(&source_name, &destination_parent, "restore.txt")
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("restore.txt")).unwrap(),
            "snapshot value\n"
        );
    }

    #[test]
    fn grouped_snapshot_restore_keeps_parent_symlink_protection() {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            run_git(dir.path(), &["init", "-q"]);
            run_git(dir.path(), &["config", "user.name", "Test"]);
            run_git(
                dir.path(),
                &["config", "user.email", "test@example.invalid"],
            );
            std::fs::write(dir.path().join("tracked.txt"), "snapshot\n").unwrap();
            run_git(dir.path(), &["add", "--all"]);
            run_git(dir.path(), &["commit", "-qm", "snapshot"]);
            let snapshot = git::ok(dir.path(), &["rev-parse", "HEAD"]).unwrap();
            std::os::unix::fs::symlink(outside.path(), dir.path().join("linked")).unwrap();
            let err = restore_snapshot_conflicts(dir.path(), &snapshot, &["linked/file".into()])
                .unwrap_err();
            assert!(err.to_string().contains("symlink"));
            assert!(!outside.path().join("file").exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_restore_and_delete_stay_on_open_parent_after_symlink_swap() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        run_git(dir.path(), &["init", "-q"]);
        run_git(dir.path(), &["config", "user.name", "Test"]);
        run_git(
            dir.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/restore.txt"), "snapshot value\n").unwrap();
        run_git(dir.path(), &["add", "--all"]);
        run_git(dir.path(), &["commit", "-qm", "snapshot"]);
        let snapshot = git::ok(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        std::fs::write(dir.path().join("sub/restore.txt"), "user edit\n").unwrap();
        std::fs::write(dir.path().join("sub/delete.txt"), "delete this\n").unwrap();
        std::fs::write(dir.path().join("sub/unrelated.txt"), "keep edit\n").unwrap();
        std::fs::write(outside.path().join("restore.txt"), "outside restore\n").unwrap();
        std::fs::write(outside.path().join("delete.txt"), "outside delete\n").unwrap();

        apply_snapshot_paths(
            dir.path(),
            &snapshot,
            &[&"sub/restore.txt".to_owned()],
            &[&"sub/delete.txt".to_owned()],
            || {},
            || {
                std::fs::rename(dir.path().join("sub"), dir.path().join("detached")).unwrap();
                std::os::unix::fs::symlink(outside.path(), dir.path().join("sub")).unwrap();
            },
        )
        .unwrap();

        assert_eq!(
            std::fs::read_to_string(outside.path().join("restore.txt")).unwrap(),
            "outside restore\n"
        );
        assert_eq!(
            std::fs::read_to_string(outside.path().join("delete.txt")).unwrap(),
            "outside delete\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("detached/restore.txt")).unwrap(),
            "snapshot value\n"
        );
        assert!(!dir.path().join("detached/delete.txt").exists());
        assert_eq!(
            std::fs::read_to_string(dir.path().join("detached/unrelated.txt")).unwrap(),
            "keep edit\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn recovery_rejects_parent_symlink_installed_before_open() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        run_git(dir.path(), &["init", "-q"]);
        run_git(dir.path(), &["config", "user.name", "Test"]);
        run_git(
            dir.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/restore.txt"), "snapshot\n").unwrap();
        run_git(dir.path(), &["add", "--all"]);
        run_git(dir.path(), &["commit", "-qm", "snapshot"]);
        let snapshot = git::ok(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        std::fs::write(dir.path().join("sub/restore.txt"), "user edit\n").unwrap();
        std::fs::write(outside.path().join("restore.txt"), "outside edit\n").unwrap();

        let error = apply_snapshot_paths(
            dir.path(),
            &snapshot,
            &[&"sub/restore.txt".to_owned()],
            &[],
            || {
                std::fs::rename(dir.path().join("sub"), dir.path().join("detached")).unwrap();
                std::os::unix::fs::symlink(outside.path(), dir.path().join("sub")).unwrap();
            },
            || {},
        )
        .unwrap_err();
        assert!(error.to_string().contains("symlink"));
        assert_eq!(
            std::fs::read_to_string(outside.path().join("restore.txt")).unwrap(),
            "outside edit\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("detached/restore.txt")).unwrap(),
            "user edit\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn recovery_fd_tracks_moved_parent_inode_without_claiming_root_containment() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        run_git(dir.path(), &["init", "-q"]);
        run_git(dir.path(), &["config", "user.name", "Test"]);
        run_git(
            dir.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/restore.txt"), "snapshot value\n").unwrap();
        run_git(dir.path(), &["add", "--all"]);
        run_git(dir.path(), &["commit", "-qm", "snapshot"]);
        let snapshot = git::ok(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        std::fs::write(dir.path().join("sub/restore.txt"), "user edit\n").unwrap();
        std::fs::write(dir.path().join("sub/delete.txt"), "delete me\n").unwrap();
        let moved = outside.path().join("moved");

        apply_snapshot_paths(
            dir.path(),
            &snapshot,
            &[&"sub/restore.txt".to_owned()],
            &[&"sub/delete.txt".to_owned()],
            || {},
            || std::fs::rename(dir.path().join("sub"), &moved).unwrap(),
        )
        .unwrap();

        // This is a boundary test: an open fd pins the directory inode, not its
        // location. Recovery therefore changes that inode after this rename;
        // it does not promise that the directory remains beneath the checkout.
        assert_eq!(
            std::fs::read_to_string(moved.join("restore.txt")).unwrap(),
            "snapshot value\n"
        );
        assert!(!moved.join("delete.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_checkout_preserves_filters_symlinks_and_executable_modes() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let dir = tempfile::tempdir().unwrap();
        run_git(dir.path(), &["init", "-q"]);
        run_git(dir.path(), &["config", "user.name", "Test"]);
        run_git(
            dir.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        run_git(dir.path(), &["config", "filter.upper.clean", "tr A-Z a-z"]);
        run_git(dir.path(), &["config", "filter.upper.smudge", "tr a-z A-Z"]);
        std::fs::write(
            dir.path().join(".gitattributes"),
            "filtered.txt filter=upper\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("filtered.txt"), "mixed content\n").unwrap();
        std::fs::write(dir.path().join("replace.txt"), "regular snapshot file\n").unwrap();
        std::fs::write(dir.path().join("executable.sh"), "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(
            dir.path().join("executable.sh"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        symlink("filtered.txt", dir.path().join("link.txt")).unwrap();
        run_git(dir.path(), &["add", "--all"]);
        run_git(dir.path(), &["commit", "-qm", "snapshot"]);
        let snapshot = git::ok(dir.path(), &["rev-parse", "HEAD"]).unwrap();
        for path in ["filtered.txt", "executable.sh", "link.txt", "replace.txt"] {
            let path = dir.path().join(path);
            let _ = std::fs::remove_file(&path);
            std::fs::write(path, "modified\n").unwrap();
        }
        std::fs::remove_file(dir.path().join("replace.txt")).unwrap();
        symlink("outside-target", dir.path().join("replace.txt")).unwrap();

        let paths = ["filtered.txt", "executable.sh", "link.txt", "replace.txt"].map(str::to_owned);
        let refs = paths.iter().collect::<Vec<_>>();
        apply_snapshot_paths(dir.path(), &snapshot, &refs, &[], || {}, || {}).unwrap();

        assert_eq!(
            std::fs::read_to_string(dir.path().join("filtered.txt")).unwrap(),
            "MIXED CONTENT\n"
        );
        assert_eq!(
            std::fs::metadata(dir.path().join("executable.sh"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        assert_eq!(
            std::fs::read_link(dir.path().join("link.txt")).unwrap(),
            Path::new("filtered.txt")
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("replace.txt")).unwrap(),
            "regular snapshot file\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn recovery_fixture_git_commits_ignore_inherited_hooks_per_invocation() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        run_git(dir.path(), &["init", "-q"]);
        run_git(dir.path(), &["config", "user.name", "Test"]);
        run_git(
            dir.path(),
            &["config", "user.email", "test@example.invalid"],
        );
        let hooks = dir.path().join(".git/hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join("pre-commit");
        std::fs::write(&hook, "#!/bin/sh\ntouch hook-ran\n").unwrap();
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(dir.path().join("file"), "fixture\n").unwrap();
        run_git(dir.path(), &["add", "--all"]);
        run_git(dir.path(), &["commit", "-qm", "snapshot"]);
        assert!(!dir.path().join("hook-ran").exists());
    }
}
