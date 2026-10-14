//! Independent session identity and persistence counterexamples.
use architect::{
    Result,
    config::RoleKind,
    domain::{Run, RunSpec},
    providers::ProviderEvent,
    sessions::{SessionLedger, SessionRecord, SessionScope, TurnRecord},
};

fn ok(_: &SessionLedger, _: &str) -> Result<()> {
    Ok(())
}

#[test]
fn native_uuid_aliases_cannot_create_independent_managed_sessions() {
    let root = tempfile::tempdir().unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    let run = Run::new(architect::id(), spec).unwrap();
    let native = "aaaaaaaa-bbbb-4ccc-8ddd-eeeeeeeeeeee";
    for alias in [native.to_uppercase(), format!("urn:uuid:{native}")] {
        assert_eq!(
            uuid::Uuid::parse_str(native).unwrap(),
            uuid::Uuid::parse_str(&alias).unwrap()
        );
        let mut ledger = SessionLedger::default();
        for (logical, turn, role, raw) in [
            ("executor", "execution-turn", RoleKind::Executor, native),
            (
                "verifier",
                "verification-turn",
                RoleKind::Verifier,
                alias.as_str(),
            ),
        ] {
            let scope = SessionScope::for_run(
                &run,
                logical.into(),
                None,
                role,
                "codex".into(),
                "0.162.1".into(),
            )
            .unwrap();
            ledger
                .reserve(
                    SessionRecord::new(logical.into(), scope).unwrap(),
                    TurnRecord::new(
                        turn.into(),
                        logical.into(),
                        architect::hash("prompt"),
                        architect::hash("schema"),
                        None,
                    )
                    .unwrap(),
                    &mut ok,
                )
                .unwrap();
            let result = ledger.record_provider_event(
                turn,
                &ProviderEvent::SessionObserved {
                    session_id: raw.into(),
                },
                &mut ok,
            );
            if logical == "executor" {
                result.unwrap();
            } else {
                assert!(result.is_err(), "accepted reused native UUID alias {raw}");
            }
        }
    }
}
