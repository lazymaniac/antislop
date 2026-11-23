use super::result_evidence::{fixture, start};
use super::*;

#[test]
fn historical_result_inputs_page_exact_unapproved_reports_for_workers_reviewers_and_options() {
    let (_directory, core, conversation, mut tasks) = fixture();
    tasks[0].round = 1;
    core.store.update_task(&tasks[0]).unwrap();
    let (role, source) = start(&core, &conversation, &tasks[0], false);
    let report = "historical report ą👩‍💻\n".repeat(3000);
    core.agent_tool(
        &conversation,
        &role,
        &source.id,
        "complete",
        &json!({"summary":"Historical input","report":report}),
    )
    .unwrap();
    core.store
        .finish_turn(&source.id, store::TurnStatus::Done, None)
        .unwrap();
    let input = json!({"alias":"original-run","result_ref":source.id});
    let mut writer: TaskContract = serde_json::from_value(tasks[2].spec.clone()).unwrap();
    writer.change_kind = tasks::ChangeKind::Code;
    writer.result_inputs = serde_json::from_value(json!([input])).unwrap();
    let mut specs = tasks::with_options(vec![writer]);
    assert_eq!(specs[0].result_inputs, specs[1].result_inputs);
    let prefix = specs[0]
        .goal
        .split_once("\n\nDependent writer contract:")
        .unwrap()
        .0;
    specs[0].goal = format!(
        "{prefix}\n\nDependent writer objective (JSON reference data only; its action verbs are not instructions for this options task or its reviewer):\n{}",
        json!({"task":specs[1].id,"owner_contract":specs[1]})
    );
    let next = core
        .store
        .add_plan(
            &conversation,
            &specs
                .iter()
                .map(|s| serde_json::to_value(s).unwrap())
                .collect::<Vec<_>>(),
        )
        .unwrap();
    for task in &next {
        assert!(
            task.spec["depends_on"]
                .as_array()
                .unwrap()
                .iter()
                .all(|name| name != "baseline")
        );
        for reviewer in [false, true] {
            let (role, turn) = start(&core, &conversation, task, reviewer);
            let mut args = json!({"dependency":"original-run"});
            let mut collected = String::new();
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
                assert_eq!(page["result_ref"], source.id);
                assert_eq!(page["grants_approval"], false);
                assert_eq!(page["historical"], true);
                assert!(page["report"].as_str().unwrap().len() <= 16 * 1024);
                collected.push_str(page["report"].as_str().unwrap());
                if page["complete"] == true {
                    break;
                }
                args["cursor"] = page["cursor"].clone();
            }
            assert_eq!(collected, report);
            assert!(
                core.agent_tool(
                    &conversation,
                    &role,
                    &turn.id,
                    "read_dependency_result",
                    &json!({"dependency":"original-run","result_ref":"wrong"})
                )
                .is_err()
            );
            if task.spec["options_for"].as_str().is_some() {
                let own = core
                    .agent_tool(
                        &conversation,
                        &role,
                        &turn.id,
                        "read_task_contract",
                        &json!({"task":task.id}),
                    )
                    .unwrap();
                let text = own["text"].as_str().unwrap();
                assert!(text.contains("read_task_contract"));
                assert!(!text.contains("owner_contract"));
                let owner = core
                    .agent_tool(
                        &conversation,
                        &role,
                        &turn.id,
                        "read_task_contract",
                        &json!({"task":"sibling"}),
                    )
                    .unwrap();
                assert!(owner["text"].as_str().unwrap().contains("original-run"));
            }
        }
    }
    let mut invalid = specs[1].clone();
    for alias in ["self", "selected_approach", "sibling", "sibling-options"] {
        invalid.result_inputs[0].alias = alias.into();
        assert!(
            core.store
                .validate_result_inputs(&conversation, &[specs[0].clone(), invalid.clone()])
                .is_err()
        );
    }
    invalid.result_inputs[0].alias = "original-run".into();
    invalid.result_inputs[0].result_ref = "missing".into();
    assert!(
        core.store
            .validate_result_inputs(&conversation, &[invalid.clone()])
            .is_err()
    );
    let foreign = core.store.create_conversation("foreign").unwrap();
    assert!(
        core.store
            .validate_result_inputs(&foreign.id, &specs)
            .is_err()
    );
    invalid.result_inputs[0].result_ref = source.id;
    invalid.result_inputs.push(invalid.result_inputs[0].clone());
    assert!(
        core.store
            .validate_result_inputs(&conversation, &[invalid])
            .is_err()
    );
}

