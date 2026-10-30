#[cfg(test)]
mod tests {
    use super::*;

    fn post(gateway: &Gateway, token: &str, body: Value) -> (u16, Value) {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .into();
        let mut response = match agent
            .post(gateway.url())
            .header("authorization", format!("Bearer {token}"))
            .send_json(body)
        {
            Ok(response) => response,
            Err(error) => panic!("request failed: {error}"),
        };
        let status = response.status().as_u16();
        let text = response.body_mut().read_to_string().unwrap();
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    fn finish(turn: &str, text: &str) -> Value {
        json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
            "name":TOOL,"arguments":{"turn":turn,"text":text}}})
    }

    #[test]
    fn the_agent_ends_exactly_the_open_turn() {
        let gateway = Gateway::start().unwrap();
        let token = gateway.token().to_owned();
        gateway.begin("t1").unwrap();
        let (_, wrong) = post(&gateway, &token, finish("t0", "stale"));
        assert_eq!(wrong["result"]["isError"], true);
        let (_, right) = post(&gateway, &token, finish("t1", "done"));
        assert_eq!(right["result"]["isError"], Value::Null);
        let (_, again) = post(&gateway, &token, finish("t1", "twice"));
        assert_eq!(again["result"]["isError"], true);
        let cancel = AtomicBool::new(false);
        let answer = gateway
            .wait("t1", &cancel, Duration::from_secs(2), || Ok(()))
            .unwrap();
        assert_eq!(answer.text, "done");
    }

    #[test]
    fn requests_without_the_session_token_are_refused() {
        let gateway = Gateway::start().unwrap();
        gateway.begin("t1").unwrap();
        let (status, _) = post(&gateway, "wrong", finish("t1", "x"));
        assert_eq!(status, 401);
        let cancel = AtomicBool::new(false);
        assert!(
            gateway
                .wait("t1", &cancel, Duration::from_millis(300), || Ok(()))
                .is_err()
        );
    }

    #[test]
    fn a_call_between_turns_is_refused() {
        let gateway = Gateway::start().unwrap();
        let token = gateway.token().to_owned();
        let (_, reply) = post(&gateway, &token, finish("t1", "late"));
        assert_eq!(reply["result"]["isError"], true);
    }

    #[test]
    fn handshake_lists_the_single_tool_and_reports_the_codex_thread() {
        let gateway = Gateway::start().unwrap();
        let token = gateway.token().to_owned();
        let (_, init) = post(
            &gateway,
            &token,
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
        );
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert!(!gateway.connected());
        let (_, tools) = post(
            &gateway,
            &token,
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        );
        assert_eq!(tools["result"]["tools"][0]["name"], TOOL);
        assert!(gateway.connected(), "listing the tool shows the CLI is up");
        gateway.begin("t1").unwrap();
        post(
            &gateway,
            &token,
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
                "name":TOOL,"arguments":{"turn":"t1","text":"ok"},
                "_meta":{"threadId":"thread-7"}}}),
        );
        assert_eq!(gateway.session().as_deref(), Some("thread-7"));
    }

    #[test]
    fn the_stdio_bridge_relays_a_whole_turn_for_agy() {
        let gateway = Gateway::start().unwrap();
        gateway.begin("t1").unwrap();
        let input = [
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
            json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{
                "name":TOOL,"arguments":{"turn":"t1","text":"from agy"}}}),
        ]
        .iter()
        .map(|message| format!("{message}\n"))
        .collect::<String>();
        let mut output = Vec::new();
        forward(
            input.as_bytes(),
            &mut output,
            gateway.url(),
            gateway.token(),
            Some("conversation-9"),
        )
        .unwrap();
        let replies = String::from_utf8(output).unwrap();
        let replies: Vec<Value> = replies
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(replies.len(), 3, "the notification gets no reply");
        assert_eq!(replies[1]["result"]["tools"][0]["name"], TOOL);
        assert!(
            replies[2]["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("recorded")
        );
        assert_eq!(gateway.session().as_deref(), Some("conversation-9"));
        let cancel = AtomicBool::new(false);
        let finish = gateway
            .wait("t1", &cancel, Duration::from_secs(2), || Ok(()))
            .unwrap();
        assert_eq!(finish.text, "from agy");
    }

    #[test]
    fn cancellation_and_a_dead_agent_end_the_wait() {
        let gateway = Gateway::start().unwrap();
        gateway.begin("t1").unwrap();
        let cancel = AtomicBool::new(true);
        let error = gateway
            .wait("t1", &cancel, Duration::from_secs(5), || Ok(()))
            .unwrap_err();
        assert_eq!(error.to_string(), CANCELLED);
        gateway.begin("t2").unwrap();
        let cancel = AtomicBool::new(false);
        let error = gateway
            .wait("t2", &cancel, Duration::from_secs(5), || {
                Err(invalid("agent exited"))
            })
            .unwrap_err();
        assert!(error.to_string().contains("agent exited"));
    }
}

