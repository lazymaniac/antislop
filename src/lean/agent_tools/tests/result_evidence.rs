use super::*;

fn fixture() -> (tempfile::TempDir, Arc<Core>, String, Vec<Task>) {
    let (directory, core, conversation) = core();
    let message = core
        .store
        .add_message(
            &conversation,
            "user",
            "Retain complete evidence and amend the report.",
        )
        .unwrap();
    let run = core.store.create_run(&conversation, &message.text).unwrap();
    let source = core
        .store
        .register_message_source(&conversation, &run.id, message.seq)
        .unwrap();
    let change = serde_json::from_value(json!({"conversation":conversation,"run_id":run.id,"expected_spec_revision":0,"operation_id":"evidence-fixture","reason":"Synthetic evidence scope","entries":[{"id":"R1","source":{"source_id":source.id,"start":0,"end":message.text.len()},"interpretation":message.text,"criteria":[{"id":"R1.accept","text":"Retain evidence"}]}]})).unwrap();
    core.store
        .apply_ledger_change(&change, &core.state, true)
        .unwrap();
    let tasks = core.store.add_plan(&conversation,&[
        json!({"id":"baseline","title":"Baseline","goal":"Measure and report","kind":"analysis","requirement_ids":["R1"],"criterion_ids":["R1.accept"]}),
        json!({"id":"consumer","title":"Consumer","goal":"Read complete evidence","kind":"analysis","depends_on":["baseline"],"requirement_ids":["R1"],"criterion_ids":["R1.accept"]}),
        json!({"id":"sibling","title":"Sibling","goal":"Other report","kind":"analysis","requirement_ids":["R1"],"criterion_ids":["R1.accept"]}),
    ]).unwrap();
    core.store
        .link_plan_run(&conversation, &tasks[0].plan_id, &run.id)
        .unwrap();
    core.store
        .bind_task_ledger(&conversation, &run.id, &tasks[0].plan_id)
        .unwrap();
    (directory, core, conversation, tasks)
}

fn start(core: &Core, conversation: &str, task: &Task, reviewer: bool) -> (String, store::Turn) {
    let role = format!(
        "{}:{}",
        if reviewer { "reviewer" } else { "worker" },
        task.id
    );
    let turn = core
        .store
        .start_turn(conversation, &role, "fixture", 0, Some(&task.plan_id))
        .unwrap();
    (role, turn)
}

