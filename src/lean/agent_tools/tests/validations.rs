use super::*;

fn fixture() -> (tempfile::TempDir, Arc<Core>, String, Task) {
    let (directory, core, conversation) = core();
    git::ok(
        directory.path(),
        &["init", "--quiet", "--initial-branch=main"],
    )
    .unwrap();
    std::fs::write(directory.path().join(".gitignore"), "state/\ntarget/\n").unwrap();
    std::fs::write(directory.path().join("source.txt"), "original\n").unwrap();
    std::fs::write(
        directory.path().join("check.sh"),
        "printf x >> \"$VALIDATION_COUNTER\"\nprintf 'all checks passed\\n'\n",
    )
    .unwrap();
    git::commit_all(directory.path(), git::CommitPurpose::TestFixture).unwrap();
    let task = core.store.add_plan(&conversation,&[json!({"id":"check","title":"Check","goal":"Verify results","kind":"testing","change_kind":"none","acceptance_criteria":[{"id":"C1","text":"The checks passed"}]})]).unwrap().remove(0);
    (directory, core, conversation, task)
}

fn start(
    core: &Core,
    conversation: &str,
    task: &Task,
    cwd: &Path,
    reviewer: bool,
) -> (String, store::Turn) {
    let (role, turn) = super::result_evidence::start(core, conversation, task, reviewer);
    core.store
        .record_run_context(&store::RunContext {
            turn_id: turn.id.clone(),
            conversation: conversation.into(),
            role: role.clone(),
            provider: "fixture".into(),
            profile: "test".into(),
            cwd: cwd.into(),
            add_dirs: vec![],
            provider_thread_id: None,
            provider_session_id: None,
        })
        .unwrap();
    (role, turn)
}

fn request() -> Value {
    json!({"command":["sh","check.sh"],"scope":"all fixture checks","environment":"local shell; shared counter; no external services"})
}

fn complete(
    core: &Arc<Core>,
    conversation: &str,
    role: &str,
    turn: &store::Turn,
    receipt: &Value,
    exit: i32,
    problems: &str,
) -> Value {
    core.agent_tool(conversation,role,&turn.id,"complete_validation",&json!({"validation":receipt["validation"]["id"],"exit_code":exit,"output":"all checks passed\n","problems":problems})).unwrap()
}

#[test]
fn a_successful_command_runs_once_and_is_reused_across_commits_worktrees_and_restart() {
    let (directory, core, conversation, task) = fixture();
    std::fs::write(directory.path().join("source.txt"), "uncommitted input\n").unwrap();
    let (role, turn) = start(&core, &conversation, &task, directory.path(), false);
    let receipt = core
        .agent_tool(
            &conversation,
            &role,
            &turn.id,
            "prepare_validation",
            &request(),
        )
        .unwrap();
    assert_eq!(receipt["action"], "run");
    let counter = directory.path().join("state/counter");
    let result = std::process::Command::new("sh")
        .arg("check.sh")
        .current_dir(directory.path())
        .env("VALIDATION_COUNTER", &counter)
        .output()
        .unwrap();
    assert!(result.status.success());
    let finished = complete(
        &core,
        &conversation,
        &role,
        &turn,
        &receipt,
        result.status.code().unwrap(),
        "",
    );
    assert_eq!(finished["validation"]["status"], "passed");
    core.store
        .finish_turn(&turn.id, store::TurnStatus::Done, None)
        .unwrap();
    git::commit_all(directory.path(), git::CommitPurpose::TestFixture).unwrap();
    let observer = tempfile::tempdir().unwrap();
    let checkout = observer.path().join("view");
    git::ok(
        directory.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "observer",
            checkout.to_str().unwrap(),
            "HEAD",
        ],
    )
    .unwrap();
    let (reviewer, review) = start(&core, &conversation, &task, &checkout, true);
    let reused = core
        .agent_tool(
            &conversation,
            &reviewer,
            &review.id,
            "prepare_validation",
            &request(),
        )
        .unwrap();
    assert_eq!(reused["action"], "reuse");
    assert_eq!(reused["validation"]["id"], receipt["validation"]["id"]);
    assert_eq!(std::fs::read(&counter).unwrap(), b"x");
    assert!(core.store.result_evidence(&review.id).unwrap().iter().any(|evidence|Some(evidence.id.as_str())==reused["validation"]["evidence"].as_str()));
    let evidence = core
        .agent_tool(
            &conversation,
            &reviewer,
            &review.id,
            "read_evidence",
            &json!({"evidence":reused["validation"]["evidence"]}),
        )
        .unwrap();
    assert!(
        evidence["text"]
            .as_str()
            .unwrap()
            .contains("all checks passed")
    );
    core.store
        .finish_turn(&review.id, store::TurnStatus::Done, None)
        .unwrap();
    let config = (*core.config_snapshot()).clone();
    drop(core);
    let reopened = Core::open(&directory.path().join("state"), config).unwrap();
    let (reviewer, review) = start(&reopened, &conversation, &task, &checkout, true);
    assert_eq!(
        reopened
            .agent_tool(
                &conversation,
                &reviewer,
                &review.id,
                "prepare_validation",
                &request()
            )
            .unwrap()["action"],
        "reuse"
    );
    reopened
        .store
        .finish_turn(&review.id, store::TurnStatus::Done, None)
        .unwrap();
    reopened.store.delete_session(&conversation).unwrap();
    assert!(
        reopened
            .store
            .validation(&conversation, receipt["validation"]["id"].as_str().unwrap())
            .is_err()
    );
}

