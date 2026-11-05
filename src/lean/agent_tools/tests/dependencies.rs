use super::*;

fn authorize(core: &Core, conversation: &str, plan: &str) {
    core.store
        .db()
        .execute(
            "INSERT INTO plan_activations(plan_id,conversation_id) VALUES(?1,?2)",
            rusqlite::params![plan, conversation],
        )
        .unwrap();
}

#[test]
fn orchestrator_reads_its_accepted_plan_while_newer_rejected_drafts_exist() {
    let (_directory, core, conversation) = core();
    let spec = json!({"id":"inventory","title":"Inventory","goal":"Report","change_kind": "none", "kind":"analysis"});
    let mut source = core
        .store
        .add_plan(&conversation, std::slice::from_ref(&spec))
        .unwrap()
        .remove(0);
    source.status = status::APPROVED.into();
    source.report = Some("ą🦀".repeat(6000));
    core.store.update_task(&source).unwrap();
    core.store
        .test_completion(&source, source.report.as_deref().unwrap());
    authorize(&core, &conversation, &source.plan_id);
    let draft = core
        .store
        .add_plan(&conversation, std::slice::from_ref(&spec))
        .unwrap()
        .remove(0);
    core.store
        .set_plan_status(&draft.plan_id, "proposed", "revised")
        .unwrap();
    let turn = core
        .store
        .start_turn(&conversation, ORCHESTRATOR, "fixture", 0, None)
        .unwrap();
    let page = core
        .agent_tool(
            &conversation,
            ORCHESTRATOR,
            &turn.id,
            "read_dependency_result",
            &json!({"dependency":"inventory"}),
        )
        .unwrap();
    assert_eq!(page["result_id"], source.id);
    // Another plan can be accepted between pages; this turn stays bound.
    let mut next = core
        .store
        .add_plan(&conversation, &[spec])
        .unwrap()
        .remove(0);
    next.status = status::APPROVED.into();
    next.report = Some("new accepted report".into());
    core.store.update_task(&next).unwrap();
    core.store
        .test_completion(&next, next.report.as_deref().unwrap());
    authorize(&core, &conversation, &next.plan_id);
    let second = core
        .agent_tool(
            &conversation,
            ORCHESTRATOR,
            &turn.id,
            "read_dependency_result",
            &json!({"dependency":"inventory","cursor":page["cursor"]}),
        )
        .unwrap();
    assert_eq!(second["result_id"], source.id);
    let fresh = core
        .store
        .start_turn(&conversation, ORCHESTRATOR, "fixture", 0, None)
        .unwrap();
    let latest = core
        .agent_tool(
            &conversation,
            ORCHESTRATOR,
            &fresh.id,
            "read_dependency_result",
            &json!({"dependency":"inventory"}),
        )
        .unwrap();
    assert_eq!(latest["result_id"], next.id);
    let foreign = core.store.create_conversation("foreign").unwrap();
    let foreign_turn = core
        .store
        .start_turn(
            &foreign.id,
            ORCHESTRATOR,
            "fixture",
            0,
            Some(&source.plan_id),
        )
        .unwrap();
    assert!(
        core.agent_tool(
            &foreign.id,
            ORCHESTRATOR,
            &foreign_turn.id,
            "read_dependency_result",
            &json!({"dependency":"inventory"})
        )
        .is_err()
    );
}

