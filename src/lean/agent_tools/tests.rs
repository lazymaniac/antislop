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
fn task_tools_reject_superseded_input_generations() {
    let (_directory, core, conversation) = core();
    let task = core
        .store
        .add_plan(
            &conversation,
            &[json!({"id":"task","title":"Task","goal":"Do work","kind":"analysis"})],
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
