use architect::{
    domain::*,
    id,
    runtime::{Command, Runtime},
    storage::Store,
    workspace::{self, DiffFileStatus, DiffLineKind},
};
use std::{collections::BTreeMap, path::Path, sync::Arc};

fn sample_spec(root: &Path, files: &[String]) -> RunSpec {
    RunSpec {
        team_template: None,
        task_family: architect::quality::TaskFamily::Software,
        config: None,
        objective: "Test diff inspection, comments, and code review".into(),
        workspace: root.to_string_lossy().into(),
        provider: "fake".into(),
        model: None,
        requirements: vec![Requirement {
            id: "R1".into(),
            original_text: "Support diff inspection and comments".into(),
            source_ref: "user:1".into(),
            criteria: BTreeMap::from([(
                "verified".into(),
                "Must inspect diff and comments".into(),
            )]),
        }],
        plan_items: vec![PlanItem {
            dependencies: std::collections::BTreeSet::new(),
            id: "P1".into(),
            requirement_ids: std::collections::BTreeSet::from(["R1".into()]),
            description: "Implement diff and review".into(),
        }],
        input_files: files.to_vec(),
        editable_files: files.to_vec(),
        check_command: vec!["true".into()],
    }
}

fn test_runtime() -> (tempfile::TempDir, Arc<Runtime>) {
    let root = tempfile::tempdir().unwrap();
    let runtime =
        Arc::new(Runtime::new(Store::open(&root.path().join("state.db")).unwrap()).unwrap());
    (root, runtime)
}

#[test]
fn test_compute_diff_and_hunks() {
    let ws = tempfile::tempdir().unwrap();
    let root = ws.path();

    let mut baseline = BTreeMap::new();
    baseline.insert("unchanged.rs".to_string(), "fn foo() {}\n".to_string());
    baseline.insert(
        "modified.rs".to_string(),
        "line 1\nline 2\nline 3\nline 4\nline 5\n".to_string(),
    );
    baseline.insert("deleted.rs".to_string(), "pub fn old() {}\n".to_string());

    // Write workspace files
    std::fs::write(root.join("unchanged.rs"), "fn foo() {}\n").unwrap();
    std::fs::write(
        root.join("modified.rs"),
        "line 1\nline 2\nline 3 modified\nline 4\nline 5\nline 6 added\n",
    )
    .unwrap();
    std::fs::write(root.join("added.rs"), "pub fn new_func() {}\n").unwrap();
    // deleted.rs is intentionally not written to disk

    let report = workspace::compute_diff(root, &baseline).unwrap();

    assert_eq!(report.files.len(), 3);

    // Verify added file
    let added = report.files.iter().find(|f| f.path == "added.rs").unwrap();
    assert_eq!(added.status, DiffFileStatus::Added);
    assert_eq!(added.additions, 1);
    assert_eq!(added.deletions, 0);
    assert_eq!(added.hunks.len(), 1);
    assert_eq!(added.hunks[0].lines[0].kind, DiffLineKind::Addition);
    assert_eq!(added.hunks[0].lines[0].new_line_number, Some(1));

    // Verify deleted file
    let deleted = report
        .files
        .iter()
        .find(|f| f.path == "deleted.rs")
        .unwrap();
    assert_eq!(deleted.status, DiffFileStatus::Deleted);
    assert_eq!(deleted.additions, 0);
    assert_eq!(deleted.deletions, 1);
    assert_eq!(deleted.hunks.len(), 1);
    assert_eq!(deleted.hunks[0].lines[0].kind, DiffLineKind::Deletion);
    assert_eq!(deleted.hunks[0].lines[0].old_line_number, Some(1));

    // Verify modified file
    let modified = report
        .files
        .iter()
        .find(|f| f.path == "modified.rs")
        .unwrap();
    assert_eq!(modified.status, DiffFileStatus::Modified);
    assert!(modified.additions >= 2);
    assert!(modified.deletions >= 1);
    assert!(!modified.hunks.is_empty());

    // Check hunk line mapping
    let hunk = &modified.hunks[0];
    assert!(hunk.lines.iter().any(|l| l.kind == DiffLineKind::Context
        && l.old_line_number.is_some()
        && l.new_line_number.is_some()));
    assert!(hunk.lines.iter().any(|l| l.kind == DiffLineKind::Deletion
        && l.old_line_number.is_some()
        && l.new_line_number.is_none()));
    assert!(hunk.lines.iter().any(|l| l.kind == DiffLineKind::Addition
        && l.old_line_number.is_none()
        && l.new_line_number.is_some()));
}