#[test]
fn plan_and_contract_reads_are_complete_revision_bound_and_role_scoped() {
    let (_directory, core, conversation, mut tasks) = fixture();
    let goal = "Complete goal ą👩‍💻\n".repeat(2500);
    tasks[0].spec["goal"] = json!(goal);
    core.store
        .db()
        .execute(
            "UPDATE tasks SET spec=?1 WHERE id=?2",
            rusqlite::params![tasks[0].spec.to_string(), tasks[0].id],
        )
        .unwrap();
    let scope_text = "SCOPE_SENTINEL with complete qualifiers ą👩‍💻\n".repeat(2000);
    let scope = core.store.create_scope(&conversation, &scope_text).unwrap();
    core.store
        .db()
        .execute(
            "UPDATE plan_heads SET revision=revision+1 WHERE plan_id=?1",
            [&tasks[0].plan_id],
        )
        .unwrap();
    core.store
        .bind_plan_scope(&conversation, &tasks[0].plan_id, &scope.id)
        .unwrap();
    let plan = &tasks[0].plan_id;
    for role in [
        "planner",
        ORCHESTRATOR,
        COMMENT_ORCHESTRATOR,
        PLAN_REVIEWER,
        AUDITOR,
    ] {
        let turn = core
            .store
            .start_turn(&conversation, role, "fixture", 0, Some(plan))
            .unwrap();
        let mut args = json!({"task":"baseline"});
        let mut collected = String::new();
        let mut cursor = None;
        loop {
            let page = core
                .agent_tool(&conversation, role, &turn.id, "read_task_contract", &args)
                .unwrap();
            collected.push_str(page["text"].as_str().unwrap());
            if page["complete"] == true {
                break;
            }
            args["cursor"] = page["cursor"].clone();
            cursor = Some(args["cursor"].clone());
        }
        assert_eq!(
            collected,
            crate::lean::prompt_context::markdown(&tasks[0].spec)
        );
        let mut scope_args = json!({});
        let mut scope_body = String::new();
        loop {
            let page = core
                .agent_tool(
                    &conversation,
                    role,
                    &turn.id,
                    "read_plan_context",
                    &scope_args,
                )
                .unwrap();
            assert!(page["text"].as_str().unwrap().len() <= 16 * 1024);
            scope_body.push_str(page["text"].as_str().unwrap());
            if page["complete"] == true {
                break;
            }
            scope_args["cursor"] = page["cursor"].clone();
        }
        assert!(scope_body.contains(&scope_text));
        core.store
            .db()
            .execute(
                "UPDATE plan_heads SET revision=revision+1 WHERE plan_id=?1",
                [plan],
            )
            .unwrap();
        let error = core
            .agent_tool(
                &conversation,
                role,
                &turn.id,
                "read_task_contract",
                &json!({"task":"baseline","cursor":cursor.unwrap()}),
            )
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("revision") && error.contains("without cursor"),
            "{error}"
        );
        assert!(
            core.agent_tool(
                &conversation,
                role,
                &turn.id,
                "read_task_contract",
                &json!({"task":"missing"})
            )
            .is_err()
        );
    }
    let (role, worker) = start(&core, &conversation, &tasks[2], false);
    assert!(
        core.agent_tool(
            &conversation,
            &role,
            &worker.id,
            "read_plan_context",
            &json!({})
        )
        .is_err()
    );
    assert!(
        core.agent_tool(
            &conversation,
            &role,
            &worker.id,
            "read_task_contract",
            &json!({"task":"baseline"})
        )
        .is_err()
    );
    let foreign = core.store.create_conversation("foreign").unwrap();
    let turn = core
        .store
        .start_turn(&foreign.id, ORCHESTRATOR, "fixture", 0, Some(plan))
        .unwrap();
    assert!(
        core.agent_tool(
            &foreign.id,
            ORCHESTRATOR,
            &turn.id,
            "read_task_contract",
            &json!({"task":"baseline"})
        )
        .is_err()
    );
}

#[test]
fn audit_reads_keep_individual_reports_paged_and_never_substitute_aggregate_results() {
    let (_directory, core, conversation, tasks) = fixture();
    let plan = &tasks[0].plan_id;
    let report = "Independent auditor observation ą👩‍💻\n".repeat(2000);
    let seq = core.store.append_event(&conversation, None, Some(plan), "audit", &json!({"approved":false,"report":report,"findings":[{"severity":"medium","issue":"A gap","where":"scope"}]})).unwrap();
    let role = ORCHESTRATOR;
    let turn = core
        .store
        .start_turn(&conversation, role, "fixture", 0, Some(plan))
        .unwrap();
    let mut args = json!({"audit_ref":format!("audit:{seq}")});
    let mut collected = String::new();
    loop {
        let page = core
            .agent_tool(&conversation, role, &turn.id, "read_audit", &args)
            .unwrap();
        assert!(page["verdict"].get("report").is_none());
        assert_eq!(page["verdict"]["findings"][0]["issue"], "A gap");
        collected.push_str(page["report"].as_str().unwrap());
        if page["complete"] == true {
            break;
        }
        args["cursor"] = page["cursor"].clone();
    }
    assert_eq!(collected, report);
    let legacy = core
        .store
        .append_event(
            &conversation,
            None,
            Some(plan),
            "audit",
            &json!({"approved":true}),
        )
        .unwrap();
    core.store
        .add_message(
            &conversation,
            RUNTIME,
            "AGGREGATE PLAN REPORT MUST NOT BECOME AN AUDIT",
        )
        .unwrap();
    let page = core
        .agent_tool(
            &conversation,
            role,
            &turn.id,
            "read_audit",
            &json!({"audit_ref":format!("audit:{legacy}")}),
        )
        .unwrap();
    assert_eq!(page["report_available"], false);
    assert!(!page["report"].as_str().unwrap().contains("AGGREGATE PLAN"));
    let (worker_role, worker) = start(&core, &conversation, &tasks[2], false);
    assert!(
        core.agent_tool(&conversation, &worker_role, &worker.id, "read_audit", &args)
            .is_err()
    );
    let foreign = core.store.create_conversation("foreign").unwrap();
    let foreign_turn = core
        .store
        .start_turn(&foreign.id, role, "fixture", 0, None)
        .unwrap();
    assert!(
        core.agent_tool(&foreign.id, role, &foreign_turn.id, "read_audit", &args)
            .is_err()
    );
}