#[test]
fn old_input_report_and_raw_evidence_survive_amendment_replan_and_reopen() {
    let (_directory, core, conversation, mut tasks) = fixture();
    let raw = "actual unit output ą🦀\n".repeat(2500);
    let report = (1..=16)
        .map(|n| format!("Finding {n}: {}\n", "ą🦀".repeat(800)))
        .collect::<String>();
    tasks[0].round = 1;
    core.store.update_task(&tasks[0]).unwrap();
    let (role, original) = start(&core, &conversation, &tasks[0], false);
    let recorded = core
        .agent_tool(
            &conversation,
            &role,
            &original.id,
            "record_evidence",
            &json!({"label":"units.stdout","text":raw}),
        )
        .unwrap();
    let id = recorded["evidence"]["id"].as_str().unwrap();
    let completion = core
        .agent_tool(
            &conversation,
            &role,
            &original.id,
            "complete",
            &json!({"summary":"Sixteen findings","report":report}),
        )
        .unwrap();
    assert!(
        completion["result"]["report"]
            .as_str()
            .unwrap()
            .contains(id)
    );
    assert!(
        !completion["result"]["report"]
            .as_str()
            .unwrap()
            .contains("actual unit output")
    );
    core.store
        .finish_turn(&original.id, store::TurnStatus::Done, None)
        .unwrap();
    core.store
        .db()
        .execute(
            "UPDATE task_inputs SET generation=1 WHERE task_id=?1",
            [&tasks[0].id],
        )
        .unwrap();
    tasks[0] = core.store.task_by_id(&tasks[0].id).unwrap().unwrap();
    tasks[0].round = 2;
    tasks[0].report = None;
    core.store.update_task(&tasks[0]).unwrap();
    let (role, amendment) = start(&core, &conversation, &tasks[0], false);
    let empty = core
        .agent_tool(
            &conversation,
            &role,
            &amendment.id,
            "read_dependency_result",
            &json!({"dependency":"self"}),
        )
        .unwrap();
    assert_eq!(empty["state"], "no_current_result");
    assert_eq!(empty["report_history"][0]["result_ref"], original.id);
    core.agent_tool(&conversation,&role,&amendment.id,"complete",&json!({"summary":"Amended one finding","report":"Correction to finding 9; original is retained.","evidence":[id]})).unwrap();
    core.store
        .finish_turn(&amendment.id, store::TurnStatus::Done, None)
        .unwrap();
    tasks[0].status = status::APPROVED.into();
    core.store.update_task(&tasks[0]).unwrap();
    // Carry the result into a new plan; the original producer stays immutable.
    let specs = tasks.iter().map(|t| t.spec.clone()).collect::<Vec<_>>();
    let new_tasks = core.store.add_plan(&conversation, &specs).unwrap();
    let mut carried = new_tasks[0].clone();
    carried.status = status::APPROVED.into();
    carried.feedback = Some(json!({"carried_from":tasks[0].id}));
    core.store.update_task(&carried).unwrap();
    for reviewer in [false, true] {
        let (role, reader) = start(&core, &conversation, &new_tasks[1], reviewer);
        let latest = core
            .agent_tool(
                &conversation,
                &role,
                &reader.id,
                "read_dependency_result",
                &json!({"dependency":"baseline"}),
            )
            .unwrap();
        assert_eq!(latest["available_rounds"], json!([2]));
        assert_eq!(latest["historical"], false);
        assert_eq!(latest["report_history"][1]["input_generation"], 0);
        assert_eq!(latest["evidence"][0]["id"], id);
        let mut args = json!({"dependency":"baseline","result_ref":original.id});
        let mut full = String::new();
        loop {
            let page = core
                .agent_tool(
                    &conversation,
                    &role,
                    &reader.id,
                    "read_dependency_result",
                    &args,
                )
                .unwrap();
            assert_eq!(page["historical"], true);
            let text = page["report"].as_str().unwrap();
            assert!(text.len() <= 16 * 1024);
            full.push_str(text);
            if page["complete"] == true {
                break;
            }
            args["cursor"] = page["cursor"].clone();
        }
        assert!(full.starts_with(&report));
        assert!(full.contains("Finding 16:"));
        assert!(
            core.agent_tool(
                &conversation,
                &role,
                &reader.id,
                "read_dependency_result",
                &json!({"dependency":"baseline","round":1})
            )
            .is_err()
        );
        assert!(
            core.agent_tool(
                &conversation,
                &role,
                &reader.id,
                "read_dependency_result",
                &json!({"dependency":"baseline","result_ref":original.id,"round":2})
            )
            .is_err()
        );
        let mut args = json!({"evidence":id});
        let mut full = String::new();
        loop {
            let page = core
                .agent_tool(&conversation, &role, &reader.id, "read_evidence", &args)
                .unwrap();
            let text = page["text"].as_str().unwrap();
            assert!(text.len() <= 16 * 1024);
            full.push_str(text);
            if page["complete"] == true {
                break;
            }
            args["cursor"] = page["cursor"].clone();
        }
        assert_eq!(full, raw);
        let (_, next) = start(&core, &conversation, &new_tasks[1], reviewer);
        assert!(
            core.agent_tool(&conversation, &role, &next.id, "read_evidence", &args)
                .is_err()
        );
        if reviewer {
            assert!(
                core.agent_tool(
                    &conversation,
                    &role,
                    &reader.id,
                    "record_evidence",
                    &json!({"label":"forbidden","text":"x"})
                )
                .is_err()
            );
        }
    }
    let reopened = store::Store::open(&core.state.join("core.db")).unwrap();
    let evidence = reopened.evidence(&conversation, id).unwrap();
    assert_eq!(reopened.evidence_text(&evidence).unwrap(), raw);
    assert_eq!(reopened.result_evidence(&amendment.id).unwrap()[0].id, id);
    core.store
        .db()
        .execute(
            "UPDATE turn_inputs SET generation=99 WHERE turn_id=?1",
            [&original.id],
        )
        .unwrap();
    let (role, reader) = start(&core, &conversation, &new_tasks[1], true);
    assert!(
        core.agent_tool(
            &conversation,
            &role,
            &reader.id,
            "read_dependency_result",
            &json!({"dependency":"baseline","result_ref":original.id})
        )
        .is_err()
    );
}

