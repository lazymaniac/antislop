use super::{
    ControlRequest, ConversationRequest, HistoryRequest, LedgerRequest, Request, SessionRequest,
    TerminalRequest, WireRequest,
};
use serde_json::json;

#[test]
fn grouped_requests_keep_the_tagged_wire_shape() {
    let cases = [
        (
            Request::Terminal(TerminalRequest::TerminalOpen {
                conversation: "c".into(),
                role: "writer".into(),
                columns: 80,
                rows: 24,
                retry: false,
            }),
            json!({"op":"terminal_open","conversation":"c","role":"writer","columns":80,"rows":24,"retry":false}),
        ),
        (
            Request::Session(SessionRequest::RunContext {
                conversation: "c".into(),
                turn: "t".into(),
            }),
            json!({"op":"run_context","conversation":"c","turn":"t"}),
        ),
        (
            Request::History(HistoryRequest::Events {
                conversation: "c".into(),
                after: 4,
                wait_ms: 0,
            }),
            json!({"op":"events","conversation":"c","after":4,"wait_ms":0,"limit":null}),
        ),
        (
            Request::Ledger(LedgerRequest::ListRuns {
                conversation: "c".into(),
            }),
            json!({"op":"list_runs","conversation":"c"}),
        ),
        (
            Request::Conversation(ConversationRequest::Plans {
                conversation: "c".into(),
                offset: None,
                limit: None,
            }),
            json!({"op":"plans","conversation":"c","offset":null,"limit":null}),
        ),
        (
            Request::Control(ControlRequest::Trust { trusted: true }),
            json!({"op":"trust","trusted":true}),
        ),
    ];

    for (request, expected) in cases {
        let encoded = serde_json::to_value(&request).unwrap();
        assert_eq!(encoded, expected);
        let decoded: Request = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
    }
}

#[test]
fn grouped_protocol_keeps_defaults_and_rejects_malformed_or_unknown_fields() {
    let decoded: Request = serde_json::from_value(json!({
        "op":"terminal_open","conversation":"c","role":"writer","columns":80,"rows":24
    }))
    .unwrap();
    assert_eq!(
        decoded,
        Request::Terminal(TerminalRequest::TerminalOpen {
            conversation: "c".into(),
            role: "writer".into(),
            columns: 80,
            rows: 24,
            retry: false,
        })
    );
    let encoded = serde_json::to_value(decoded).unwrap();
    assert_eq!(encoded["retry"], false);
    let trust: Request = serde_json::from_value(json!({"op":"trust"})).unwrap();
    assert_eq!(
        trust,
        Request::Control(ControlRequest::Trust { trusted: true })
    );

    for malformed in [
        json!({"op":"terminal_open","conversation":"c"}),
        json!({"op":"trust","trusted":"yes"}),
        json!({"op":"trust","trusted":true,"extra":1}),
        json!({"op":"unknown_operation"}),
    ] {
        assert!(serde_json::from_value::<Request>(malformed).is_err());
    }
}