#[test]
fn unchanged_head_does_not_hide_modified_or_untracked_files_or_different_commands_and_environments()
{
    let (directory, core, conversation, task) = fixture();
    let (role, turn) = start(&core, &conversation, &task, directory.path(), false);
    let original = core
        .agent_tool(
            &conversation,
            &role,
            &turn.id,
            "prepare_validation",
            &request(),
        )
        .unwrap();
    complete(&core, &conversation, &role, &turn, &original, 0, "");
    assert_eq!(
        core.agent_tool(
            &conversation,
            &role,
            &turn.id,
            "prepare_validation",
            &request()
        )
        .unwrap()["action"],
        "reuse"
    );
    let head = git::head(directory.path()).unwrap();
    std::fs::write(directory.path().join("source.txt"), "changed\n").unwrap();
    assert_eq!(
        core.agent_tool(
            &conversation,
            &role,
            &turn.id,
            "prepare_validation",
            &request()
        )
        .unwrap()["action"],
        "run"
    );
    std::fs::write(directory.path().join("source.txt"), "original\n").unwrap();
    std::fs::write(directory.path().join("new-source.txt"), "new input\n").unwrap();
    assert_eq!(
        core.agent_tool(
            &conversation,
            &role,
            &turn.id,
            "prepare_validation",
            &request()
        )
        .unwrap()["action"],
        "run"
    );
    std::fs::remove_file(directory.path().join("new-source.txt")).unwrap();
    for (field, value) in [
        ("command", json!(["sh", "check.sh", "--more"])),
        ("environment", json!("different toolchain or fixture")),
        ("scope", json!("additional coverage")),
    ] {
        let mut changed = request();
        changed[field] = value;
        assert_eq!(
            core.agent_tool(
                &conversation,
                &role,
                &turn.id,
                "prepare_validation",
                &changed
            )
            .unwrap()["action"],
            "run"
        );
    }
    assert_eq!(git::head(directory.path()).unwrap(), head);
    let mut outside = request();
    outside["directory"] = json!("..");
    assert!(
        core.agent_tool(
            &conversation,
            &role,
            &turn.id,
            "prepare_validation",
            &outside
        )
        .unwrap_err()
        .to_string()
        .contains("outside")
    );
}

