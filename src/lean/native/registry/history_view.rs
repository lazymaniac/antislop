use super::*;
use crate::lean::git;

/// A removed read-only view can be reconstructed from its exact retained Git
/// commit. Ownership comes from the recorded turn, never an arbitrary path.
pub(super) fn restore_view_directory(core: &Core, context: &store::RunContext) -> Result<()> {
    let (task, round) = turn_scope(core, &context.turn_id)?
        .ok_or_else(|| invalid("no task owns this session directory"))?;
    let owner = core
        .store
        .task_by_id(&task)?
        .ok_or_else(|| invalid("the task for this view no longer exists"))?;
    require(
        core.store.plan_conversation(&owner.plan_id)? == context.conversation,
        "the view belongs to another conversation",
    )?;
    let id = crate::lean::core_path_helpers::task_path_component(&task)?;
    let generation: i64 = core.store.db().query_row(
        "SELECT generation FROM turn_inputs WHERE turn_id=?1",
        [&context.turn_id],
        |row| row.get(0),
    )?;
    let name = context
        .cwd
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    let expected_review = format!("{id}-review-g{generation}-{round}");
    let expected_observation = format!("{id}-observation-g{generation}");
    let directory = crate::lean::core_path_helpers::worktree_directory(&core.state)?;
    require(
        context.cwd.parent() == Some(directory.as_path())
            && (name == expected_review || name == expected_observation),
        "the session directory is not a retained task view",
    )?;
    let project = std::path::PathBuf::from(core.store.conversation(&context.conversation)?.project);
    git::restore_clean_view(&project, &context.cwd)?;
    core.queue_worktree_cleanup(&task);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_history_views_restore_only_for_the_recorded_owner_and_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        assert!(
            std::process::Command::new("git")
                .args(["init", "--quiet", "-b", "main"])
                .current_dir(&root)
                .status()
                .unwrap()
                .success()
        );
        std::fs::write(root.join(".gitignore"), ".antislop/\n").unwrap();
        std::fs::write(root.join("data"), "original").unwrap();
        let commit = git::commit_all(&root, "Original").unwrap();
        let config = crate::config::Config::from_toml(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/antislop.toml"
        )))
        .unwrap();
        let core = Core::open(&root.join(".antislop"), config).unwrap();
        let conversation = core
            .store
            .create_conversation(root.to_str().unwrap())
            .unwrap();
        let mut task=core.store.add_plan(&conversation.id,&[serde_json::json!({"id":"analyze","title":"Analyze","goal":"Report","change_kind": "none", "kind":"analysis"})]).unwrap().remove(0);
        task.status = "approved".into();
        task.round = 1;
        core.store.update_task(&task).unwrap();
        let role = format!("reviewer:{}", task.id);
        let turn = core
            .store
            .start_turn(&conversation.id, &role, "codex", 0, Some(&task.plan_id))
            .unwrap();
        let name = format!("{}-review-g0-1", task.id);
        let path = core.state.join("worktrees").join(&name);
        let branch = format!("antislop/{name}");
        git::add_worktree(&root, &path, &branch, &commit).unwrap();
        let context = store::RunContext {
            turn_id: turn.id.clone(),
            conversation: conversation.id.clone(),
            role,
            provider: "codex".into(),
            profile: "fixture".into(),
            cwd: path.clone(),
            add_dirs: vec![],

            provider_thread_id: Some("fixture-session".into()),
            provider_session_id: None,
        };
        core.store.record_run_context(&context).unwrap();
        core.store
            .finish_turn(&turn.id, store::TurnStatus::Done, None)
            .unwrap();
        git::remove_clean_view(&root, &path, &branch).unwrap();
        let foreign = core.store.create_conversation("foreign").unwrap();
        let mut wrong = context.clone();
        wrong.conversation = foreign.id;
        assert!(restore_view_directory(&core, &wrong).is_err());
        wrong = context.clone();
        wrong.cwd = root.join(&name);
        assert!(restore_view_directory(&core, &wrong).is_err());
        assert!(!path.exists());
        std::fs::write(root.join("data"), "new primary data").unwrap();
        git::commit_all(&root, "Primary advanced").unwrap();
        let graph = core.graph_updates.lock().unwrap();
        let restored = recorded_context(&core, &conversation.id, &turn.id).unwrap();
        assert_eq!(restored, context);
        assert_eq!(
            std::fs::read_to_string(path.join("data")).unwrap(),
            "original"
        );
        assert_eq!(git::head(&path).unwrap(), commit);
        drop(graph);
        let deadline = Instant::now() + Duration::from_secs(5);
        while path.exists() {
            core.drain_worktree_cleanup();
            assert!(
                Instant::now() < deadline,
                "restored history view was not cleaned"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(!path.exists());
        assert_eq!(
            std::fs::read_to_string(root.join("data")).unwrap(),
            "new primary data"
        );
    }
}
