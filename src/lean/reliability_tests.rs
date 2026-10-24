use super::core_path_helpers::*;
use super::core_plan_helpers::writer_base_scratch;
use super::*;

#[test]
fn task_path_components_reject_unsafe_database_ids() {
    for id in [
        "",
        "short",
        "../escape",
        "safe/escape",
        r"safe\escape",
        ".",
        "..",
        "safe..id",
        "abcdefgh.",
        "abcdefghé",
        "🦀🦀🦀",
        "abcdefgh\0",
    ] {
        let error = task_path_component(id).unwrap_err();
        assert!(error.to_string().contains("invalid task ID"), "{id:?}");
    }
}

#[test]
fn task_branches_validate_ids_and_distinguish_shared_prefixes() {
    let mut task: Task = serde_json::from_value(json!({
        "id": "", "plan_id": "plan", "spec": {}, "status": "pending",
        "round": 0, "report": null, "verdict": null, "base": null,
        "result": null
    }))
    .unwrap();
    for id in ["", "short", "abcdefgh/escape", "abcdefgh..", "🦀🦀🦀"] {
        task.id = id.to_owned();
        assert!(
            branch(&task)
                .unwrap_err()
                .to_string()
                .contains("invalid task ID")
        );
    }
    task.id = format!("combine-base-{}", "a".repeat(64));
    assert!(branch(&task).is_err());
    task.id = "12345678-1234-4234-8234-123456789abc".to_owned();
    assert_eq!(branch(&task).unwrap(), format!("antislop/{}", task.id));
    let first = branch(&task).unwrap();
    task.id = "12345678-2222-4222-8222-222222222222".into();
    assert_ne!(first, branch(&task).unwrap());
}

#[test]
fn resumed_worktrees_keep_legacy_branches_and_cleanup_the_actual_branch() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    assert!(
        std::process::Command::new("git")
            .args(["init", "--quiet", "-b", "main"])
            .current_dir(root)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(root.join("initial"), "initial").unwrap();
    let base = git::commit_all(root, "initial").unwrap();
    let mut task: Task = serde_json::from_value(json!({
        "id": "12345678-1111-4111-8111-111111111111", "plan_id": "plan",
        "spec": {}, "status": "pending", "round": 0, "report": null,
        "verdict": null, "base": null, "result": null
    }))
    .unwrap();
    let state = root.join(".antislop");
    let path = state.join("worktrees").join(&task.id);
    let legacy = "antislop/12345678";
    git::add_worktree(root, &path, legacy, &base).unwrap();
    let actual = worktree_branch(&state, &task).unwrap();
    assert_eq!(actual, legacy);
    git::add_worktree(root, &path, &actual, &base).unwrap();
    assert_eq!(git::worktree_branch(&path).unwrap(), legacy);
    // A second task sharing the prefix gets its own branch and worktree.
    task.id = "12345678-2222-4222-8222-222222222222".into();
    let other = state.join("worktrees").join(&task.id);
    let unique = worktree_branch(&state, &task).unwrap();
    git::add_worktree(root, &other, &unique, &base).unwrap();
    git::remove_worktree(root, &path, &actual).unwrap();
    assert_eq!(git::worktree_branch(&other).unwrap(), unique);
    assert!(git::add_worktree(root, &path, &unique, &base).is_err());
    assert_eq!(git::head(&other).unwrap(), base);
    git::remove_worktree_keep_branch(root, &other).unwrap();
    git::add_worktree(root, &other, &unique, &base).unwrap();
    assert_eq!(git::worktree_branch(&other).unwrap(), unique);
    std::fs::write(other.join("initial"), "changed").unwrap();
    let changed = git::commit_all(&other, "changed").unwrap();
    git::remove_worktree_keep_branch(root, &other).unwrap();
    assert!(git::add_worktree(root, &other, &unique, &base).is_err());
    git::add_worktree(root, &other, &unique, &changed).unwrap();
    assert_eq!(git::head(&other).unwrap(), changed);
    // A switched worktree must neither resume nor delete an unrelated branch.
    assert!(
        std::process::Command::new("git")
            .args(["checkout", "--quiet", "-b", "foreign-user-branch"])
            .current_dir(&other)
            .status()
            .unwrap()
            .success()
    );
    assert!(
        worktree_branch(&state, &task)
            .unwrap_err()
            .to_string()
            .contains("unrelated branch")
    );
    assert!(git::add_worktree(root, &other, &unique, &changed).is_err());
    assert!(git::remove_worktree(root, &other, &unique).is_err());
    assert!(other.exists());
    assert_eq!(git::worktree_branch(&other).unwrap(), "foreign-user-branch");
    assert_eq!(git::head(&other).unwrap(), changed);
    let preserved = std::process::Command::new("git")
        .args(["rev-parse", "--verify", &format!("refs/heads/{unique}")])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(preserved.status.success());
    assert_eq!(String::from_utf8(preserved.stdout).unwrap().trim(), changed);
    assert!(
        std::process::Command::new("git")
            .args(["checkout", "--quiet", "--detach"])
            .current_dir(&other)
            .status()
            .unwrap()
            .success()
    );
    assert!(worktree_branch(&state, &task).is_err());
    std::fs::create_dir(&path).unwrap();
    assert!(git::worktree_branch(&path).is_err());
}

#[test]
fn task_path_components_preserve_uuid_and_full_path_identity() {
    let id = uuid::Uuid::new_v4().to_string();
    assert_eq!(task_path_component(&id).unwrap(), id);
    let root = Path::new(".antislop/worktrees");
    let path = root.join(task_path_component(&id).unwrap());
    assert_eq!(path.parent(), Some(root));
    assert_eq!(path.file_name().unwrap().to_str(), Some(id.as_str()));
    // Path validation must not collapse tasks sharing a branch prefix.
    let first = "12345678-1111-4111-8111-111111111111";
    let second = "12345678-2222-4222-8222-222222222222";
    assert_ne!(
        root.join(task_path_component(first).unwrap()),
        root.join(task_path_component(second).unwrap())
    );
}

#[test]
fn scratch_paths_contain_untrusted_ids_and_distinguish_pairs() {
    let state = Path::new(".antislop");
    for id in ["../../escape", "/tmp/escape", r"a\b", "", "W1", "a..b"] {
        let path = writer_base_scratch(state, "../../plan", id);
        assert_eq!(path.parent(), Some(state.join("worktrees").as_path()));
        assert!(
            path.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("base-")
        );
    }
    assert_ne!(
        writer_base_scratch(state, "a-b", "c"),
        writer_base_scratch(state, "a", "b-c")
    );
    assert_ne!(
        writer_base_scratch(state, "p", "W1"),
        writer_base_scratch(state, "p", "w1")
    );
}