#[test]
fn errors_code_changes_and_invalidations_cannot_be_reused_as_success() {
    let (directory, core, conversation, task) = fixture();
    let (role, turn) = start(&core, &conversation, &task, directory.path(), false);
    for (exit, problem) in [
        (1, "compiler error"),
        (0, "test runner reported errors despite exit zero"),
    ] {
        let run = core
            .agent_tool(
                &conversation,
                &role,
                &turn.id,
                "prepare_validation",
                &request(),
            )
            .unwrap();
        assert_eq!(run["action"], "run");
        assert_eq!(
            complete(&core, &conversation, &role, &turn, &run, exit, problem)["validation"]["status"],
            "failed"
        );
    }
    let run = core
        .agent_tool(
            &conversation,
            &role,
            &turn.id,
            "prepare_validation",
            &request(),
        )
        .unwrap();
    std::fs::write(
        directory.path().join("source.txt"),
        "changed during validation\n",
    )
    .unwrap();
    assert_eq!(
        complete(&core, &conversation, &role, &turn, &run, 0, "")["validation"]["status"],
        "changed"
    );
    let run = core
        .agent_tool(
            &conversation,
            &role,
            &turn.id,
            "prepare_validation",
            &request(),
        )
        .unwrap();
    let passed = complete(&core, &conversation, &role, &turn, &run, 0, "");
    core.agent_tool(&conversation,&role,&turn.id,"invalidate_validation",&json!({"validation":passed["validation"]["id"],"reason":"test discovery omitted a required target"})).unwrap();
    let new_run = core
        .agent_tool(
            &conversation,
            &role,
            &turn.id,
            "prepare_validation",
            &request(),
        )
        .unwrap();
    assert_eq!(new_run["action"], "run");
    complete(
        &core,
        &conversation,
        &role,
        &turn,
        &new_run,
        1,
        "new failure after the earlier success",
    );
    let retry = core
        .agent_tool(
            &conversation,
            &role,
            &turn.id,
            "prepare_validation",
            &request(),
        )
        .unwrap();
    assert_eq!(retry["action"], "run");
    assert_ne!(retry["validation"]["id"], new_run["validation"]["id"]);
}

#[test]
fn concurrent_agents_wait_and_interrupted_claims_are_recoverable_without_timeouts() {
    let (directory, core, conversation, task) = fixture();
    let (worker, first) = start(&core, &conversation, &task, directory.path(), false);
    let claimed = core
        .agent_tool(
            &conversation,
            &worker,
            &first.id,
            "prepare_validation",
            &request(),
        )
        .unwrap();
    let (reviewer, second) = start(&core, &conversation, &task, directory.path(), true);
    let waiting = core
        .agent_tool(
            &conversation,
            &reviewer,
            &second.id,
            "prepare_validation",
            &request(),
        )
        .unwrap();
    assert_eq!(waiting["action"], "wait");
    assert_eq!(waiting["validation"]["id"], claimed["validation"]["id"]);
    assert!(core.agent_tool(&conversation,&reviewer,&second.id,"complete_validation",&json!({"validation":claimed["validation"]["id"],"exit_code":0,"output":"fake","problems":""})).is_err());
    core.store
        .finish_turn(
            &first.id,
            store::TurnStatus::Interrupted,
            Some("daemon restart"),
        )
        .unwrap();
    assert_eq!(
        core.agent_tool(
            &conversation,
            &reviewer,
            &second.id,
            "prepare_validation",
            &request()
        )
        .unwrap()["action"],
        "run"
    );
    let other = core
        .store
        .create_conversation(directory.path().to_str().unwrap())
        .unwrap();
    assert!(
        core.store
            .validation(&other.id, claimed["validation"]["id"].as_str().unwrap())
            .is_err()
    );
}