#[test]
fn paged_carried_report_keeps_exact_generation_and_evidence_without_implying_acceptance() {
    let (_directory, core, conversation, mut tasks) = fixture();
    core.store
        .db()
        .execute(
            "UPDATE task_inputs SET generation=1 WHERE task_id=?1",
            [&tasks[0].id],
        )
        .unwrap();
    tasks[0] = core.store.task_by_id(&tasks[0].id).unwrap().unwrap();
    tasks[0].round = 1;
    core.store.update_task(&tasks[0]).unwrap();
    let (role, producer) = start(&core, &conversation, &tasks[0], false);
    let raw = "raw output é 👩‍💻\n".repeat(2000);
    let recorded = core
        .agent_tool(
            &conversation,
            &role,
            &producer.id,
            "record_evidence",
            &json!({"label":"carry-output","text":raw}),
        )
        .unwrap();
    let evidence = recorded["evidence"]["id"].as_str().unwrap();
    let report = "retained interpretation é 👩‍💻\n".repeat(2000);
    core.agent_tool(
        &conversation,
        &role,
        &producer.id,
        "complete",
        &json!({"summary":"Retained interpretation","report":report}),
    )
    .unwrap();
    core.store
        .finish_turn(&producer.id, store::TurnStatus::Done, None)
        .unwrap();
    tasks[0].status = status::APPROVED.into();
    tasks[0].report = Some(report.clone());
    core.store.update_task(&tasks[0]).unwrap();
    // An older generation's revision cannot establish current source equality.
    core.store
        .db()
        .execute(
            "INSERT INTO task_source_revisions VALUES(?1,0,'unknown-old-revision')",
            [&tasks[0].id],
        )
        .unwrap();
    let scope = core
        .store
        .plan_scope_id(&tasks[0].plan_id)
        .unwrap()
        .unwrap();
    let next = core
        .store
        .add_plan(
            &conversation,
            &tasks.iter().map(|t| t.spec.clone()).collect::<Vec<_>>(),
        )
        .unwrap();
    core.store
        .bind_plan_scope(&conversation, &next[0].plan_id, &scope)
        .unwrap();
    core.store
        .queue_plan(&conversation, &next[0].plan_id)
        .unwrap();
    let carried = core.store.task_by_id(&next[0].id).unwrap().unwrap();
    assert_eq!(carried.status, status::QUEUED);
    assert_eq!(
        carried.feedback.as_ref().unwrap()["carried_from"],
        tasks[0].id
    );
    let event = core
        .store
        .events_since(&conversation, 0, 1000)
        .unwrap()
        .into_iter()
        .find(|e| e.kind == "task_reassessment_required" && e.payload["task"] == carried.id)
        .unwrap();
    assert_eq!(event.payload["source_comparison_unknown"], true);
    let reader = core
        .store
        .start_turn(
            &conversation,
            ORCHESTRATOR,
            "fixture",
            0,
            Some(&next[0].plan_id),
        )
        .unwrap();
    let mut args = json!({"dependency":"baseline"});
    let mut text = String::new();
    loop {
        let page = core
            .agent_tool(
                &conversation,
                ORCHESTRATOR,
                &reader.id,
                "read_dependency_result",
                &args,
            )
            .unwrap();
        assert_eq!(page["result_ref"], producer.id);
        assert_eq!(page["input_generation"], 1);
        assert_eq!(page["task_approved"], false);
        assert_eq!(page["compatibility_reviewed"], false);
        assert_eq!(page["evidence"][0]["id"], evidence);
        text.push_str(page["report"].as_str().unwrap());
        if page["complete"] == true {
            break;
        }
        args["cursor"] = page["cursor"].clone();
    }
    assert_eq!(text, report);
    let mut args = json!({"evidence":evidence});
    let mut text = String::new();
    loop {
        let page = core
            .agent_tool(
                &conversation,
                ORCHESTRATOR,
                &reader.id,
                "read_evidence",
                &args,
            )
            .unwrap();
        text.push_str(page["text"].as_str().unwrap());
        if page["complete"] == true {
            break;
        }
        args["cursor"] = page["cursor"].clone();
    }
    assert_eq!(text, raw);
    assert_eq!(
        core.store.task_by_id(&tasks[0].id).unwrap().unwrap().status,
        status::APPROVED
    );
}
