use architect::tui::{app::App, model::Snapshot};
use serde_json::{Value, json};

fn run() -> Value {
    json!({"id":"run","status":"running","active_session":"other","session_ledger":{
        "sessions":{
            "a-new":{"id":"a-new","state":"failed","scope":{"agent_id":"A","role":"executor","provider":"claude","stage":"execution"}},
            "z-old":{"id":"z-old","state":"completed","scope":{"agent_id":"A","role":"executor","provider":"codex","stage":"execution"}},
            "other":{"id":"other","state":"sent","scope":{"agent_id":"B","role":"verifier","provider":"gemini","stage":"planning"}}
        },
        "turns":{
            "z-turn":{"id":"z-turn","session_id":"z-old","output":"old output"},
            "a-turn":{"id":"a-turn","session_id":"a-new","output":"new output"},
            "other-turn":{"id":"other-turn","session_id":"other","output":"unrelated output"}
        },"turn_order":["z-turn","a-turn","other-turn"]},
        "artifacts":[{"id":"old-artifact","session_id":"z-old","hash":"old-hash"},{"id":"new-artifact","session_id":"a-new","hash":"new-hash"},{"id":"other-artifact","session_id":"other","hash":"other-hash"}],
        "reviews":[
            {"executor_session":"z-old","artifact_hash":"old-hash","findings":["old finding"],"checks":[{"check_id":"old-check","evidence_ids":["shared-evidence"]}]},
            {"executor_session":"a-new","artifact_hash":"new-hash","findings":["new finding"],"checks":[{"check_id":"new-check","evidence_ids":[]}]},
            {"executor_session":"other","artifact_hash":"other-hash","findings":["unrelated finding"],"checks":[{"check_id":"unrelated-check"}]}
        ],
        "evidence":[{"id":"old-evidence","artifact_hash":"old-hash"},{"id":"new-evidence","artifact_hash":"new-hash"},{"id":"unrelated-evidence","artifact_hash":"other-hash"},{"id":"shared-evidence","artifact_hash":"shared-hash"}],
        "communication_ledger":{"messages":{"m":{"id":"m","recipient_agent_id":"A","state":"queued_next_turn"},"other-message":{"id":"other-message","recipient_agent_id":"B"}}}
    })
}

#[test]
fn retry_projection_uses_durable_order_retains_history_and_scopes_proofs_to_the_agent() {
    let document = run();
    let snapshot = Snapshot::from_document(&document, &json!({}));
    let agent = snapshot.agents.iter().find(|a| a.id == "A").unwrap();
    assert_eq!(agent.status, "failed");
    assert_eq!(agent.provider, "claude");
    let session: Value = serde_json::from_str(&agent.details["Session"]).unwrap();
    assert_eq!(session["current_session"], "a-new");
    assert_eq!(session["sessions"].as_array().unwrap().len(), 2);
    assert!(agent.details["Session"].contains("old output"));
    assert!(agent.details["Session"].contains("new output"));
    assert!(!agent.details["Session"].contains("unrelated output"));
    for (tab, expected, excluded) in [
        ("Artifacts", "old-artifact", "other-artifact"),
        ("Findings", "old finding", "unrelated finding"),
        ("Checks", "old-check", "unrelated-check"),
        ("Evidence", "new-evidence", "unrelated-evidence"),
        ("Messages", "queued_next_turn", "other-message"),
    ] {
        assert!(agent.details[tab].contains(expected), "{tab}: {expected}");
        assert!(!agent.details[tab].contains(excluded), "{tab}: {excluded}");
    }
    assert!(agent.details["Evidence"].contains("shared-evidence"));
    let mut app = App::default();
    app.update(snapshot);
    app.selected_agent_id = Some("A".into());
    let mut updated = document.clone();
    updated["active_session"] = json!("a-new");
    app.update(Snapshot::from_document(&updated, &json!({})));
    assert_eq!(app.selected_agent_id.as_deref(), Some("A"));
}

#[test]
fn ambiguous_legacy_session_history_is_explicitly_unknown_without_an_active_identity() {
    let mut document = run();
    document["session_ledger"]
        .as_object_mut()
        .unwrap()
        .remove("turn_order");
    let snapshot = Snapshot::from_document(&document, &json!({}));
    let agent = snapshot.agents.iter().find(|a| a.id == "A").unwrap();
    assert_eq!(agent.status, "unknown");
    let details: Value = serde_json::from_str(&agent.details["Session"]).unwrap();
    assert!(details["current_session"].is_null());
    assert_eq!(details["sessions"].as_array().unwrap().len(), 2);
    assert!(
        details["selection"]
            .as_str()
            .unwrap()
            .contains("unavailable")
    );
    document["active_session"] = json!("a-new");
    let snapshot = Snapshot::from_document(&document, &json!({}));
    assert_eq!(
        snapshot.agents.iter().find(|a| a.id == "A").unwrap().status,
        "failed"
    );
}

#[test]
fn team_projection_retains_all_owned_sessions_and_honors_its_explicit_current_session() {
    let mut document = run();
    document["team"] =
        json!({"agents":[{"id":"A","role":"implementer","session_id":"a-new"}],"tasks":[]});
    let snapshot = Snapshot::from_document(&document, &json!({}));
    let agent = &snapshot.agents[0];
    let details: Value = serde_json::from_str(&agent.details["Session"]).unwrap();
    assert_eq!(details["current_session"], "a-new");
    assert_eq!(details["sessions"].as_array().unwrap().len(), 2);
    assert!(agent.details["Artifacts"].contains("old-artifact"));
    assert!(!agent.details["Artifacts"].contains("other-artifact"));
}