#[cfg(test)]
mod role_tests {
    use super::*;

    fn shared(role: &str) -> Shared {
        Shared {
            token: "test".into(),
            state: Mutex::new(State {
                role: role.into(),
                expected: Some("current".into()),
                ..State::default()
            }),
            changed: Condvar::new(),
        }
    }

    #[test]
    fn tools_and_result_schemas_are_role_scoped() {
        for (role, expected) in [
            (
                "worker:task",
                vec!["complete", "select", "ask", "wait_for_answer"],
            ),
            ("verifier:task", vec!["review", "ask", "wait_for_answer"]),
        ] {
            let tools = tool_list(role);
            let names: Vec<_> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
            assert_eq!(names, expected);
            assert!(!allowed_tools(role).contains(&"revise_plan"));
        }
        let review = &tool_list("verifier:task")[0]["inputSchema"];
        assert_eq!(
            review["required"],
            json!(["turn", "report", "checks", "findings"])
        );
        let finding = &review["properties"]["findings"]["items"];
        for derived in ["id", "approved"] {
            assert!(finding["properties"][derived].is_null(), "{derived}");
            assert!(review["properties"][derived].is_null(), "{derived}");
        }
        assert_eq!(
            allowed_tools("orchestrator"),
            vec![
                "finish_turn",
                "ask_user",
                "submit_requirements",
                "submit_plan",
                "answer_agent",
                "defer_to_user",
                "retry_task",
                "revise_plan",
                "request_replan",
            ]
        );
        assert_eq!(allowed_tools("planner"), vec!["submit_plan", "finish_turn"]);
        for reviewer in ["auditor", "plan-verifier", "code-review:t1"] {
            assert_eq!(allowed_tools(reviewer), vec!["review"], "{reviewer}");
            let review = &tool_list(reviewer)[0]["inputSchema"];
            assert_eq!(review["required"], json!(["turn", "report", "findings"]));
            assert!(review["properties"]["checks"].is_null());
        }
        for (role, tool) in [
            ("requirements-extraction-analyst", "submit_ledger"),
            ("requirements-audit-analyst", "submit_audit"),
            ("requirements-audit-verifier", "submit_audit_review"),
        ] {
            assert_eq!(allowed_tools(role), vec![tool], "{role}");
            assert!(is_terminal(tool));
        }
        let entry = &tool_list("requirements-extraction-analyst")[0]["inputSchema"]["properties"]["entries"]
            ["items"];
        assert_eq!(
            entry["required"],
            json!(["id", "quote", "interpretation", "criteria"])
        );
        for gone in [
            "start",
            "end",
            "position",
            "disposition",
            "implementation_owners",
        ] {
            assert!(entry["properties"][gone].is_null(), "{gone}");
        }
        let evidence = &tool_list("requirements-audit-analyst")[0]["inputSchema"]["properties"]["evidence"]
            ["items"];
        for gone in [
            "entry_id",
            "entry_version",
            "generation",
            "integrated_fingerprint",
        ] {
            assert!(evidence["properties"][gone].is_null(), "{gone}");
        }
        assert_eq!(allowed_tools("anyone-else"), vec!["finish_turn"]);
        let finish = &tool_list("orchestrator")[0]["inputSchema"];
        assert_eq!(finish["required"], json!(["turn", "text"]));
        assert!(finish["properties"]["addressed"].is_object());
        assert!(tool_list("auditor")[0]["inputSchema"]["properties"]["addressed"].is_null());
        let task = &schema::revision()["tasks"]["items"];
        assert_eq!(task["required"], json!(["id", "title", "goal", "kind"]));
        for gone in ["inputs", "outputs", "verifier", "requirement_ids"] {
            assert!(task["properties"][gone].is_null(), "{gone}");
        }
        let planner = tool_list("planner");
        assert_eq!(
            planner[0]["inputSchema"]["required"],
            json!(["turn", "document", "tasks"])
        );
    }