#[test]
fn missing_raw_evidence_forces_fresh_execution_and_unversioned_results_are_not_cacheable() {
    let (directory, core, conversation, task) = fixture();
    let (role, turn) = start(&core, &conversation, &task, directory.path(), false);
    let run = core
        .agent_tool(
            &conversation,
            &role,
            &turn.id,
            "prepare_validation",
            &request(),
        )
        .unwrap();
    let finished = complete(&core, &conversation, &role, &turn, &run, 0, "");
    let evidence = core
        .store
        .evidence(
            &conversation,
            finished["validation"]["evidence"].as_str().unwrap(),
        )
        .unwrap();
    let path = core
        .store
        .content
        .path(&crate::lean::content::Reference {
            id: evidence.id,
            bytes: evidence.bytes,
        })
        .unwrap();
    std::fs::remove_file(path).unwrap();
    let fresh = core
        .agent_tool(
            &conversation,
            &role,
            &turn.id,
            "prepare_validation",
            &request(),
        )
        .unwrap();
    assert_eq!(fresh["action"], "run");
    assert!(
        fresh["prior_evidence_error"]
            .as_str()
            .unwrap()
            .contains("unavailable")
    );
    std::fs::remove_dir_all(directory.path().join(".git")).unwrap();
    let unversioned = core
        .agent_tool(
            &conversation,
            &role,
            &turn.id,
            "prepare_validation",
            &request(),
        )
        .unwrap();
    assert_eq!(
        complete(&core, &conversation, &role, &turn, &unversioned, 0, "")["validation"]["status"],
        "uncacheable"
    );
    assert_eq!(
        core.agent_tool(
            &conversation,
            &role,
            &turn.id,
            "prepare_validation",
            &request()
        )
        .unwrap()["action"],
        "run"
    );
}

#[test]
fn dirty_submodule_contents_do_not_reuse_an_unchanged_gitlink() {
    let (directory, core, conversation, task) = fixture();
    let child = directory.path().join("dependency");
    std::fs::create_dir(&child).unwrap();
    git::ok(&child, &["init", "--quiet", "--initial-branch=main"]).unwrap();
    std::fs::write(child.join("lib.txt"), "baseline\n").unwrap();
    git::commit_all(&child, git::CommitPurpose::TestFixture).unwrap();
    git::commit_all(directory.path(), git::CommitPurpose::TestFixture).unwrap();
    let (role, turn) = start(&core, &conversation, &task, directory.path(), false);
    let run = core
        .agent_tool(
            &conversation,
            &role,
            &turn.id,
            "prepare_validation",
            &request(),
        )
        .unwrap();
    assert!(!run["validation"]["tree"].is_null());
    assert_eq!(
        complete(&core, &conversation, &role, &turn, &run, 0, "")["validation"]["status"],
        "passed"
    );
    std::fs::write(child.join("lib.txt"), "uncommitted nested source\n").unwrap();
    let changed = core
        .agent_tool(
            &conversation,
            &role,
            &turn.id,
            "prepare_validation",
            &request(),
        )
        .unwrap();
    assert_eq!(changed["action"], "run");
    assert!(changed["validation"]["tree"].is_null());
    assert_eq!(
        complete(&core, &conversation, &role, &turn, &changed, 0, "")["validation"]["status"],
        "uncacheable"
    );
}

#[test]
fn structured_validation_checks_raw_test_totals_and_keeps_limitations_separate() {
    let (directory, core, conversation, task) = fixture();
    let (role, turn) = start(&core, &conversation, &task, directory.path(), false);
    let prepare = |command: Value| {
        core.agent_tool(
            &conversation,
            &role,
            &turn.id,
            "prepare_validation",
            &json!({"command":command,"scope":"targeted tests","environment":"fixture"}),
        )
    };
    assert!(prepare(json!(["/bin/sh", "-lc", "cargo test | tail"])).is_err());
    for (filter, totals, expected) in [
        ("match", "ok. 2 passed; 0 failed; 0 ignored", "passed"),
        ("typo", "ok. 0 passed; 0 failed; 0 ignored", "failed"),
        ("masked", "FAILED. 1 passed; 1 failed; 0 ignored", "failed"),
    ] {
        let receipt = prepare(json!(["./cargo.sh", "test", "--lib", filter])).unwrap();
        let finished = core
            .agent_tool(
                &conversation,
                &role,
                &turn.id,
                "complete_validation",
                &json!({"validation":receipt["validation"]["id"],"exit_code":0,
                "output":format!("test result: {totals}; 90 filtered out"),
                "problems":[],"limitations":["Only this filter was run"]}),
            )
            .unwrap();
        let validation = &finished["validation"];
        assert_eq!(validation["status"], expected);
        let stored = core
            .store
            .validation(&conversation, validation["id"].as_str().unwrap())
            .unwrap();
        let observation = stored.observation.unwrap();
        assert_eq!(observation.limitations, ["Only this filter was run"]);
        assert_eq!(
            observation.tests_executed,
            Some(if filter == "typo" { 0 } else { 2 })
        );
    }
}

