use super::*;
use crate::config::Config;
use serde_json::json;

fn core() -> (tempfile::TempDir, Arc<Core>, String) {
    let directory = tempfile::tempdir().unwrap();
    let config = Config::from_toml(include_str!("../../../examples/antislop.toml")).unwrap();
    let core = Core::open(&directory.path().join("state"), config).unwrap();
    let conversation = core
        .store
        .create_conversation(directory.path().to_str().unwrap())
        .unwrap();
    (directory, core, conversation.id)
}

#[test]
fn tool_dispatch_keeps_turn_ownership_and_role_gates() {
    let (_directory, core, conversation) = core();
    let turn = core
        .store
        .start_turn(&conversation, "planner", "codex", 0, None)
        .unwrap();

    let error = core
        .agent_tool(
            &conversation,
            "orchestrator",
            &turn.id,
            "finish_turn",
            &json!({"text":"done"}),
        )
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("closed or belongs to another agent"),
        "{error}"
    );

    let error = core
        .agent_tool(
            &conversation,
            "planner",
            &turn.id,
            "answer_agent",
            &json!({"request":"q","answer":"a"}),
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("only the orchestrator"), "{error}");
}

#[test]
fn required_text_validation_preserves_shared_error_contract() {
    for value in [json!({}), json!({"text": "  "})] {
        let error = required_text(&value, "text").unwrap_err().to_string();
        assert_eq!(error, "text: required");
    }
    assert_eq!(
        required_text(&json!({"text":" ok "}), "text").unwrap(),
        " ok "
    );
}