#[test]
fn approved_dependency_exposes_complete_earlier_rounds_after_a_carried_amendment() {
    let (_directory, core, conversation) = core();
    let specs = [
        json!({"id":"inventory","title":"Inventory","goal":"Report","change_kind": "none", "kind":"analysis"}),
        json!({"id":"consumer","title":"Consumer","goal":"Read inventory","change_kind": "none", "kind":"analysis","depends_on":["inventory"]}),
    ];
    let mut source = core
        .store
        .add_plan(&conversation, &specs)
        .unwrap()
        .remove(0);
    let full = (1..=16)
        .map(|n| format!("Finding {n}: {}\n", "ą🦀".repeat(600)))
        .collect::<String>();
    let mut original_turn = None;
    for (round, report) in [
        (1, full.clone()),
        (2, "Amendment to round 1: correct row 17.".into()),
    ] {
        source.round = round;
        core.store.update_task(&source).unwrap();
        let turn = core
            .store
            .start_turn(
                &conversation,
                &format!("worker:{}", source.id),
                "fixture",
                0,
                Some(&source.plan_id),
            )
            .unwrap();
        core.store
            .save_turn_result(
                &turn.id,
                &TurnResult::Completed {
                    summary: "Summary".into(),
                    report: report.clone(),
                    selection: None,
                },
            )
            .unwrap();
        core.store
            .finish_turn(&turn.id, store::TurnStatus::Done, None)
            .unwrap();
        if round == 1 {
            original_turn = Some(turn.id);
        }
        source.report = Some(report);
    }
    source.status = status::APPROVED.into();
    core.store.update_task(&source).unwrap();
    let tasks = core.store.add_plan(&conversation, &specs).unwrap();
    let mut carried = tasks[0].clone();
    carried.status = status::APPROVED.into();
    carried.feedback = Some(json!({"carried_from":source.id}));
    core.store.test_result_source(&carried, &source);
    core.store.update_task(&carried).unwrap();
    let role = format!("reviewer:{}", tasks[1].id);
    let turn = core
        .store
        .start_turn(&conversation, &role, "fixture", 0, Some(&tasks[1].plan_id))
        .unwrap();
    // The handoff's full-result producer id is a scoped alias, not a global lookup.
    let mut args = json!({"dependency":source.id});
    let latest = core
        .agent_tool(
            &conversation,
            &role,
            &turn.id,
            "read_dependency_result",
            &args,
        )
        .unwrap();
    assert_eq!(latest["available_rounds"], json!([2, 1]));
    assert!(latest["report"].as_str().unwrap().starts_with("Amendment"));
    args["round"] = 1.into();
    let mut report = String::new();
    let mut saved_cursor = None;
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
        let text = page["report"].as_str().unwrap();
        assert!(text.len() <= 16 * 1024);
        report.push_str(text);
        if page["complete"] == true {
            break;
        }
        saved_cursor.get_or_insert(page["cursor"].clone());
        args["cursor"] = page["cursor"].clone();
    }
    assert_eq!(report, full);
    assert!(report.contains("Finding 16:"));
    assert!(
        core.agent_tool(
            &conversation,
            &role,
            &turn.id,
            "read_dependency_result",
            &json!({"dependency":"inventory","round":2,"cursor":saved_cursor})
        )
        .is_err()
    );
    assert!(
        core.agent_tool(
            &conversation,
            &role,
            &turn.id,
            "read_dependency_result",
            &json!({"dependency":"inventory","round":3})
        )
        .is_err()
    );
    core.store
        .db()
        .execute(
            "UPDATE turn_inputs SET generation=99 WHERE turn_id=?1",
            [original_turn.unwrap()],
        )
        .unwrap();
    assert!(
        core.agent_tool(
            &conversation,
            &role,
            &turn.id,
            "read_dependency_result",
            &json!({"dependency":"inventory","round":1})
        )
        .is_err()
    );
}

