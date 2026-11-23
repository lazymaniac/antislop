use super::{
    ControlRequest, ConversationRequest, HistoryRequest, Request, SessionActionKind,
    SessionActionResult, SessionRequest, TerminalRequest, WireRequest,
};
use serde_json::json;

#[test]
fn grouped_requests_keep_the_tagged_wire_shape() {
    let cases = [
        (
            Request::Control(ControlRequest::ActivateRuntime {
                request_id: None,
                executable: "/runtime/antislop".into(),
                path: Some("/tools:/usr/bin".into()),
                conversation: Some("c".into()),
                task: Some("t".into()),
            }),
            json!({"op":"activate_runtime","executable":"/runtime/antislop","path":"/tools:/usr/bin","conversation":"c","task":"t"}),
        ),
        (
            Request::Control(ControlRequest::RuntimeActivationStatus),
            json!({"op":"runtime_activation_status"}),
        ),
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
            Request::Session(SessionRequest::SessionAction {
                conversation: "c".into(),
                action: SessionActionKind::CreatePr,
            }),
            json!({"op":"session_action","conversation":"c","action":"create_pr"}),
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
            Request::Conversation(ConversationRequest::GetSource {
                source_id: "s".into(),
                conversation: "c".into(),
            }),
            json!({"op":"get_source","conversation":"c","source_id":"s"}),
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
            Request::Control(ControlRequest::RuntimeNotice {
                conversation: "c".into(),
                text: "Verified toolchain path; no new execution grant".into(),
            }),
            json!({"op":"runtime_notice","conversation":"c","text":"Verified toolchain path; no new execution grant"}),
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
fn session_action_results_preserve_success_raw_failure_and_missing_inputs() {
    let cases = [
        (
            SessionActionResult::Queued { message: 12 },
            json!({"status":"queued","message":12}),
        ),
        (
            SessionActionResult::Succeeded {
                summary: "PR 42 opened".into(),
            },
            json!({"status":"succeeded","summary":"PR 42 opened"}),
        ),
        (
            SessionActionResult::Failed {
                details: "git push: permission denied (publickey)".into(),
            },
            json!({"status":"failed","details":"git push: permission denied (publickey)"}),
        ),
        (
            SessionActionResult::MissingInputs {
                inputs: vec!["remote repository".into()],
            },
            json!({"status":"missing_inputs","inputs":["remote repository"]}),
        ),
    ];
    for (result, expected) in cases {
        assert_eq!(serde_json::to_value(&result).unwrap(), expected);
        assert_eq!(
            serde_json::from_value::<SessionActionResult>(expected).unwrap(),
            result
        );
    }
    assert!(
        serde_json::from_value::<SessionActionResult>(json!({
            "status":"failed","details":"raw failure","ignored":true
        }))
        .is_err()
    );
    for invalid in [
        json!({"status":"failed","details":"  "}),
        json!({"status":"succeeded","summary":""}),
        json!({"status":"missing_inputs","inputs":[]}),
        json!({"status":"missing_inputs","inputs":[" "]}),
    ] {
        assert!(serde_json::from_value::<SessionActionResult>(invalid).is_err());
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
    for malformed in [
        json!({"op":"terminal_open","conversation":"c"}),
        json!({"op":"trust","trusted":"yes"}),
        json!({"op":"trust","trusted":true,"extra":1}),
        json!({"op":"unknown_operation"}),
    ] {
        assert!(serde_json::from_value::<Request>(malformed).is_err());
    }
}

mod fixtures;

#[test]
fn every_operation_matches_frozen_messages_in_both_directions() {
    let fixtures = fixtures::fixtures();
    assert_eq!(fixtures.len(), 58);
    let mut operations = std::collections::BTreeSet::new();
    for fixture in fixtures {
        let expected = fixture.expected;
        assert!(operations.insert(expected["op"].as_str().unwrap().to_owned()));
        assert_eq!(serde_json::to_value(&fixture.request).unwrap(), expected);
        assert_eq!(
            serde_json::from_value::<Request>(expected.clone()).unwrap(),
            fixture.request,
            "{expected}"
        );
        // Every supplied field has a type contract; arbitrary arrays are invalid.
        for field in expected.as_object().unwrap().keys() {
            let mut invalid = expected.clone();
            invalid[field] = json!([{}]);
            assert!(
                serde_json::from_value::<Request>(invalid).is_err(),
                "{field}: {expected}"
            );

            let mut absent = expected.clone();
            absent.as_object_mut().unwrap().remove(field);
            if let Some(default) = fixture.defaults.get(field) {
                let mut canonical = expected.clone();
                if fixture.omitted.contains(&field.as_str()) {
                    canonical.as_object_mut().unwrap().remove(field);
                } else {
                    canonical[field] = default.clone();
                }
                let request: Request = serde_json::from_value(absent).unwrap();
                assert_eq!(
                    serde_json::to_value(request).unwrap(),
                    canonical,
                    "{field}: {expected}"
                );
            } else {
                assert!(
                    serde_json::from_value::<Request>(absent).is_err(),
                    "{field}: {expected}"
                );
            }

            let mut null = expected.clone();
            null[field] = serde_json::Value::Null;
            if fixture.optional.contains(&field.as_str()) {
                let request: Request = serde_json::from_value(null).unwrap();
                let mut canonical = expected.clone();
                if fixture.omitted.contains(&field.as_str()) {
                    canonical.as_object_mut().unwrap().remove(field);
                } else {
                    canonical[field] = serde_json::Value::Null;
                }
                assert_eq!(serde_json::to_value(request).unwrap(), canonical);
            } else {
                assert!(
                    serde_json::from_value::<Request>(null).is_err(),
                    "{field}: {expected}"
                );
            }
        }
        let mut absent_defaults = expected.clone();
        let mut canonical_defaults = expected.clone();
        for (field, default) in fixture.defaults.as_object().unwrap() {
            absent_defaults.as_object_mut().unwrap().remove(field);
            canonical_defaults[field] = default.clone();
            let mut explicit = expected.clone();
            explicit[field] = default.clone();
            let mut canonical = explicit.clone();
            if fixture.omitted.contains(&field.as_str()) {
                canonical.as_object_mut().unwrap().remove(field);
                canonical_defaults.as_object_mut().unwrap().remove(field);
            }
            let request: Request = serde_json::from_value(explicit).unwrap();
            assert_eq!(serde_json::to_value(request).unwrap(), canonical);
        }
        let request: Request = serde_json::from_value(absent_defaults).unwrap();
        assert_eq!(serde_json::to_value(request).unwrap(), canonical_defaults);

        let mut unknown = expected.clone();
        unknown["scope_hash"] = json!("obsolete");
        if expected.as_object().unwrap().len() == 1 {
            // Existing internally tagged unit variants ignore surplus fields.
            assert_eq!(
                serde_json::from_value::<Request>(unknown).unwrap(),
                fixture.request
            );
        } else {
            assert!(
                serde_json::from_value::<Request>(unknown)
                    .unwrap_err()
                    .to_string()
                    .contains("unknown field")
            );
        }
    }
}

#[test]
fn special_adapters_keep_their_grouped_variants() {
    for (input, request, output) in [
        (
            json!({"op":"start"}),
            Request::Control(ControlRequest::Start),
            json!({"op":"start","project":null}),
        ),
        (
            json!({"op":"start","project":null}),
            Request::Control(ControlRequest::Start),
            json!({"op":"start","project":null}),
        ),
        (
            json!({"op":"events","conversation":"c","after":1,"wait_ms":2}),
            Request::History(HistoryRequest::Events {
                conversation: "c".into(),
                after: 1,
                wait_ms: 2,
            }),
            json!({"op":"events","conversation":"c","after":1,"wait_ms":2,"limit":null}),
        ),
    ] {
        assert_eq!(serde_json::from_value::<Request>(input).unwrap(), request);
        assert_eq!(serde_json::to_value(request).unwrap(), output);
    }
    let paged = Request::History(HistoryRequest::EventsPaged {
        conversation: "c".into(),
        after: 1,
        wait_ms: 2,
        limit: None,
    });
    assert_eq!(
        serde_json::to_value(paged).unwrap(),
        json!({"op":"events","conversation":"c","after":1,"wait_ms":2,"limit":null})
    );
}

#[test]
fn removed_mode_and_execution_authorization_operations_are_not_decoded() {
    for operation in [
        "mode",
        "authorize_task_execution",
        "accept",
        "comment_at",
        "trust",
        "trust_status",
    ] {
        let error =
            serde_json::from_value::<WireRequest>(json!({"op":operation,"conversation":"fixture"}))
                .unwrap_err()
                .to_string();
        assert!(error.contains("unknown variant"), "{error}");
    }
}

#[test]
fn removed_scope_fingerprint_field_is_rejected_in_favor_of_revision_token() {
    assert!(
        serde_json::from_value::<WireRequest>(json!({
            "op":"resume", "conversation":"c", "plan":"p", "scope_hash":"old"
        }))
        .is_err()
    );
    let request: Request = serde_json::from_value(json!({
        "op":"resume", "conversation":"c", "plan":"p", "revision_token":"current"
    }))
    .unwrap();
    assert_eq!(
        serde_json::to_value(request).unwrap()["revision_token"],
        "current"
    );
}

#[test]
fn envelope_and_transport_keep_validation_errors() {
    for input in [
        json!({"protocol":6,"request":{"op":"identity"}}),
        json!({"protocol":6,"request":{"op":"identity"},"client":null}),
        json!({"protocol":6,"request":{"op":"identity"},"client":"frontend"}),
    ] {
        let envelope: super::RequestEnvelope = serde_json::from_value(input.clone()).unwrap();
        assert_eq!(envelope.protocol, 6);
        assert_eq!(envelope.request, Request::Control(ControlRequest::Identity));
        assert_eq!(envelope.client.as_deref(), input["client"].as_str());
    }
    for (input, error) in [
        (
            json!({"request":{"op":"identity"}}),
            "missing field `protocol`",
        ),
        (json!({"protocol":6}), "missing field `request`"),
        (
            json!({"protocol":6,"request":{"op":"identity"},"extra":true}),
            "unknown field `extra`",
        ),
        (
            json!({"protocol":6,"request":{"op":"identity"},"client":false}),
            "invalid type",
        ),
        (
            json!({"protocol":"6","request":{"op":"identity"}}),
            "invalid type",
        ),
    ] {
        let actual = serde_json::from_value::<super::RequestEnvelope>(input)
            .err()
            .unwrap()
            .to_string();
        assert!(actual.contains(error), "{actual}");
    }
    // Use the real framing/unique-key decoder rather than serde_json alone.
    for bytes in [
        &b"{\"protocol\":6,\"protocol\":6,\"request\":{\"op\":\"identity\"}}\n"[..],
        &b"{\"protocol\":6,\"request\":{\"op\":\"identity\",\"op\":\"shutdown\"}}\n"[..],
    ] {
        let error =
            crate::transport::read::<super::RequestEnvelope>(&mut std::io::Cursor::new(bytes))
                .err()
                .unwrap()
                .to_string();
        assert!(error.contains("duplicate JSON object key"), "{error}");
    }
}

#[test]
fn malformed_messages_keep_diagnostic_text() {
    let unknown = serde_json::from_value::<Request>(json!({"op":"unknown_operation"}))
        .unwrap_err()
        .to_string();
    assert_eq!(
        unknown,
        include_str!("tests/unknown_operation.txt").trim_end()
    );
    for (input, expected) in [
        (
            json!({"op":"session_action","conversation":"c","action":"publish"}),
            "unknown variant `publish`, expected one of `create_pr`, `merge_with_main`, `continue_work`",
        ),
        (
            json!({"op":"send","conversation":"c"}),
            "missing field `text`",
        ),
        (
            json!({"op":"terminal_open","conversation":"c","role":"writer","columns":65536,"rows":24}),
            "invalid value: integer `65536`, expected u16",
        ),
        (
            json!({"op":"send","conversation":"c","text":"hello","scope_hash":"old"}),
            "unknown field `scope_hash`, expected one of `conversation`, `operation_id`, `text`, `to`, `images`",
        ),
    ] {
        assert_eq!(
            serde_json::from_value::<Request>(input)
                .unwrap_err()
                .to_string(),
            expected
        );
    }
}