#[test]
fn evidence_is_immutable_scoped_and_not_a_filesystem_or_history_backdoor() {
    let (_directory, core, conversation, tasks) = fixture();
    let (role, producer) = start(&core, &conversation, &tasks[0], false);
    let args = json!({"label":"run.exit","text":"exit=137; signal unknown\ną🦀"});
    let first = core
        .agent_tool(&conversation, &role, &producer.id, "record_evidence", &args)
        .unwrap();
    assert_eq!(
        first,
        core.agent_tool(&conversation, &role, &producer.id, "record_evidence", &args)
            .unwrap()
    );
    let id = first["evidence"]["id"].as_str().unwrap();
    assert!(
        core.agent_tool(
            &conversation,
            &role,
            &producer.id,
            "record_evidence",
            &json!({"label":"run.exit","text":"exit=0"})
        )
        .is_err()
    );
    assert!(
        core.agent_tool(
            &conversation,
            &role,
            &producer.id,
            "record_evidence",
            &json!({"label":"large","text":"a".repeat(262145)})
        )
        .is_err()
    );
    let (other, reader) = start(&core, &conversation, &tasks[2], false);
    assert_eq!(
        core.agent_tool(
            &conversation,
            &other,
            &reader.id,
            "read_evidence",
            &json!({"evidence":id})
        )
        .unwrap()["text"],
        args["text"]
    );
    assert_eq!(
        core.agent_tool(
            &conversation,
            &other,
            &reader.id,
            "complete",
            &json!({"summary":"Done","report":"Other","evidence":[id]})
        )
        .unwrap()["recorded"],
        true
    );
    assert!(
        core.agent_tool(
            &conversation,
            &other,
            &reader.id,
            "read_dependency_result",
            &json!({"dependency":"sibling","result_ref":producer.id})
        )
        .is_err()
    );
    let foreign = core.store.create_conversation("foreign").unwrap();
    assert!(core.store.evidence(&foreign.id, id).is_err());
    for value in ["../../core.db", "/tmp/log"] {
        assert!(
            core.agent_tool(
                &conversation,
                &role,
                &producer.id,
                "read_evidence",
                &json!({"evidence":value})
            )
            .is_err()
        );
    }
    let metadata: String = core
        .store
        .db()
        .query_row(
            "SELECT group_concat(label||sha256) FROM agent_evidence",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(!metadata.contains("exit=137"));
    let mut changed = args["text"].as_str().unwrap().as_bytes().to_vec();
    changed[0] = b'E';
    std::fs::write(core.state.join("session-content").join(id), changed).unwrap();
    let error = core
        .agent_tool(
            &conversation,
            &role,
            &producer.id,
            "read_evidence",
            &json!({"evidence":id}),
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("checksum changed"), "{error}");
}

#[test]
fn evidence_tools_are_nonterminal_and_completion_links_the_wire_record() {
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;
    let (_directory, core, conversation, tasks) = fixture();
    let (role, turn) = start(&core, &conversation, &tasks[0], false);
    let gateway = mcp::Gateway::start().unwrap();
    gateway.set_role(&role).unwrap();
    gateway
        .set_task_contract(serde_json::from_value(tasks[0].spec.clone()).unwrap())
        .unwrap();
    gateway.begin(&turn.id).unwrap();
    let bound_core = core.clone();
    let bound_conversation = conversation.clone();
    let bound_turn = turn.id.clone();
    gateway
        .set_handler(Arc::new(move |tool, args| {
            bound_core.agent_tool(&bound_conversation, &role, &bound_turn, tool, args)
        }))
        .unwrap();
    let call = |tool: &str, args: Value| {
        let mut response=ureq::post(gateway.url()).header("authorization",format!("Bearer {}",gateway.token())).send_json(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":tool,"arguments":args}})).unwrap();
        let reply: Value = response.body_mut().read_json().unwrap();
        assert_ne!(reply["result"]["isError"], true, "{reply}");
        serde_json::from_str::<Value>(reply["result"]["content"][0]["text"].as_str().unwrap())
            .unwrap()
    };
    let recorded = call(
        "record_evidence",
        json!({"turn":turn.id,"label":"measurement.stdout","text":"planned=6 completed=6 exit=0\n".repeat(3000)}),
    );
    assert_eq!(recorded["recorded"], true);
    assert_eq!(call("current_turn", json!({}))["active"], true);
    let id = recorded["evidence"]["id"].as_str().unwrap();
    assert_eq!(
        call("read_evidence", json!({"turn":turn.id,"evidence":id}))["complete"],
        false
    );
    let complete =
        json!({"turn":turn.id,"summary":"Measured","report":"Read immutable output references."});
    assert_eq!(call("complete", complete.clone())["recorded"], true);
    assert_eq!(call("complete", complete)["recorded"], true);
    let finish = gateway
        .wait(
            &turn.id,
            &AtomicBool::new(false),
            Duration::from_secs(2),
            || Ok(()),
        )
        .unwrap();
    let result: TurnResult = serde_json::from_str(&finish.text).unwrap();
    assert!(result.text().contains(id));
    assert_eq!(core.store.result_evidence(&turn.id).unwrap()[0].id, id);
    let spec: TaskContract = serde_json::from_value(tasks[0].spec.clone()).unwrap();
    let prompt = tasks::reviewer_prompt(".", &spec, &result.text(), None);
    assert!(prompt.contains("read_evidence"));
    assert!(prompt.contains("not whether it was pasted inline"));
}

#[test]
fn a_new_producers_amendment_does_not_hide_the_carried_original() {
    let (_directory, core, conversation, mut tasks) = fixture();
    tasks[0].round = 1;
    core.store.update_task(&tasks[0]).unwrap();
    let (_, original) = start(&core, &conversation, &tasks[0], false);
    core.store
        .save_turn_result(
            &original.id,
            &TurnResult::Completed {
                summary: "Original".into(),
                report: "Original complete report retained across producers.".into(),
                selection: None,
            },
        )
        .unwrap();
    core.store
        .finish_turn(&original.id, store::TurnStatus::Done, None)
        .unwrap();
    tasks[0].status = status::APPROVED.into();
    core.store.update_task(&tasks[0]).unwrap();
    let specs = tasks.iter().map(|t| t.spec.clone()).collect::<Vec<_>>();
    let mut next = core.store.add_plan(&conversation, &specs).unwrap();
    next[0].round = 1;
    next[0].feedback = Some(json!({"carried_from":tasks[0].id}));
    core.store.update_task(&next[0]).unwrap();
    let (_, amendment) = start(&core, &conversation, &next[0], false);
    core.store
        .save_turn_result(
            &amendment.id,
            &TurnResult::Completed {
                summary: "Correction".into(),
                report: "Amendment from a new producer.".into(),
                selection: None,
            },
        )
        .unwrap();
    core.store
        .finish_turn(&amendment.id, store::TurnStatus::Done, None)
        .unwrap();
    next[0].status = status::APPROVED.into();
    core.store.update_task(&next[0]).unwrap();
    let (role, reader) = start(&core, &conversation, &next[1], true);
    let latest = core
        .agent_tool(
            &conversation,
            &role,
            &reader.id,
            "read_dependency_result",
            &json!({"dependency":"baseline"}),
        )
        .unwrap();
    assert_eq!(latest["report"], "Amendment from a new producer.");
    assert_eq!(latest["report_history"].as_array().unwrap().len(), 2);
    let old = core
        .agent_tool(
            &conversation,
            &role,
            &reader.id,
            "read_dependency_result",
            &json!({"dependency":"baseline","result_ref":original.id}),
        )
        .unwrap();
    assert_eq!(
        old["report"],
        "Original complete report retained across producers."
    );
    assert_eq!(old["historical"], true);
    assert_eq!(old["report_producer"], tasks[0].id);
}

#[test]
fn reference_diagnostics_distinguish_typos_wrong_dependency_and_reference_kind() {
    let malformed = crate::lean::agent_tools::references::reference_error(
        "evidence",
        &"ą\n".repeat(5000),
        "invalid reference",
        &[],
    )
    .to_string();
    assert!(
        malformed.len() < 400 && !malformed.contains('\n'),
        "{malformed}"
    );
    let (_directory, core, conversation, mut tasks) = fixture();
    let (role, producer) = start(&core, &conversation, &tasks[0], false);
    let response = core
        .agent_tool(
            &conversation,
            &role,
            &producer.id,
            "record_evidence",
            &json!({"label":"bounded-output","text":"synthetic raw output"}),
        )
        .unwrap();
    let id = response["evidence"]["id"].as_str().unwrap();
    let mut typo = id.to_owned();
    typo.replace_range(0..1, if &id[..1] == "a" { "b" } else { "a" });
    let error = core
        .agent_tool(
            &conversation,
            &role,
            &producer.id,
            "read_evidence",
            &json!({"evidence":typo}),
        )
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("possible typo")
            && error.contains(&format!("requested {typo:?}"))
            && error.contains(id)
            && error.contains("nothing was substituted"),
        "{error}"
    );
    for (index, turn) in [
        (0, producer.clone()),
        (2, start(&core, &conversation, &tasks[2], false).1),
    ] {
        core.store
            .save_turn_result(
                &turn.id,
                &TurnResult::Completed {
                    summary: "Done".into(),
                    report: format!("report {index}"),
                    selection: None,
                },
            )
            .unwrap();
        core.store
            .finish_turn(&turn.id, store::TurnStatus::Done, None)
            .unwrap();
        tasks[index].status = status::APPROVED.into();
        core.store.update_task(&tasks[index]).unwrap();
    }
    let sibling = core
        .store
        .last_turn(&conversation, &format!("worker:{}", tasks[2].id))
        .unwrap()
        .unwrap();
    let (role, reader) = start(&core, &conversation, &tasks[1], true);
    let error = core
        .agent_tool(
            &conversation,
            &role,
            &reader.id,
            "read_dependency_result",
            &json!({"dependency":"baseline","result_ref":sibling.id}),
        )
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("wrong dependency") && error.contains(&producer.id),
        "{error}"
    );
    assert!(!error.contains("report 2"));
    let error = core
        .agent_tool(
            &conversation,
            &role,
            &reader.id,
            "read_evidence",
            &json!({"evidence":producer.id}),
        )
        .unwrap_err()
        .to_string();
    assert!(error.contains("this is a report reference"), "{error}");
}