#[test]
fn compliance_roles_read_full_text_deliverables_only_in_their_bound_run() {
    let (_directory, core, conversation) = core();
    let run = core.store.create_run(&conversation, "Analyze").unwrap();
    let spec = json!({"id":"analysis","title":"Analysis","goal":"Deliver text","change_kind": "none", "kind":"analysis"});
    let mut task = core
        .store
        .add_plan(&conversation, std::slice::from_ref(&spec))
        .unwrap()
        .remove(0);
    core.store
        .link_plan_run(&conversation, &task.plan_id, &run.id)
        .unwrap();
    let full = (1..=16)
        .map(|i| format!("Problem {i}: {}\n", "ą🦀".repeat(500)))
        .collect::<String>();
    task.report = Some(full.clone());
    task.status = status::APPROVED.into();
    core.store.update_task(&task).unwrap();
    core.store
        .test_completion(&task, task.report.as_deref().unwrap());
    // Same authored id in a newer, unrelated run must not change the result.
    let other_run = core
        .store
        .create_run(&conversation, "Other requirements")
        .unwrap();
    let mut other = core
        .store
        .add_plan(&conversation, &[spec])
        .unwrap()
        .remove(0);
    core.store
        .link_plan_run(&conversation, &other.plan_id, &other_run.id)
        .unwrap();
    other.report = Some("Unrelated report".into());
    other.status = status::APPROVED.into();
    core.store.update_task(&other).unwrap();
    core.store
        .test_completion(&other, other.report.as_deref().unwrap());
    authorize(&core, &conversation, &other.plan_id);
    for role in ["requirements-audit-analyst", "requirements-audit-reviewer"] {
        let turn = core
            .store
            .start_turn(&conversation, role, "fixture", 0, Some(&task.plan_id))
            .unwrap();
        let mut args = json!({"dependency":task.id});
        let mut read = String::new();
        let mut cursor = None;
        loop {
            let page = core
                .agent_tool(
                    &conversation,
                    role,
                    &turn.id,
                    "read_dependency_result",
                    &args,
                )
                .unwrap();
            assert_eq!(page["result_id"], task.id);
            assert!(page["report"].as_str().unwrap().len() <= 16 * 1024);
            read.push_str(page["report"].as_str().unwrap());
            if page["complete"] == true {
                break;
            }
            cursor = Some(page["cursor"].clone());
            args["cursor"] = page["cursor"].clone();
        }
        assert_eq!(read, full);
        assert!(
            core.agent_tool(
                &conversation,
                role,
                &turn.id,
                "read_dependency_result",
                &json!({"dependency":other.id})
            )
            .is_err()
        );
        let next = core
            .store
            .start_turn(&conversation, role, "fixture", 0, Some(&task.plan_id))
            .unwrap();
        assert!(
            core.agent_tool(
                &conversation,
                role,
                &next.id,
                "read_dependency_result",
                &json!({"dependency":task.id,"cursor":cursor})
            )
            .is_err()
        );
        let unbound = core
            .store
            .start_turn(&conversation, role, "fixture", 0, None)
            .unwrap();
        assert!(
            core.agent_tool(
                &conversation,
                role,
                &unbound.id,
                "read_dependency_result",
                &json!({"dependency":task.id})
            )
            .is_err()
        );
        let foreign = core.store.create_conversation("foreign").unwrap();
        let foreign_turn = core
            .store
            .start_turn(&foreign.id, role, "fixture", 0, Some(&task.plan_id))
            .unwrap();
        assert!(
            core.agent_tool(
                &foreign.id,
                role,
                &foreign_turn.id,
                "read_dependency_result",
                &json!({"dependency":task.id})
            )
            .is_err()
        );
    }
    let role = "requirements-extraction-analyst";
    let turn = core
        .store
        .start_turn(&conversation, role, "fixture", 0, Some(&task.plan_id))
        .unwrap();
    assert!(
        core.agent_tool(
            &conversation,
            role,
            &turn.id,
            "read_dependency_result",
            &json!({"dependency":task.id})
        )
        .is_err()
    );
}

#[test]
fn local_options_handoff_alias_reads_the_complete_selection_without_expanding_access() {
    let (_directory, core, conversation) = core();
    let specs = [
        json!({"id":"consumer-options","title":"Options","goal":"Choose","change_kind": "none", "kind":"options","options_for":"consumer"}),
        json!({"id":"unrelated-options","title":"Other options","goal":"Choose","change_kind": "none", "kind":"options","options_for":"other"}),
        json!({"id":"consumer","title":"Consume","goal":"Apply selected approach","change_kind": "none", "kind":"analysis"}),
    ];
    let tasks = core.store.add_plan(&conversation, &specs).unwrap();
    for source in &tasks[..2] {
        let mut source = source.clone();
        source.status = status::APPROVED.into();
        source.round = 1;
        core.store.update_task(&source).unwrap();
        let turn = core
            .store
            .start_turn(
                &conversation,
                &format!("worker:{}", source.id),
                "fixture",
                0,
                Some(&source.plan_id),
            )
            .unwrap();
        core.store.save_turn_result(&turn.id, &TurnResult::Completed {
            summary:"Chose A".into(), report:"Full rationale".into(),
            selection:Some(serde_json::from_value(json!({"selected":"A","rationale":"All details retained","alternatives":[{"id":"A","approach":"Use A","tradeoffs":"Cost A"},{"id":"B","approach":"Use B","tradeoffs":"Cost B"}]})).unwrap())
        }).unwrap();
        core.store
            .finish_turn(&turn.id, store::TurnStatus::Done, None)
            .unwrap();
    }
    for role in [
        format!("worker:{}", tasks[2].id),
        format!("reviewer:{}", tasks[2].id),
    ] {
        let turn = core
            .store
            .start_turn(&conversation, &role, "fixture", 0, Some(&tasks[2].plan_id))
            .unwrap();
        for alias in [
            "selected_approach",
            "consumer-options",
            tasks[0].id.as_str(),
        ] {
            let page = core
                .agent_tool(
                    &conversation,
                    &role,
                    &turn.id,
                    "read_dependency_result",
                    &json!({"dependency":alias}),
                )
                .unwrap();
            assert_eq!(page["result_id"], tasks[0].id);
            assert!(
                page["report"]
                    .as_str()
                    .unwrap()
                    .contains("All details retained")
            );
            assert!(page["report"].as_str().unwrap().contains("Cost B"));
        }
        let error = core
            .agent_tool(
                &conversation,
                &role,
                &turn.id,
                "read_dependency_result",
                &json!({"dependency":tasks[1].id}),
            )
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("available results: consumer-options"),
            "{error}"
        );
        assert!(!error.contains(&format!("Other options ({})", tasks[1].id)));
    }
}

