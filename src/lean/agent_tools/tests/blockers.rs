use super::*;
use crate::lean::core_task_execution::task_execution::worker::complete_report;
use std::sync::atomic::AtomicBool;

#[test]
fn worker_blocker_ends_the_turn_without_approving_or_releasing_dependents() {
    for options in [true, false] {
        let (_directory, core, conversation) = core();
        let tasks = options_tasks(&core, &conversation);
        let mut consumer = tasks[1].clone();
        consumer.status = status::QUEUED.into();
        core.store.update_task(&consumer).unwrap();
        let mut task = tasks[0].clone();
        if !options {
            task.spec["kind"] = "analysis".into();
            task.spec["options_for"] = Value::Null;
        }
        task.status = status::WORKING.into();
        task.round = 1;
        core.store.update_task(&task).unwrap();
        core.store
            .db()
            .execute(
                "UPDATE tasks SET spec=?2 WHERE id=?1",
                rusqlite::params![task.id, task.spec.to_string()],
            )
            .unwrap();
        let spec: TaskContract = serde_json::from_value(task.spec.clone()).unwrap();
        let role = format!("worker:{}", task.id);
        let turn = core
            .store
            .start_turn(&conversation, &role, "fixture", 1, Some(&task.plan_id))
            .unwrap();
        let gateway = mcp::Gateway::start().unwrap();
        gateway.set_role(&role).unwrap();
        gateway.set_task_contract(spec.clone()).unwrap();
        gateway.begin(&turn.id).unwrap();
        assert!(gateway.tool_names().contains(&"report_blocker"));
        assert_eq!(gateway.tool_names().contains(&"complete"), !options);
        let bound_core = core.clone();
        let bound_conversation = conversation.clone();
        let bound_turn = turn.id.clone();
        gateway
            .set_handler(Arc::new(move |tool, args| {
                bound_core.agent_tool(&bound_conversation, &role, &bound_turn, tool, args)
            }))
            .unwrap();
        let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{
            "name":"report_blocker","arguments":{"turn":turn.id,"report":"None of the candidates meets the required measurement threshold.","resume_condition":"Revise the consumer's threshold or supply a valid candidate.","unblocked_work_remaining":false}
        }});
        let mut premature = request.clone();
        premature["params"]["arguments"]["unblocked_work_remaining"] = true.into();
        let rejected: Value = ureq::post(gateway.url())
            .header("authorization", format!("Bearer {}", gateway.token()))
            .send_json(&premature)
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap();
        assert_eq!(rejected["result"]["isError"], true);
        assert_eq!(
            core.store.task_by_id(&task.id).unwrap().unwrap().status,
            status::WORKING
        );
        for _ in 0..2 {
            let mut response = ureq::post(gateway.url())
                .header("authorization", format!("Bearer {}", gateway.token()))
                .send_json(&request)
                .unwrap();
            let response: Value = response.body_mut().read_json().unwrap();
            assert_ne!(response["result"]["isError"], true, "{response}");
            let receipt: Value = serde_json::from_str(
                response["result"]["content"][0]["text"]
                    .as_str()
                    .expect("MCP tool response"),
            )
            .unwrap();
            assert_eq!(receipt["recorded"], true, "{response}");
        }
        let finish = gateway
            .wait(&turn.id, &AtomicBool::new(false), || Ok(()))
            .unwrap();
        let result: TurnResult = serde_json::from_str(&finish.text).unwrap();
        assert!(matches!(result, TurnResult::Blocked { .. }));
        assert!(
            complete_report(&core, &conversation, &mut task, &spec, None, finish.text)
                .unwrap()
                .is_none()
        );
        let blocked = core.store.task_by_id(&task.id).unwrap().unwrap();
        assert_eq!(blocked.status, status::COORDINATING);
        assert!(blocked.report.unwrap().contains("None of the candidates"));
        assert_eq!(
            blocked.feedback.unwrap()["reason"],
            "Revise the consumer's threshold or supply a valid candidate."
        );
        assert_eq!(
            core.store.task_by_id(&tasks[1].id).unwrap().unwrap().status,
            status::QUEUED
        );
    }
}

#[test]
fn blockers_require_a_worker_and_a_concrete_resume_condition() {
    let (_directory, core, conversation) = core();
    let tasks = options_tasks(&core, &conversation);
    for task in &tasks {
        let role = format!("reviewer:{}", task.id);
        let turn = core
            .store
            .start_turn(&conversation, &role, "fixture", 0, Some(&task.plan_id))
            .unwrap();
        let error = core
            .agent_tool(
                &conversation,
                &role,
                &turn.id,
                "report_blocker",
                &json!({"report":"Blocked","resume_condition":"Revise","unblocked_work_remaining":false}),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("a reviewer ends"), "{error}");
        let gateway = mcp::Gateway::start().unwrap();
        gateway.set_role(&role).unwrap();
        gateway
            .set_task_contract(serde_json::from_value(task.spec.clone()).unwrap())
            .unwrap();
        assert!(!gateway.tool_names().contains(&"report_blocker"));
    }
    let role = format!("worker:{}", tasks[0].id);
    let turn = core
        .store
        .start_turn(&conversation, &role, "fixture", 0, Some(&tasks[0].plan_id))
        .unwrap();
    let error = core
        .agent_tool(
            &conversation,
            &role,
            &turn.id,
            "report_blocker",
            &json!({"report":"Blocked","resume_condition":"  "}),
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("resume_condition: required"), "{error}");
}

fn options_tasks(core: &Core, conversation: &str) -> Vec<Task> {
    let message = core
        .store
        .add_message(
            conversation,
            "user",
            "Select a viable approach before continuing.",
        )
        .unwrap();
    let run = core.store.create_run(conversation, &message.text).unwrap();
    let source = core
        .store
        .register_message_source(conversation, &run.id, message.seq)
        .unwrap();
    let change = serde_json::from_value(json!({"conversation":conversation,"run_id":run.id,"expected_spec_revision":0,"operation_id":"options-fixture","reason":"Synthetic options scope","entries":[{"id":"R1","source":{"source_id":source.id,"start":0,"end":message.text.len()},"interpretation":message.text,"criteria":[{"id":"R1.accept","text":"No dependent work until a viable choice is approved"}]}]})).unwrap();
    core.store
        .apply_ledger_change(&change, &core.state, true)
        .unwrap();
    let tasks = core.store.add_plan(conversation, &[
        json!({"id":"options","title":"Options","goal":"Choose a viable approach","change_kind": "none", "kind":"options","options_for":"consumer","requirement_ids":["R1"],"criterion_ids":["R1.accept"]}),
        json!({"id":"consumer","title":"Consumer","goal":"Apply the approach","change_kind": "none", "kind":"analysis","depends_on":["options"],"requirement_ids":["R1"],"criterion_ids":["R1.accept"]}),
    ]).unwrap();
    core.store
        .link_plan_run(conversation, &tasks[0].plan_id, &run.id)
        .unwrap();
    core.store
        .bind_task_ledger(conversation, &run.id, &tasks[0].plan_id)
        .unwrap();
    tasks
}