#[test]
fn evidence_context_survives_an_amendment_that_omits_its_execution_references() {
    let (_directory, core, conversation, mut tasks) = fixture();
    tasks[0].round = 1;
    core.store.update_task(&tasks[0]).unwrap();
    let (role, original) = start(&core, &conversation, &tasks[0], false);
    let recorded = core
        .agent_tool(
            &conversation,
            &role,
            &original.id,
            "record_evidence",
            &json!({"label":"post-fix-success","text":"immutable raw output: exit=0"}),
        )
        .unwrap();
    let id = recorded["evidence"]["id"].as_str().unwrap();
    core.agent_tool(
        &conversation,
        &role,
        &original.id,
        "complete",
        &json!({"summary":"Executed","report":"Success"}),
    )
    .unwrap();
    core.store
        .finish_turn(&original.id, store::TurnStatus::Done, None)
        .unwrap();
    tasks[0].round = 2;
    core.store.update_task(&tasks[0]).unwrap();
    let (role, amendment) = start(&core, &conversation, &tasks[0], false);
    // Reproduce an amended report that accidentally loses its earlier links.
    core.agent_tool(&conversation, &role, &amendment.id, "complete", &json!({"summary":"Fixture corrected","report":"Only the test fixture changed; no new trial."})).unwrap();
    core.store
        .finish_turn(&amendment.id, store::TurnStatus::Done, None)
        .unwrap();
    assert!(
        core.store
            .result_evidence(&amendment.id)
            .unwrap()
            .is_empty()
    );
    let manifest = core.store.task_evidence_context(&tasks[0].id, 0).unwrap();
    assert_eq!(manifest.len(), 1);
    assert_eq!(manifest[0].id, id);
    assert!(
        !serde_json::to_string(&manifest)
            .unwrap()
            .contains("immutable raw output")
    );
    assert!(
        core.store
            .task_evidence_context(&tasks[1].id, 0)
            .unwrap()
            .is_empty()
    );
    assert!(
        core.store
            .task_evidence_context(&tasks[0].id, 1)
            .unwrap()
            .is_empty()
    );
    // Keep prompt cost bounded even when a task has many command records.
    tasks[0].round = 3;
    core.store.update_task(&tasks[0]).unwrap();
    let (_, many) = start(&core, &conversation, &tasks[0], false);
    for i in 0..64 {
        core.store
            .record_evidence(&many.id, &format!("check-{i}"), "raw")
            .unwrap();
    }
    core.store
        .finish_turn(&many.id, store::TurnStatus::Done, None)
        .unwrap();
    let manifest = core.store.task_evidence_context(&tasks[0].id, 0).unwrap();
    assert_eq!(manifest.len(), 64);
    assert!(manifest.iter().all(|e| e.producer_turn == many.id));
}