    #[test]
    fn a_submitted_plan_ends_the_turn_and_a_repeat_is_acknowledged() {
        let shared = shared("planner");
        shared.state.lock().unwrap().expected = Some("t1".into());
        let call = json!({"name":"submit_plan","arguments":{"turn":"t1","document":"# Plan","tasks":[{"id":"t1","title":"Do","goal":"Do it","kind":"implementation"}]}});
        assert_eq!(dispatch(&shared, &call).unwrap(), json!({"recorded":true}));
        let finish = shared.state.lock().unwrap().finished.clone().unwrap();
        let (text, plan) = super::super::turn_result::TurnResult::plan_answer(&finish.text);
        assert_eq!(text, "# Plan");
        let (tasks, keep) = plan.unwrap();
        assert!(tasks[0].clone().into_contract().writes && keep.is_empty());
        shared.state.lock().unwrap().finished = None;
        assert_eq!(dispatch(&shared, &call).unwrap(), json!({"recorded":true}));
        let mut other = call.clone();
        other["arguments"]["document"] = json!("# Other");
        assert!(dispatch(&shared, &other).is_err());
    }

    #[test]
    fn stale_leases_and_foreign_role_commands_are_rejected_before_dispatch() {
        let shared = shared("worker:task");
        let stale =
            json!({"name":"complete","arguments":{"turn":"old","summary":"done","report":"done"}});
        assert!(
            dispatch(&shared, &stale)
                .unwrap_err()
                .to_string()
                .contains("turn is not open")
        );
        let foreign = json!({"name":"answer_agent","arguments":{"turn":"current","request":"q","answer":"ok"}});
        assert!(
            dispatch(&shared, &foreign)
                .unwrap_err()
                .to_string()
                .contains("unavailable for this role")
        );
        assert!(shared.state.lock().unwrap().finished.is_none());
    }

    #[test]
    fn finish_receipt_is_idempotent_after_the_result_is_consumed() {
        let shared = shared("orchestrator");
        let submission = json!({"name":"finish_turn","arguments":{"turn":"current","text":"done"}});
        assert_eq!(dispatch(&shared, &submission).unwrap()["recorded"], true);
        {
            let mut state = shared.state.lock().unwrap();
            state.finished.take();
            state.expected = None;
        }
        assert_eq!(dispatch(&shared, &submission).unwrap()["recorded"], true);
        let mut conflict = submission;
        conflict["arguments"]["text"] = json!("different");
        let error = dispatch(&shared, &conflict).unwrap_err().to_string();
        assert!(
            error.contains("already ended with your earlier finish_turn call"),
            "{error}"
        );
    }
}

#[cfg(test)]
mod rejection_tests {
    use super::*;

    fn submission(turn: &str) -> Value {
        json!({"name":TOOL,"arguments":{"turn":turn,"text":"done"}})
    }