#[test]
fn test_diff_comments_add_delete_resolve() {
    let ws = tempfile::tempdir().unwrap();
    let root = ws.path();
    std::fs::write(root.join("file.rs"), "line 1\nline 2\n").unwrap();

    let (_dir, runtime) = test_runtime();
    let spec = sample_spec(root, &["file.rs".to_string()]);
    let run = runtime.dispatch(&id(), Command::Create { spec }).unwrap();

    // Add file-level comment
    let run = runtime
        .dispatch(
            &id(),
            Command::AddDiffComment {
                run_id: run.id.clone(),
                path: "file.rs".into(),
                line_number: None,
                text: "Overall file architecture looks good".into(),
                author: "reviewer".into(),
            },
        )
        .unwrap();

    assert_eq!(run.diff_comments.len(), 1);
    assert_eq!(run.diff_comments[0].path, "file.rs");
    assert_eq!(run.diff_comments[0].line_number, None);
    assert_eq!(
        run.diff_comments[0].text,
        "Overall file architecture looks good"
    );
    assert!(!run.diff_comments[0].resolved);
    let file_comment_id = run.diff_comments[0].id.clone();

    // Add line-level comment
    let run = runtime
        .dispatch(
            &id(),
            Command::AddDiffComment {
                run_id: run.id.clone(),
                path: "file.rs".into(),
                line_number: Some(2),
                text: "Consider refactoring line 2".into(),
                author: "reviewer".into(),
            },
        )
        .unwrap();

    assert_eq!(run.diff_comments.len(), 2);
    let line_comment_id = run.diff_comments[1].id.clone();
    assert_eq!(run.diff_comments[1].line_number, Some(2));

    // Resolve line comment
    let run = runtime
        .dispatch(
            &id(),
            Command::ResolveDiffComment {
                run_id: run.id.clone(),
                comment_id: line_comment_id.clone(),
                resolved: true,
            },
        )
        .unwrap();

    assert!(
        run.diff_comments
            .iter()
            .find(|c| c.id == line_comment_id)
            .unwrap()
            .resolved
    );

    // Delete file-level comment
    let run = runtime
        .dispatch(
            &id(),
            Command::DeleteDiffComment {
                run_id: run.id.clone(),
                comment_id: file_comment_id.clone(),
            },
        )
        .unwrap();

    assert_eq!(run.diff_comments.len(), 1);
    assert_eq!(run.diff_comments[0].id, line_comment_id);
}

#[test]
fn test_diff_comments_in_orchestrator_context() {
    let ws = tempfile::tempdir().unwrap();
    let root = ws.path();
    std::fs::write(root.join("file.rs"), "line 1\n").unwrap();

    let (_dir, runtime) = test_runtime();
    let spec = sample_spec(root, &["file.rs".to_string()]);
    let mut run = runtime.dispatch(&id(), Command::Create { spec }).unwrap();

    let context_before = architect::orchestration::context(&run).unwrap();
    assert_eq!(context_before["diff_comments"].as_array().unwrap().len(), 0);
    assert_eq!(
        context_before["unresolved_diff_comments"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    // Add unresolved comment
    run.diff_comments.push(DiffComment {
        id: "c1".into(),
        run_id: run.id.clone(),
        path: "file.rs".into(),
        line_number: Some(1),
        text: "Please add validation".into(),
        author: "user".into(),
        created_at: "12345".into(),
        resolved: false,
    });

    let context_after = architect::orchestration::context(&run).unwrap();
    assert_eq!(context_after["diff_comments"].as_array().unwrap().len(), 1);
    assert_eq!(
        context_after["unresolved_diff_comments"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    // Add resolved comment
    run.diff_comments.push(DiffComment {
        id: "c2".into(),
        run_id: run.id.clone(),
        path: "file.rs".into(),
        line_number: None,
        text: "Already done".into(),
        author: "user".into(),
        created_at: "12346".into(),
        resolved: true,
    });

    let context_resolved = architect::orchestration::context(&run).unwrap();
    assert_eq!(
        context_resolved["diff_comments"].as_array().unwrap().len(),
        2
    );
    assert_eq!(
        context_resolved["unresolved_diff_comments"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn test_on_demand_diff_code_review() {
    let ws = tempfile::tempdir().unwrap();
    let root = ws.path();
    std::fs::write(root.join("main.rs"), "fn main() { println!(\"old\"); }\n").unwrap();

    let (_dir, runtime) = test_runtime();
    let spec = sample_spec(root, &["main.rs".to_string()]);
    let run = runtime.dispatch(&id(), Command::Create { spec }).unwrap();

    // Modify file in workspace
    std::fs::write(
        root.join("main.rs"),
        "fn main() { println!(\"new reviewed\"); }\n",
    )
    .unwrap();

    let scope_hash = run.diff_review_scope().unwrap();
    assert!(!scope_hash.is_empty());

    // Dispatch ReviewDiff
    let run = runtime
        .dispatch(
            &id(),
            Command::ReviewDiff {
                run_id: run.id.clone(),
                scope_hash: scope_hash.clone(),
            },
        )
        .unwrap();

    // Verify approval recorded
    assert!(
        run.approvals
            .iter()
            .any(|a| a.action == "review-diff" && a.scope_hash == scope_hash)
    );

    assert_eq!(run.status, Status::Running);
    assert!(run.reviews.is_empty()); // No invented stage pass at admission.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let run = loop {
        let current = runtime.store.lock().unwrap().get(&run.id).unwrap();
        if current.status != Status::Running {
            break current;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{:?}",
            current.diagnostic
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    let review = run.diff_reviews.last().unwrap();
    assert_eq!(
        review.state,
        architect::runtime::diff_review::State::Completed
    );
    assert_eq!(review.response.as_ref().unwrap().status, CheckStatus::Pass);
    assert!(
        review
            .response
            .as_ref()
            .unwrap()
            .method
            .contains("Synthetic")
    );
    assert!(run.reviews.is_empty());
    let session = &run.session_ledger.sessions[review.session_id.as_ref().unwrap()];
    assert!(session.native_observed);
    assert_eq!(session.scope.role, architect::config::RoleKind::Verifier);
    assert_eq!(run.status, Status::AwaitingStart);
    assert_eq!(
        run.input_snapshot["main.rs"],
        "fn main() { println!(\"old\"); }\n"
    );

    // Stale scope_hash is rejected
    assert!(
        runtime
            .dispatch(
                &id(),
                Command::ReviewDiff {
                    run_id: run.id.clone(),
                    scope_hash: "stale-scope".into(),
                },
            )
            .is_err()
    );
}