#[test]
fn approved_transitive_results_are_readable_but_siblings_and_foreign_results_are_not() {
    let (_directory, core, conversation) = core();
    let specs = [
        json!({"id":"measurement-path","title":"Measurement","goal":"Report","change_kind": "none", "kind":"analysis"}),
        json!({"id":"baseline","title":"Baseline","goal":"Measure","change_kind": "none", "kind":"analysis","depends_on":["measurement-path"]}),
        json!({"id":"synthesis","title":"Synthesis","goal":"Use measurements","change_kind": "none", "kind":"analysis","depends_on":["baseline"]}),
        json!({"id":"sibling","title":"Sibling","goal":"Unrelated","change_kind": "none", "kind":"analysis"}),
    ];
    let mut tasks = core.store.add_plan(&conversation, &specs).unwrap();
    let report = (1..=16)
        .map(|i| format!("Problem {i}: {}\n", "ą🦀".repeat(600)))
        .collect::<String>();
    for index in [0, 1, 3] {
        tasks[index].status = status::APPROVED.into();
        tasks[index].report = Some(if index == 0 {
            report.clone()
        } else {
            "Unrelated or intermediate report".into()
        });
        core.store.update_task(&tasks[index]).unwrap();
        core.store
            .test_completion(&tasks[index], tasks[index].report.as_deref().unwrap());
    }
    let foreign = core.store.create_conversation("foreign").unwrap();
    let foreign_task = core
        .store
        .add_plan(&foreign.id, &specs[..1])
        .unwrap()
        .remove(0);
    for role in [
        format!("worker:{}", tasks[2].id),
        format!("reviewer:{}", tasks[2].id),
    ] {
        let turn = core
            .store
            .start_turn(&conversation, &role, "fixture", 0, Some(&tasks[2].plan_id))
            .unwrap();
        let mut args = json!({"dependency":"measurement-path"});
        let mut text = String::new();
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
            assert_eq!(page["result_id"], tasks[0].id);
            text.push_str(page["report"].as_str().unwrap());
            if page["complete"] == true {
                break;
            }
            args["cursor"] = page["cursor"].clone();
        }
        assert_eq!(text, report);
        for denied in [tasks[3].id.as_str(), foreign_task.id.as_str()] {
            assert!(
                core.agent_tool(
                    &conversation,
                    &role,
                    &turn.id,
                    "read_dependency_result",
                    &json!({"dependency":denied})
                )
                .is_err()
            );
        }
    }
    tasks[0].status = status::QUEUED.into();
    core.store.update_task(&tasks[0]).unwrap();
    let role = format!("worker:{}", tasks[2].id);
    let turn = core
        .store
        .start_turn(&conversation, &role, "fixture", 0, Some(&tasks[2].plan_id))
        .unwrap();
    assert!(
        core.agent_tool(
            &conversation,
            &role,
            &turn.id,
            "read_dependency_result",
            &json!({"dependency":"measurement-path"})
        )
        .unwrap_err()
        .to_string()
        .contains("not approved")
    );
}