#[cfg(unix)]
#[test]
fn create_session_pr_targets_completed_branch_and_records_success() {
    use std::os::unix::fs::PermissionsExt;
    let (directory, app, conversation) = core();
    std::process::Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    std::fs::write(directory.path().join("tracked.txt"), "base").unwrap();
    std::process::Command::new("git")
        .args(["add", "tracked.txt"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-m",
            "base",
        ])
        .current_dir(directory.path())
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["branch", "session/finished-42"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    std::fs::write(directory.path().join("user-change.txt"), "preserve").unwrap();
    app.store
        .append_event(
            &conversation,
            None,
            None,
            "session_completed",
            &json!({"branch":"session/finished-42", "summary":"Implemented the requested change."}),
        )
        .unwrap();
    let gh = directory.path().join("gh-success");
    std::fs::write(
        &gh,
        "#!/bin/sh\nprintf 'cwd=%s\\n' \"$PWD\" > \"$PR_ARGS_FILE\"\nprintf 'git_root=%s\\n' \"$(git rev-parse --show-toplevel)\" >> \"$PR_ARGS_FILE\"\nprintf 'branch=%s\\n' \"$(git branch --show-current)\" >> \"$PR_ARGS_FILE\"\nprintf 'args=%s\\n' \"$*\" >> \"$PR_ARGS_FILE\"\necho https://github.com/o/r/pull/7\n",
    )
    .unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let args_file = directory.path().join("args");
    unsafe {
        std::env::set_var("PR_ARGS_FILE", &args_file);
    }
    let result = app
        .create_session_pr_with(&conversation, gh.to_str().unwrap())
        .unwrap();
    unsafe {
        std::env::remove_var("PR_ARGS_FILE");
    }
    assert_eq!(result["status"], "succeeded");
    let invocation = std::fs::read_to_string(&args_file).unwrap();
    let field = |name: &str| {
        invocation
            .lines()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .unwrap()
    };
    let working_directory = std::path::Path::new(field("cwd"));
    assert_ne!(working_directory, directory.path());
    assert_eq!(working_directory, std::path::Path::new(field("git_root")));
    assert_eq!(field("branch"), "session/finished-42");
    assert_eq!(
        invocation.split_once("args=").unwrap().1.trim_end(),
        "pr create --head session/finished-42 --title Session changes: session/finished-42 --body ## Session summary\n\nImplemented the requested change.\n\nCreated from completed session branch `session/finished-42`."
    );
    assert_eq!(
        std::fs::read_to_string(directory.path().join("user-change.txt")).unwrap(),
        "preserve"
    );
    let events = app.store.events_since(&conversation, 0, 100).unwrap();
    assert!(events.iter().any(
        |event| event.kind == "session_action_result" && event.payload["status"] == "succeeded"
    ));
}

#[cfg(unix)]
#[test]
fn create_session_pr_records_raw_failures_and_missing_inputs() {
    use std::os::unix::fs::PermissionsExt;
    let (directory, app, conversation) = core();
    std::process::Command::new("git")
        .args(["init", "-b", "main"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    std::fs::write(directory.path().join("base"), "base").unwrap();
    std::process::Command::new("git")
        .args(["add", "base"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-m",
            "base",
        ])
        .current_dir(directory.path())
        .output()
        .unwrap();
    std::process::Command::new("git")
        .args(["branch", "session/fails"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    app.store
        .append_event(
            &conversation,
            None,
            None,
            "session_completed",
            &json!({"branch":"session/fails", "summary":"A completed change."}),
        )
        .unwrap();
    let gh = directory.path().join("gh-failure");
    std::fs::write(&gh, "#!/bin/sh\necho raw-provider-error >&2\nexit 23\n").unwrap();
    std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
    let failed = app
        .create_session_pr_with(&conversation, gh.to_str().unwrap())
        .unwrap();
    assert_eq!(failed["status"], "failed");
    assert!(
        failed["details"]
            .as_str()
            .unwrap()
            .contains("raw-provider-error")
    );
    let (_dir2, core2, conversation2) = core();
    let missing = core2
        .create_session_pr_with(&conversation2, gh.to_str().unwrap())
        .unwrap();
    assert_eq!(missing["status"], "missing_inputs");
}

#[test]
fn image_retrieval_returns_provider_image_blocks_for_all_message_linked_images() {
    use base64::Engine;
    let (directory, core, conversation) = core();
    let source = directory.path().join("ui.png");
    let image = crate::lean::content::valid_png_fixture();
    std::fs::write(&source, &image).unwrap();
    core.store
        .add_message_with_images(
            &conversation,
            "user",
            "Inspect",
            std::slice::from_ref(&source),
        )
        .unwrap();
    core.store
        .add_message_with_images(
            &conversation,
            "user",
            "Inspect this too",
            std::slice::from_ref(&source),
        )
        .unwrap();
    let turn = core
        .store
        .start_turn(&conversation, "orchestrator", "codex", 1, None)
        .unwrap();
    let result = core
        .agent_tool(
            &conversation,
            "orchestrator",
            &turn.id,
            "get_images",
            &json!({"offset":0}),
        )
        .unwrap();
    assert_eq!(result["_mcp_content"][1]["type"], "image");
    assert_eq!(result["_mcp_content"][1]["mimeType"], "image/png");
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(result["_mcp_content"][1]["data"].as_str().unwrap())
            .unwrap(),
        image
    );
    let next = core
        .agent_tool(
            &conversation,
            "orchestrator",
            &turn.id,
            "get_images",
            &json!({"offset":1}),
        )
        .unwrap();
    assert!(
        next["_mcp_content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("Image 2/2")
    );
    assert!(
        core.agent_tool(
            &conversation,
            "orchestrator",
            &turn.id,
            "get_images",
            &json!({})
        )
        .is_ok()
    );

    let incompatible = core
        .store
        .start_turn(&conversation, "orchestrator", "opencode", 1, None)
        .unwrap();
    assert!(
        core.agent_tool(
            &conversation,
            "orchestrator",
            &incompatible.id,
            "get_images",
            &json!({})
        )
        .unwrap_err()
        .to_string()
        .contains("not supported by the configured opencode provider")
    );
}

#[test]
fn submitting_a_plan_records_implicit_and_explicit_feature_choices() {
    use crate::lean::tasks::Team;

    for explicit_team in [None, Some("feature"), Some("small-change")] {
        let (_directory, core, conversation) = core();
        core.store
            .db()
            .execute(
                "UPDATE session_workflow SET phase='planning' WHERE conversation_id=?1",
                [&conversation],
            )
            .unwrap();
        let turn = core
            .store
            .start_turn(&conversation, PLANNER, "fixture", 0, None)
            .unwrap();
        let mut args = json!({
            "document": "# Plan",
            "tasks": [{
                "id": "analysis",
                "title": "Analyze",
                "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "Report findings",
                "kind": "analysis",
                "change_kind": "none"
            }]
        });
        if let Some(team) = explicit_team {
            args["team"] = json!(team);
            if team != Team::default().name() {
                args["team_reason"] = json!("The planner selected the smallest suitable gate set");
            }
        }

        core.agent_tool(&conversation, PLANNER, &turn.id, "submit_plan", &args)
            .unwrap();

        let expected_team = explicit_team
            .map(Team::parse)
            .transpose()
            .unwrap()
            .unwrap_or_default();
        assert_eq!(core.store.team(&conversation).unwrap(), expected_team);
        assert_eq!(core.store.user_selected_team(&conversation).unwrap(), None);
        let events = core.store.events_since(&conversation, 0, 100).unwrap();
        let selected = events
            .iter()
            .find(|event| event.kind == "team_selected")
            .expect("the default team choice is projected");
        assert_eq!(selected.payload["team"], expected_team.name());
        assert_eq!(selected.payload["origin"], "planner");

        // A new draft, even with another explicit recommendation, uses the
        // existing team and does not emit another recommendation popup.
        args["team"] = json!("security-fix");
        args["team_reason"] = json!("The revised plan suggests more gates");
        core.agent_tool(&conversation, PLANNER, &turn.id, "submit_plan", &args)
            .unwrap();
        assert_eq!(core.store.team(&conversation).unwrap(), expected_team);
        let guidance = core.store.team_guidance(&conversation).unwrap().unwrap();
        assert_eq!(guidance.team, expected_team);
        assert!(guidance.reason != "The revised plan suggests more gates");
        assert_eq!(
            core.store
                .events_since(&conversation, 0, 100)
                .unwrap()
                .iter()
                .filter(|event| event.kind == "team_selected")
                .count(),
            1
        );
    }
}

#[test]
fn submit_plan_preserves_a_user_selected_small_change_team() {
    use crate::lean::tasks::Team;

    let (_directory, core, conversation) = core();
    core.store
        .db()
        .execute(
            "UPDATE session_workflow SET phase='planning' WHERE conversation_id=?1",
            [&conversation],
        )
        .unwrap();
    core.store
        .select_team(&conversation, Team::SmallChange, "Selected by user in UI")
        .unwrap();
    let turn = core
        .store
        .start_turn(&conversation, PLANNER, "fixture", 0, None)
        .unwrap();
    let mut args = json!({
        "document": "# Small fix",
        "tasks": [{
            "id": "writer",
            "title": "Fix typo",
            "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "Fix a contained typo.",
            "kind": "implementation",
            "change_kind": "code"
        }],
        "team": "feature",
        "team_reason": "The planner prefers extra gates"
    });

    let error = core
        .agent_tool(&conversation, PLANNER, &turn.id, "submit_plan", &args)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("user selected rigor `small-change`"),
        "{error}"
    );
    assert_eq!(core.store.team(&conversation).unwrap(), Team::SmallChange);
    assert_eq!(
        core.store.user_selected_team(&conversation).unwrap(),
        Some(Team::SmallChange)
    );

    args["team"] = json!("small-change");
    args["team_reason"] = json!("Contained typo fix");
    core.agent_tool(&conversation, PLANNER, &turn.id, "submit_plan", &args)
        .unwrap();
    assert_eq!(core.store.team(&conversation).unwrap(), Team::SmallChange);
    assert_eq!(
        core.store.user_selected_team(&conversation).unwrap(),
        Some(Team::SmallChange)
    );
    let authored: Vec<crate::lean::tasks::PlannedTask> =
        serde_json::from_value(args["tasks"].clone()).unwrap();
    let graph = crate::lean::tasks::planned_graph(authored).unwrap();
    let finalized = crate::lean::core::plan::proposal::checked_team_plan(
        &graph,
        &core.config_snapshot(),
        core.store.team(&conversation).unwrap(),
    )
    .unwrap();
    let gates: Vec<_> = finalized
        .iter()
        .filter(|task| task.quality_gate)
        .map(|task| task.kind)
        .collect();
    assert_eq!(gates, [WorkKind::StaticAnalysis, WorkKind::Testing]);
    let writer = finalized.iter().find(|task| task.id == "writer").unwrap();
    let options = finalized
        .iter()
        .find(|task| task.options_for.as_deref() == Some("writer"))
        .unwrap();
    assert!(writer.depends_on.contains(&options.id));
    assert_eq!(options.kind, WorkKind::Options);
}

#[test]
fn submit_plan_preserves_an_unknown_legacy_team_until_the_user_reselects() {
    use crate::lean::tasks::Team;

    let (_directory, core, conversation) = core();
    core.store
        .db()
        .execute(
            "UPDATE session_workflow SET phase='planning' WHERE conversation_id=?1",
            [&conversation],
        )
        .unwrap();
    core.store
        .select_planned_team(&conversation, Team::SmallChange, "legacy planner wording")
        .unwrap();
    core.store
        .db()
        .execute(
            "UPDATE teams SET origin='unknown',user_selected=0,reason='legacy saved reason' WHERE conversation_id=?1",
            [&conversation],
        )
        .unwrap();
    let turn = core
        .store
        .start_turn(&conversation, PLANNER, "fixture", 0, None)
        .unwrap();
    let args = json!({
        "document": "# Small fix",
        "tasks": [{
            "id": "writer",
            "title": "Fix typo",
            "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "Fix a contained typo.",
            "kind": "implementation",
            "change_kind": "code"
        }],
        "team": "feature",
        "team_reason": "The planner prefers extra gates"
    });

    core.agent_tool(&conversation, PLANNER, &turn.id, "submit_plan", &args)
        .unwrap();
    assert_eq!(core.store.team(&conversation).unwrap(), Team::SmallChange);
    assert_eq!(core.store.user_selected_team(&conversation).unwrap(), None);
    assert_eq!(
        core.store.team_origin(&conversation).unwrap(),
        Some(crate::lean::store::TeamOrigin::Unknown)
    );
    let guidance = core.store.team_guidance(&conversation).unwrap().unwrap();
    assert_eq!(guidance.origin, crate::lean::store::TeamOrigin::Unknown);
    assert_eq!(guidance.reason, "legacy saved reason");
    let saved_reason: String = core
        .store
        .db()
        .query_row(
            "SELECT reason FROM teams WHERE conversation_id=?1",
            [&conversation],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(saved_reason, "legacy saved reason");

    core.store
        .select_team(&conversation, Team::Refactoring, "user reselected rigor")
        .unwrap();
    assert_eq!(
        core.store.user_selected_team(&conversation).unwrap(),
        Some(Team::Refactoring)
    );
    assert_eq!(
        core.store.team_origin(&conversation).unwrap(),
        Some(crate::lean::store::TeamOrigin::User)
    );
}

#[test]
fn task_tools_reject_superseded_input_generations() {
    let (_directory, core, conversation) = core();
    let task = core
        .store
        .add_plan(
            &conversation,
            &[json!({"id":"task","title":"Task","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Do work","change_kind": "none", "kind":"analysis"})],
        )
        .unwrap()
        .remove(0);
    let role = format!("worker:{}", task.id);
    let turn = core
        .store
        .start_turn(&conversation, &role, "codex", 0, Some(&task.plan_id))
        .unwrap();
    let _run = core
        .store
        .create_scope(&conversation, "New task inputs")
        .unwrap();
    core.store
        .db()
        .execute(
            "UPDATE task_inputs SET generation=0 WHERE task_id=?1",
            rusqlite::params![&task.id],
        )
        .unwrap();
    core.store
        .db()
        .execute(
            "UPDATE task_inputs SET generation=1 WHERE task_id=?1",
            [&task.id],
        )
        .unwrap();

    let error = core
        .agent_tool(
            &conversation,
            &role,
            &turn.id,
            "complete",
            &json!({"summary":"Done","report":"Finished"}),
        )
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("task inputs changed; this turn is superseded"),
        "{error}"
    );
}

#[test]
fn full_dependency_reports_are_paged_scoped_and_follow_the_original_producer() {
    let (_directory, core, conversation) = core();
    let specs = [
        json!({"id":"analysis","title":"Analysis","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Find all issues","change_kind": "none", "kind":"analysis"}),
        json!({"id":"consumer","title":"Consume","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Use all findings","change_kind": "none", "kind":"analysis","depends_on":["analysis"]}),
    ];
    let tasks = core.store.add_plan(&conversation, &specs).unwrap();
    let report = (1..=16)
        .map(|number| format!("Finding {number}: {}\n", "ą🦀".repeat(700)))
        .collect::<String>();
    let mut producer = tasks[0].clone();
    producer.report = Some(report.clone());
    producer.status = status::APPROVED.into();
    core.store.update_task(&producer).unwrap();
    core.store.test_completion(&producer, &report);
    let mut carried_specs = specs.clone();
    carried_specs[0]["title"] = "Carried analysis".into();
    let new_tasks = core.store.add_plan(&conversation, &carried_specs).unwrap();
    let mut carried = new_tasks[0].clone();
    carried.status = status::APPROVED.into();
    carried.feedback = Some(json!({"carried_from":producer.id}));
    core.store.test_result_source(&carried, &producer);
    core.store.update_task(&carried).unwrap();
    let consumer = &new_tasks[1];
    let role = format!("worker:{}", consumer.id);
    let turn = core
        .store
        .start_turn(&conversation, &role, "codex", 0, Some(&consumer.plan_id))
        .unwrap();
    let mut args = json!({"dependency":"analysis"});
    let mut read = String::new();
    let mut first_cursor = None;
    loop {
        let page = core
            .agent_tool(
                &conversation,
                &role,
                &turn.id,
                "read_dependency_result",
                &args,
            )
            .unwrap();
        assert_eq!(page["result_id"], producer.id);
        let text = page["report"].as_str().unwrap();
        assert!(text.len() <= 16 * 1024);
        read.push_str(text);
        if page["complete"] == true {
            break;
        }
        args["cursor"] = page["cursor"].clone();
        first_cursor.get_or_insert(page["cursor"].clone());
    }
    assert_eq!(read, report);
    assert!(read.contains("Finding 16:"));
    assert!(
        core.agent_tool("other", &role, &turn.id, "read_dependency_result", &args)
            .is_err()
    );
    assert!(
        core.agent_tool(
            &conversation,
            &role,
            &turn.id,
            "read_dependency_result",
            &json!({"dependency":"consumer"})
        )
        .is_err()
    );
    let second = core
        .store
        .start_turn(&conversation, &role, "codex", 0, Some(&consumer.plan_id))
        .unwrap();
    assert!(
        core.agent_tool(
            &conversation,
            &role,
            &second.id,
            "read_dependency_result",
            &json!({"dependency":"analysis","cursor":first_cursor})
        )
        .is_err()
    );
}

#[test]
fn foreign_conversations_cannot_submit_bind_resume_or_revise_a_plan() {
    let (directory, core, owner) = core();
    let foreign = core
        .store
        .create_conversation(directory.path().to_str().unwrap())
        .unwrap()
        .id;
    let task = core
        .store
        .add_plan(
            &owner,
            &[json!({"id":"local","title":"Local","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Report","change_kind": "none", "kind":"analysis"})],
        )
        .unwrap()
        .remove(0);
    let turn = core
        .store
        .start_turn(&owner, PLANNER, "fixture", 0, None)
        .unwrap();
    assert!(core.agent_tool(&foreign,PLANNER,&turn.id,"submit_plan",&json!({"document":"Plan","tasks":[{"id":"other","title":"Other","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Report","change_kind": "none", "kind":"analysis"}]})).is_err());
    let run = core
        .store
        .create_scope(&foreign, "Foreign requirements")
        .unwrap();
    assert!(
        core.store
            .bind_plan_scope(&foreign, &task.plan_id, &run.id)
            .is_err()
    );
    assert!(core.resume_plan(&foreign, &task.plan_id).is_err());
    let revision = store::PlanRevision {
        scope: None,
        plan: task.plan_id.clone(),
        base_revision: 0,
        reason: "Foreign revision".into(),
        tasks: vec![],
        retire: vec![],
        redo: vec![],
        replacements: Default::default(),
    };
    assert!(
        core.submit_revision(&foreign, "foreign-key", revision)
            .is_err()
    );
    assert_eq!(core.store.plan_conversation(&task.plan_id).unwrap(), owner);
    assert_eq!(core.store.plan(&task.plan_id).unwrap().len(), 1);
}

#[test]
fn answered_questions_return_their_answer_on_wait_and_are_not_asked_again() {
    let (_directory, core, conversation) = core();
    let task = core
        .store
        .add_plan(
            &conversation,
            &[json!({"id":"analysis","title":"Analysis","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Report","change_kind": "none", "kind":"analysis"})],
        )
        .unwrap()
        .remove(0);
    let role = format!("worker:{}", task.id);
    let turn = core
        .store
        .start_turn(&conversation, &role, "fixture", 0, Some(&task.plan_id))
        .unwrap();
    let request = core
        .store
        .ask_agent_question(
            &conversation,
            &turn.id,
            "question-key",
            "Which report format?",
        )
        .unwrap();
    core.store
        .answer_agent_question(&conversation, &request, "Use Markdown")
        .unwrap();
    let history = core
        .store
        .turn_question_history(&conversation, &turn.id)
        .unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["question"], "Which report format?");
    assert_eq!(history[0]["answer"], "Use Markdown");
    assert_eq!(history[0]["state"], "answered");
    assert!(history[0]["answered_ms"].is_i64());
    assert!(
        core.store
            .turn_question_history("foreign-conversation", &turn.id)
            .unwrap()
            .is_empty()
    );
    for _ in 0..2 {
        let answer = core
            .agent_tool(
                &conversation,
                &role,
                &turn.id,
                "wait_for_answer",
                &json!({}),
            )
            .unwrap();
        assert_eq!(answer["answer"], "Use Markdown");
        assert_eq!(answer["disposition"], "continue");
        assert!(answer["recorded"].is_null());
    }
    let next = core
        .store
        .start_turn(&conversation, &role, "fixture", 0, Some(&task.plan_id))
        .unwrap();
    let answer = core
        .agent_tool(
            &conversation,
            &role,
            &next.id,
            "ask",
            &json!({"question":"Which report format?"}),
        )
        .unwrap();
    assert_eq!(answer["request"], request);
    let count: i64 = core
        .store
        .db()
        .query_row(
            "SELECT count(*) FROM agent_questions WHERE conversation_id=?1",
            [&conversation],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
}

mod dependencies;

mod blockers;

mod result_evidence;

#[test]
fn answer_delivery_commits_while_integration_is_held_and_rejects_a_closed_caller() {
    let (_directory, core, conversation) = core();
    let worker = core
        .store
        .start_turn(&conversation, "worker:question", "fixture", 0, None)
        .unwrap();
    let request = core
        .store
        .ask_agent_question(&conversation, &worker.id, "question", "Which path?")
        .unwrap();
    let caller = core
        .store
        .start_turn(&conversation, "orchestrator", "fixture", 0, None)
        .unwrap();
    let integration = core.locks.integration();
    let args = json!({"request":request,"answer":"Use the approved path"});
    let receipt = core
        .answer_agent_tool(&conversation, &caller.id, &args)
        .unwrap();
    assert_eq!(receipt["delivered"], true);
    assert_eq!(
        core.answer_agent_tool(&conversation, &caller.id, &args)
            .unwrap(),
        receipt
    );
    assert_eq!(
        core.store
            .agent_question(&conversation, &request, None)
            .unwrap()["answer"],
        "Use the approved path"
    );
    assert!(
        core.answer_agent_tool(
            &conversation,
            &caller.id,
            &json!({"request":request,"answer":"Different"})
        )
        .is_err()
    );
    core.store
        .db()
        .execute(
            "UPDATE turns SET status='interrupted' WHERE id=?1",
            [&caller.id],
        )
        .unwrap();
    assert!(
        core.answer_agent_tool(&conversation, &caller.id, &args)
            .unwrap_err()
            .to_string()
            .contains("closed or superseded")
    );
    drop(integration);
}
