use super::*;

fn authorize(core: &Core, conversation: &str, plan: &str) {
    core.store.db().execute("INSERT INTO plan_authorizations(plan_id,conversation_id,scope,write_scope) VALUES(?1,?2,'[\".\"]','[]')",rusqlite::params![plan,conversation]).unwrap();
}

#[test]
fn orchestrator_reads_its_accepted_plan_while_newer_rejected_drafts_exist() {
    let (_directory, core, conversation) = core();
    let spec = json!({"id":"inventory","title":"Inventory","goal":"Report","kind":"analysis"});
    let mut source = core
        .store
        .add_plan(&conversation, std::slice::from_ref(&spec))
        .unwrap()
        .remove(0);
    source.status = status::APPROVED.into();
    source.report = Some("ą🦀".repeat(6000));
    core.store.update_task(&source).unwrap();
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
        json!({"id":"inventory","title":"Inventory","goal":"Report","kind":"analysis"}),
        json!({"id":"consumer","title":"Consumer","goal":"Read inventory","kind":"analysis","depends_on":["inventory"]}),
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
    core.store.update_task(&carried).unwrap();
    let role = format!("verifier:{}", tasks[1].id);
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