    #[test]
    fn rejected_finish_can_be_corrected_without_accepting_a_stale_turn() {
        let gateway = Gateway::start().unwrap();
        gateway.begin("current").unwrap();
        gateway
            .set_handler(Arc::new(|_, _| {
                Err(invalid("request: unknown question ID"))
            }))
            .unwrap();
        assert_eq!(call(&gateway.shared, &submission("old"))["isError"], true);
        assert!(
            gateway
                .shared
                .state
                .lock()
                .unwrap()
                .rejected_finish
                .is_none()
        );
        assert_eq!(
            call(&gateway.shared, &submission("current"))["isError"],
            true
        );
        assert!(gateway.shared.state.lock().unwrap().finished.is_none());
        let rejected_at = gateway
            .shared
            .state
            .lock()
            .unwrap()
            .rejected_finish
            .as_ref()
            .unwrap()
            .0;
        call(&gateway.shared, &submission("current"));
        assert_eq!(
            gateway
                .shared
                .state
                .lock()
                .unwrap()
                .rejected_finish
                .as_ref()
                .unwrap()
                .0,
            rejected_at
        );
        gateway
            .set_handler(Arc::new(|_, _| Ok(json!({"recorded":true}))))
            .unwrap();
        call(&gateway.shared, &submission("current"));
        assert_eq!(
            gateway
                .wait(
                    "current",
                    &AtomicBool::new(false),
                    Duration::ZERO,
                    || Ok(())
                )
                .unwrap()
                .text,
            "done"
        );
    }

    #[test]
    fn a_correction_after_sixty_seconds_can_finish_within_the_turn_timeout() {
        let gateway = Gateway::start().unwrap();
        gateway.begin("current").unwrap();
        gateway.shared.state.lock().unwrap().rejected_finish = Some((
            Instant::now() - Duration::from_secs(61),
            "old rejection".into(),
        ));
        let finish = gateway
            .wait(
                "current",
                &AtomicBool::new(false),
                Duration::from_secs(1800),
                || {
                    assert_eq!(
                        call(&gateway.shared, &submission("current"))["isError"],
                        Value::Null
                    );
                    Ok(())
                },
            )
            .unwrap();
        assert_eq!(finish.text, "done");
    }

    #[test]
    fn rejection_keeps_the_turn_timeout_and_resets_for_a_new_turn() {
        let gateway = Gateway::start().unwrap();
        gateway.begin("current").unwrap();
        gateway.shared.state.lock().unwrap().rejected_finish = Some((
            Instant::now() - Duration::from_secs(61),
            "request: unknown question ID".into(),
        ));
        let error = gateway
            .wait(
                "current",
                &AtomicBool::new(false),
                Duration::ZERO,
                || Ok(()),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("request: unknown question ID"));
        assert!(error.contains("no accepted result"), "{error}");
        assert!(gateway.shared.state.lock().unwrap().expected.is_none());
        gateway.begin("next").unwrap();
        assert!(
            gateway
                .shared
                .state
                .lock()
                .unwrap()
                .rejected_finish
                .is_none()
        );
    }

    #[test]
    fn cli_exit_retains_rejection_and_closes_the_turn() {
        let gateway = Gateway::start().unwrap();
        gateway.begin("current").unwrap();
        gateway.shared.state.lock().unwrap().rejected_finish =
            Some((Instant::now(), "bad checkpoint".into()));
        let error = gateway
            .wait(
                "current",
                &AtomicBool::new(false),
                Duration::from_secs(1800),
                || Err(invalid("CLI exited (1)")),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("CLI exited (1)"));
        assert!(error.contains("last rejected result: bad checkpoint"));
        assert!(gateway.shared.state.lock().unwrap().expected.is_none());
    }

    #[test]
    fn an_agent_is_awaiting_an_answer_only_while_its_wait_is_open() {
        let gateway = Gateway::start().unwrap();
        gateway.begin("current").unwrap();
        assert!(!gateway.awaiting_answer());
        gateway.shared.state.lock().unwrap().waiting = Some(Instant::now());
        assert!(gateway.awaiting_answer());
    }

    #[test]
    fn timeout_retains_the_rejection_reason() {
        let gateway = Gateway::start().unwrap();
        gateway.begin("current").unwrap();
        gateway.shared.state.lock().unwrap().rejected_finish =
            Some((Instant::now(), "bad checkpoint".into()));
        let error = gateway
            .wait(
                "current",
                &AtomicBool::new(false),
                Duration::ZERO,
                || Ok(()),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("last rejected submission: bad checkpoint"));
    }
}