#[test]
fn a_reproduced_finding_requires_a_new_matching_validation_on_the_reviewed_tree() {
    let (directory, core, conversation, task) = fixture();
    let spec = serde_json::from_value(task.spec.clone()).unwrap();
    let checks: serde_json::Map<_, _> = tasks::obligations(&spec)
        .into_iter()
        .map(|(id, _)| (id, json!({"status":"ok","note":"Checked in fixture"})))
        .collect();
    let (reviewer, first) = start(&core, &conversation, &task, directory.path(), true);
    let observe = |turn: &store::Turn, exit, command: Value| {
        let receipt = core
            .agent_tool(
                &conversation,
                &reviewer,
                &turn.id,
                "prepare_validation",
                &json!({"command":command,"scope":"Scenario","environment":"Fixture environment"}),
            )
            .unwrap();
        let result = core.agent_tool(&conversation, &reviewer, &turn.id, "complete_validation",
            &json!({"validation":receipt["validation"]["id"],"exit_code":exit,"problems":[],"output":"Observed the fixture"})).unwrap();
        result["validation"]["id"].as_str().unwrap().to_owned()
    };
    let failing = observe(&first, 101, json!(["sh", "check.sh"]));
    let review = core.agent_tool(&conversation, &reviewer, &first.id, "review",
        &json!({"report":"Reproduced","checks":checks,"findings":[{
          "check":"goal","severity":"medium","issue":"Cannot resume",
          "where":"source.txt","reproduction":{"scenario":"resume-after-error","validation":failing}
        }]})).unwrap();
    assert_eq!(review["result"]["verdict"]["approved"], false);
    let finding = review["result"]["verdict"]["findings"][0]["id"]
        .as_str()
        .unwrap();
    core.store
        .finish_turn(&first.id, store::TurnStatus::Done, None)
        .unwrap();
    let (_, second) = start(&core, &conversation, &task, directory.path(), true);
    let previous = core
        .agent_tool(
            &conversation,
            &reviewer,
            &second.id,
            "read_dependency_result",
            &json!({"source":"previous_review"}),
        )
        .unwrap();
    assert!(previous["report"].as_str().unwrap().contains(finding));
    let mut verdict = json!({"report":"Fixed","checks":checks,"findings":[],"resolved":[finding]});
    assert!(
        core.agent_tool(&conversation, &reviewer, &second.id, "review", &verdict)
            .is_err()
    );
    let wrong_command = observe(&second, 0, json!(["sh", "different.sh"]));
    verdict["resolution_evidence"] =
        json!({(finding):{"scenario":"resume-after-error","validation":wrong_command}});
    assert!(
        core.agent_tool(&conversation, &reviewer, &second.id, "review", &verdict)
            .is_err()
    );
    let wrong_tree = observe(&second, 0, json!(["sh", "check.sh"]));
    std::fs::write(directory.path().join("source.txt"), "fixed\n").unwrap();
    verdict["resolution_evidence"][finding]["validation"] = json!(wrong_tree);
    assert!(
        core.agent_tool(&conversation, &reviewer, &second.id, "review", &verdict)
            .is_err()
    );
    let passing = observe(&second, 0, json!(["sh", "check.sh"]));
    verdict["resolution_evidence"][finding]["validation"] = json!(passing);
    let result = core
        .agent_tool(&conversation, &reviewer, &second.id, "review", &verdict)
        .unwrap();
    assert_eq!(result["result"]["verdict"]["approved"], true);
}