#[test]
fn every_wire_operation_round_trips_through_its_grouped_request() {
    let inputs = [
        json!({"op":"terminal_open","conversation":"c","role":"writer","columns":80,"rows":24}),
        json!({"op":"terminal_read","conversation":"c","role":"writer","handle":"h","after":2}),
        json!({"op":"terminal_input","conversation":"c","role":"writer","handle":"h","sequence":3,"input":{"kind":"paste","text":"hello"}}),
        json!({"op":"terminal_resize","conversation":"c","role":"writer","handle":"h","columns":100,"rows":30}),
        json!({"op":"terminal_detach","conversation":"c","role":"writer","handle":"h"}),
        json!({"op":"run_context","conversation":"c","turn":"t"}),
        json!({"op":"comment_at","conversation":"c","plan":null,"target":"T1","text":"note","round":null,"run":null}),
        json!({"op":"send_to_agent","message":{"conversation":"c","plan":null,"recipient":"worker","operation_id":"op","expected_round":null,"expected_run":null,"text":"hello"}}),
        json!({"op":"task_rounds","conversation":"c","plan":"p","id":"task"}),
        json!({"op":"task_round","conversation":"c","plan":"p","id":"task","round":1}),
        json!({"op":"session_events","conversation":"c","turn":"t","after":0,"limit":null}),
        json!({"op":"history_content","conversation":"c","owner":{"kind":"round","task":"task","round":1,"field":"report"},"offset":0}),
        json!({"op":"history_messages","conversation":"c","scope":{"kind":"round","task":"task","round":1},"after":0,"limit":null}),
        json!({"op":"session_content","conversation":"c","event":1,"field":"prompt","offset":0}),
        json!({"op":"list_runs","conversation":"c"}),
        json!({"op":"reject_ledger_proposal","conversation":"c","run_id":"r","proposal_id":"p","diff_hash":"h","comment":"no"}),
        json!({"op":"get_ledger_proposal","conversation":"c","run_id":"r"}),
        json!({"op":"list_requirements","query":{"conversation":"c","run_id":"r"}}),
        json!({"op":"get_ledger_entry","conversation":"c","run_id":"r","id":"e","revision":null}),
        json!({"op":"get_ledger_diff","conversation":"c","run_id":"r","from_revision":0,"to_revision":1}),
        json!({"op":"get_evidence","conversation":"c","run_id":"r","entry_id":null}),
        json!({"op":"propose_ledger_change","change":{"conversation":"c","run_id":"r","expected_spec_revision":0,"operation_id":"op","reason":"test","entries":[]}}),
        json!({"op":"apply_ledger_change","proposal_id":"p","diff_hash":"h","change":{"conversation":"c","run_id":"r","expected_spec_revision":0,"operation_id":"op","reason":"test","entries":[]}}),
        json!({"op":"create_run","conversation":"c","goal":"goal"}),
        json!({"op":"add_ledger_source","conversation":"c","run_id":"r","text":"source"}),
        json!({"op":"get_ledger_source","conversation":"c","run_id":"r","source_id":"s"}),
        json!({"op":"identity"}),
        json!({"op":"get_config"}),
        json!({"op":"set_config","toml":"[policy]","expected_revision":"revision"}),
        json!({"op":"shutdown"}),
        json!({"op":"force_shutdown"}),
        json!({"op":"conversations"}),
        json!({"op":"start"}),
        json!({"op":"send","conversation":"c","text":"hello"}),
        json!({"op":"accept","conversation":"c","plan":"p"}),
        json!({"op":"reject","conversation":"c","plan":"p"}),
        json!({"op":"resume","conversation":"c","plan":"p"}),
        json!({"op":"review_comments","conversation":"c","plan":"p"}),
        json!({"op":"recover","conversation":"c","plan":"p","task":"t","action":"retry_with_guidance","freshness_token":"f"}),
        json!({"op":"restart_orchestrator","conversation":"c"}),
        json!({"op":"cancel","conversation":"c"}),
        json!({"op":"pause","conversation":"c","paused":true}),
        json!({"op":"mode","conversation":"c","autonomous":false}),
        json!({"op":"plans","conversation":"c"}),
        json!({"op":"plan","conversation":"c","plan":"p"}),
        json!({"op":"comments","conversation":"c"}),
        json!({"op":"comment","conversation":"c","target":"T1","text":"note"}),
        json!({"op":"comment_status","conversation":"c","id":1,"status":"resolved"}),
        json!({"op":"diff","conversation":"c","plan":"p","id":"t"}),
        json!({"op":"review","conversation":"c","plan":"p","id":"t"}),
        json!({"op":"task_checks","plan":"p","id":"t"}),
        json!({"op":"task","plan":"p","id":"t"}),
        json!({"op":"events","conversation":"c","after":0,"wait_ms":0}),
        json!({"op":"sessions","conversation":"c"}),
        json!({"op":"history","conversation":"c"}),
        json!({"op":"mail","conversation":"c"}),
        json!({"op":"trust_status"}),
        json!({"op":"trust"}),
    ];

    for input in inputs {
        let wire: WireRequest = serde_json::from_value(input.clone()).unwrap();
        let expected = serde_json::to_value(wire).unwrap();
        let request: Request = serde_json::from_value(input).unwrap();
        assert_group_matches_operation(&request, expected["op"].as_str().unwrap());
        assert_eq!(serde_json::to_value(request).unwrap(), expected);
    }
}

fn assert_group_matches_operation(request: &Request, operation: &str) {
    let actual = match request {
        Request::Terminal(_) => "terminal",
        Request::Session(_) => "session",
        Request::History(_) => "history",
        Request::Ledger(_) => "ledger",
        Request::Conversation(_) => "conversation",
        Request::Control(_) => "control",
    };
    let expected = match operation {
        "terminal_open" | "terminal_read" | "terminal_input" | "terminal_resize"
        | "terminal_detach" => "terminal",
        "run_context" | "session_events" | "session_content" | "sessions" => "session",
        "task_rounds" | "task_round" | "history_content" | "history_messages" | "events"
        | "history" | "mail" => "history",
        "list_runs"
        | "reject_ledger_proposal"
        | "get_ledger_proposal"
        | "list_requirements"
        | "get_ledger_entry"
        | "get_ledger_diff"
        | "get_evidence"
        | "propose_ledger_change"
        | "apply_ledger_change"
        | "create_run"
        | "add_ledger_source"
        | "get_ledger_source" => "ledger",
        "send_to_agent"
        | "send"
        | "accept"
        | "reject"
        | "resume"
        | "review_comments"
        | "recover"
        | "restart_orchestrator"
        | "cancel"
        | "pause"
        | "mode"
        | "plans"
        | "plan"
        | "comments"
        | "comment"
        | "comment_status"
        | "diff"
        | "review"
        | "task_checks"
        | "task"
        | "comment_at" => "conversation",
        "identity" | "get_config" | "set_config" | "shutdown" | "force_shutdown"
        | "conversations" | "start" | "trust_status" | "trust" => "control",
        unknown => panic!("unmapped wire operation: {unknown}"),
    };
    assert_eq!(actual, expected, "operation: {operation}");
}
