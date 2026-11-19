use super::*;
use crate::lean::mcp::gateway::{Shared, State};
use std::sync::{Arc, Condvar, Mutex};

fn shared() -> Shared {
    Shared {
        token: "fixture".into(),
        state: Mutex::new(State {
            role: "worker:task".into(),
            expected: Some("current-turn".into()),
            ..State::default()
        }),
        changed: Condvar::new(),
    }
}

fn data(reply: Value) -> Value {
    assert!(reply["isError"].is_null(), "{reply}");
    serde_json::from_str(reply["content"][0]["text"].as_str().unwrap()).unwrap()
}

#[test]
fn turn_discovery_recovers_a_stale_result_without_accepting_it_or_exposing_its_body() {
    let shared = shared();
    let logged = Arc::new(Mutex::new(vec![]));
    let copy = logged.clone();
    shared.state.lock().unwrap().protocol_observer = Some(Arc::new(move |event| {
        copy.lock().unwrap().push(event.clone());
        Ok(())
    }));
    let stale = json!({"name":"complete","arguments":{"turn":"previous-turn","summary":"Done","report":"private report content"}});
    let refused = call(&shared, &stale);
    assert_eq!(refused["isError"], true);
    assert!(
        refused["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("current_turn")
    );
    assert!(shared.state.lock().unwrap().finished.is_none());
    let entries = logged.lock().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["submitted_turn"], "previous-turn");
    assert_eq!(entries[0]["expected_turn"], "current-turn");
    assert!(!entries[0].to_string().contains("private report content"));
    drop(entries);
    let lease = data(call(
        &shared,
        &json!({"name":"current_turn","arguments":{}}),
    ));
    assert_eq!(lease["turn"], "current-turn");
    assert_eq!(lease["role"], "worker:task");
    assert_eq!(lease["active"], true);
    assert!(shared.state.lock().unwrap().finished.is_none());
    let accepted = data(call(
        &shared,
        &json!({"name":"complete","arguments":{"turn":lease["turn"],"summary":"Done","report":"Actual result"}}),
    ));
    assert_eq!(accepted["recorded"], true);
    assert!(shared.state.lock().unwrap().protocol_rejected_at.is_none());
    assert_eq!(
        data(call(
            &shared,
            &json!({"name":"current_turn","arguments":{}})
        ))["active"],
        false
    );
}

#[test]
fn a_cancelled_ask_response_cannot_mutate_the_next_turns_wait_state() {
    let shared = shared();
    let started = Instant::now();
    {
        let mut state = shared.state.lock().unwrap();
        state.expected = Some("next-turn".into());
        state.waiting = Some(started);
    }
    assert!(
        finish_dispatch(
            &shared,
            ASK,
            "current-turn",
            &json!({}),
            Ok(json!({"state":"answered"}))
        )
        .is_err()
    );
    let state = shared.state.lock().unwrap();
    assert_eq!(state.expected.as_deref(), Some("next-turn"));
    assert_eq!(state.waiting, Some(started));
    assert!(state.finished.is_none());
}

#[test]
fn a_contract_error_does_not_suggest_replacing_the_correct_turn_id() {
    let shared = shared();
    let reply = call(
        &shared,
        &json!({"name":"select","arguments":{
            "turn":"current-turn","alternatives":[],"selected":"missing","rationale":"No valid choice"
        }}),
    );
    assert_eq!(reply["isError"], true);
    let reason = reply["content"][0]["text"].as_str().unwrap();
    assert!(!reason.contains("current_turn"), "{reason}");
    let state = shared.state.lock().unwrap();
    assert!(state.protocol_rejected_at.is_none());
    assert!(state.finished.is_none());
}

#[test]
fn a_refused_non_terminal_call_names_the_exact_current_turn() {
    let shared = shared();
    let refused = call(
        &shared,
        &json!({"name":"read_evidence","arguments":{"turn":"current-turn-typo","evidence":"x"}}),
    );
    assert_eq!(refused["isError"], true);
    let text = refused["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("turn ID mismatch"), "{text}");
    assert!(
        text.contains("the current turn is \"current-turn\""),
        "{text}"
    );
}

#[test]
fn audit_and_plan_review_can_recover_a_mistyped_turn_and_record_their_findings() {
    let active = "ef09f7e4-d5fb-4bbb-b7bf-5f841eb5170c";
    let mistyped = "ef09f7e4-d5fb-4bbb-b7f5-5f841eb5170c";
    for role in ["auditor", "plan-reviewer", "code-review:t1"] {
        let shared = shared();
        {
            let mut state = shared.state.lock().unwrap();
            state.role = role.into();
            state.expected = Some(active.into());
        }
        let mut review = json!({"name":"review","arguments":{
            "turn":mistyped,"report":"One high finding remains open.",
            "findings":[{"severity":"high","issue":"Quality tasks contradict the selected rigor.","where":"plan"}]
        }});
        let refused = call(&shared, &review);
        assert_eq!(refused["isError"], true);
        let error = refused["content"][0]["text"].as_str().unwrap();
        assert!(error.contains("turn ID mismatch"), "{error}");
        assert!(
            error.contains(mistyped) && error.contains(active),
            "{error}"
        );
        assert!(error.contains("current_turn"), "{error}");
        assert!(shared.state.lock().unwrap().finished.is_none());
        let current = data(call(
            &shared,
            &json!({"name":"current_turn","arguments":{}}),
        ));
        assert_eq!(current["active"], true);
        assert_eq!(current["turn"], active);
        review["arguments"]["turn"] = current["turn"].clone();
        assert_eq!(data(call(&shared, &review))["recorded"], true);
        let result = shared.state.lock().unwrap().finished.clone().unwrap();
        let (report, verdict) =
            crate::lean::turn_result::TurnResult::parse_turn_review(&result.text).unwrap();
        assert_eq!(report, "One high finding remains open.");
        assert!(!verdict.approved);
        assert_eq!(verdict.findings.len(), 1);
        assert_eq!(data(call(&shared, &review))["recorded"], true);
    }
}
