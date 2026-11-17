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
                "goal": "Report findings",
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
            "goal": "Fix a contained typo.",
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
            "goal": "Fix a contained typo.",
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
    assert!(
        core.store
            .team_note(&conversation)
            .unwrap()
            .contains("origin unknown")
    );
    assert!(
        core.store
            .team_note(&conversation)
            .unwrap()
            .contains("legacy saved reason")
    );

    core.store
        .select_team(&conversation, Team::Refactoring, "user reselected rigor")
        .unwrap();
    assert_eq!(
        core.store.user_selected_team(&conversation).unwrap(),
        Some(Team::Refactoring)
    );
}

#[test]
fn task_tools_reject_superseded_input_generations() {
    let (_directory, core, conversation) = core();
    let task = core
        .store
        .add_plan(
            &conversation,
            &[json!({"id":"task","title":"Task","goal":"Do work","change_kind": "none", "kind":"analysis"})],
        )
        .unwrap()
        .remove(0);
    let role = format!("worker:{}", task.id);
    let turn = core
        .store
        .start_turn(&conversation, &role, "codex", 0, Some(&task.plan_id))
        .unwrap();
    let run = core
        .store
        .create_run(&conversation, "New task inputs")
        .unwrap();
    core.store
        .db()
        .execute(
            "INSERT INTO task_inputs(task_id,run_id) VALUES(?1,?2)",
            [&task.id, &run.id],
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
        json!({"id":"analysis","title":"Analysis","goal":"Find all issues","change_kind": "none", "kind":"analysis"}),
        json!({"id":"consumer","title":"Consume","goal":"Use all findings","change_kind": "none", "kind":"analysis","depends_on":["analysis"]}),
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
            &[json!({"id":"local","title":"Local","goal":"Report","change_kind": "none", "kind":"analysis"})],
        )
        .unwrap()
        .remove(0);
    let turn = core
        .store
        .start_turn(&owner, PLANNER, "fixture", 0, None)
        .unwrap();
    assert!(core.agent_tool(&foreign,PLANNER,&turn.id,"submit_plan",&json!({"document":"Plan","tasks":[{"id":"other","title":"Other","goal":"Report","change_kind": "none", "kind":"analysis"}]})).is_err());
    let run = core
        .store
        .create_run(&foreign, "Foreign requirements")
        .unwrap();
    assert!(
        core.store
            .link_plan_run(&foreign, &task.plan_id, &run.id)
            .is_err()
    );
    assert!(core.resume_plan(&foreign, &task.plan_id).is_err());
    let revision = store::PlanRevision {
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
            &[json!({"id":"analysis","title":"Analysis","goal":"Report","change_kind": "none", "kind":"analysis"})],
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