#[test]
fn blocked_worker_history_remains_readable_without_becoming_a_completed_result() {
    let (_directory, core, conversation, mut tasks) = fixture();
    tasks[0].round = 1;
    core.store.update_task(&tasks[0]).unwrap();
    let (role, blocked) = start(&core, &conversation, &tasks[0], false);
    let report = "Preserved failed qualification; equivalence not established. ą🦀\n".repeat(700);
    core.agent_tool(&conversation, &role, &blocked.id, "report_blocker", &json!({
        "report":report, "resume_condition":"Read the preserved record before continuing analysis"
    })).unwrap();
    core.store
        .finish_turn(&blocked.id, store::TurnStatus::Done, None)
        .unwrap();
    tasks[0].report = Some(report.clone());
    core.store.update_task(&tasks[0]).unwrap();
    assert!(core.store.completed_task_result(&tasks[0]).is_err());

    // Coordination needs the blocked producer's full result without approving it.
    for coordinator in [ORCHESTRATOR, COMMENT_ORCHESTRATOR] {
        let turn = core
            .store
            .start_turn(
                &conversation,
                coordinator,
                "fixture",
                0,
                Some(&tasks[0].plan_id),
            )
            .unwrap();
        let page = core
            .agent_tool(
                &conversation,
                coordinator,
                &turn.id,
                "read_dependency_result",
                &json!({"dependency":"baseline"}),
            )
            .unwrap();
        assert_eq!(page["result_type"], "blocked");
        assert_eq!(page["task_approved"], false);
        assert_eq!(page["task_status"], tasks[0].status);
        assert_eq!(page["result_ref"], blocked.id);
        assert!(
            page["report"]
                .as_str()
                .unwrap()
                .starts_with("Preserved failed qualification")
        );
        core.store
            .finish_turn(&turn.id, store::TurnStatus::Done, None)
            .unwrap();
    }
    // Both the next worker and its reviewer need the full record, not just a handoff.
    tasks[0].round = 2;
    core.store.update_task(&tasks[0]).unwrap();
    for reviewer in [false, true] {
        let (role, reader) = start(&core, &conversation, &tasks[0], reviewer);
        let mut args = json!({"dependency":"self"});
        let mut full = String::new();
        loop {
            let page = core
                .agent_tool(
                    &conversation,
                    &role,
                    &reader.id,
                    "read_dependency_result",
                    &args,
                )
                .unwrap();
            assert_eq!(page["result_type"], "blocked");
            assert_eq!(page["available_rounds"], json!([]));
            assert_eq!(page["report_history"][0]["result_ref"], blocked.id);
            assert_eq!(page["report_history"][0]["result_type"], "blocked");
            full.push_str(page["report"].as_str().unwrap());
            if page["complete"] == true {
                break;
            }
            args["cursor"] = page["cursor"].clone();
        }
        assert_eq!(full, report);
        core.store
            .finish_turn(&reader.id, store::TurnStatus::Done, None)
            .unwrap();
    }

    core.store
        .db()
        .execute(
            "UPDATE task_inputs SET generation=1 WHERE task_id=?1",
            [&tasks[0].id],
        )
        .unwrap();
    let fresh = core.store.task_by_id(&tasks[0].id).unwrap().unwrap();
    let (role, reader) = start(&core, &conversation, &fresh, false);
    let history = core
        .agent_tool(
            &conversation,
            &role,
            &reader.id,
            "read_dependency_result",
            &json!({"dependency":"self"}),
        )
        .unwrap();
    assert_eq!(history["state"], "no_current_result");
    assert_eq!(history["report_history"][0]["result_ref"], blocked.id);
    let old = core
        .agent_tool(
            &conversation,
            &role,
            &reader.id,
            "read_dependency_result",
            &json!({"dependency":"self", "result_ref":blocked.id}),
        )
        .unwrap();
    assert_eq!(old["historical"], true);
    assert_eq!(old["result_type"], "blocked");
    assert!(core.store.completed_task_result(&fresh).is_err());
    let (role, consumer) = start(&core, &conversation, &tasks[1], false);
    assert!(
        core.agent_tool(
            &conversation,
            &role,
            &consumer.id,
            "read_dependency_result",
            &json!({"dependency":"baseline", "result_ref":blocked.id})
        )
        .is_err()
    );
}
