use super::*;

fn context(store: &Store, provider: &str) -> RunContext {
    let conversation = store.create_conversation("project").unwrap();
    let turn = store
        .start_turn(&conversation.id, "orchestrator", provider, 0, None)
        .unwrap();
    RunContext {
        turn_id: turn.id,
        conversation: conversation.id,
        role: turn.role,
        provider: provider.into(),
        profile: "default".into(),
        cwd: std::env::current_dir().unwrap(),
        add_dirs: vec![],
        access: RunAccess::ReadOnly,
        backend: RunBackend::Cli,
        provider_thread_id: None,
        provider_session_id: None,
        provider_turn_id: None,
    }
}

#[test]
fn schema_nine_is_rejected_without_changing_history() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("core.db");
    let store = Store::open(&path).unwrap();
    let context = context(&store, "codex");
    store.set_session(&context.turn_id, "old-thread").unwrap();
    store
        .db()
        .execute_batch("DROP TABLE run_contexts; PRAGMA user_version=9;")
        .unwrap();
    drop(store);
    let before = std::fs::read(&path).unwrap();
    assert!(Store::open(&path).is_err());
    assert_eq!(before, std::fs::read(&path).unwrap());
}

#[test]
fn context_survives_restart_and_keeps_thread_separate_from_session() {
    for provider in ["codex", "opencode", "claude", "gemini", "agy"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.db");
        let store = Store::open(&path).unwrap();
        let mut context = context(&store, provider);
        store.record_run_context(&context).unwrap();
        store.record_run_context(&context).unwrap();
        store.set_session(&context.turn_id, "provider-id").unwrap();
        if provider == "codex" {
            context.provider_thread_id = Some("provider-id".into());
        } else {
            context.provider_session_id = Some("provider-id".into());
        }
        drop(store);
        let store = Store::open(&path).unwrap();
        assert_eq!(
            store
                .run_context(&context.conversation, &context.turn_id)
                .unwrap(),
            Some(context)
        );
    }
}

#[test]
fn context_cannot_change_scope_backend_profile_or_directory() {
    let store = Store::open(Path::new(":memory:")).unwrap();
    let original = context(&store, "codex");
    store.record_run_context(&original).unwrap();
    let other = store.create_conversation("other").unwrap();
    assert!(
        store
            .run_context(&other.id, &original.turn_id)
            .unwrap()
            .is_none()
    );
    for field in [
        "conversation",
        "role",
        "provider",
        "profile",
        "cwd",
        "access",
        "backend",
    ] {
        let mut changed = original.clone();
        match field {
            "conversation" => changed.conversation = other.id.clone(),
            "role" => changed.role = "worker:foreign".into(),
            "provider" => changed.provider = "opencode".into(),
            "profile" => changed.profile = "foreign".into(),
            "cwd" => changed.cwd = original.cwd.join("foreign"),
            "access" => changed.access = RunAccess::Write,
            "backend" => changed.backend = RunBackend::CodexAppServer,
            _ => unreachable!(),
        }
        assert!(store.record_run_context(&changed).is_err(), "{field}");
        assert_eq!(
            store
                .run_context(&original.conversation, &original.turn_id)
                .unwrap(),
            Some(original.clone())
        );
    }
    store
        .finish_turn(&original.turn_id, TurnStatus::Done, None)
        .unwrap();
    assert!(store.record_run_context(&original).is_err());
}

#[test]
fn native_turn_binding_is_scoped_and_idempotent() {
    let store = Store::open(Path::new(":memory:")).unwrap();
    let mut context = context(&store, "codex");
    assert!(
        store
            .bind_provider_turn(&context.conversation, &context.turn_id, "turn-1")
            .is_err()
    );
    context.backend = RunBackend::CodexAppServer;
    store.record_run_context(&context).unwrap();
    assert!(
        store
            .bind_provider_turn("foreign", &context.turn_id, "turn-1")
            .is_err()
    );
    assert!(
        store
            .bind_provider_turn(&context.conversation, &context.turn_id, "")
            .is_err()
    );
    store
        .bind_provider_turn(&context.conversation, &context.turn_id, "turn-1")
        .unwrap();
    store
        .bind_provider_turn(&context.conversation, &context.turn_id, "turn-1")
        .unwrap();
    assert!(
        store
            .bind_provider_turn(&context.conversation, &context.turn_id, "turn-2")
            .is_err()
    );
    let bound = store
        .run_context(&context.conversation, &context.turn_id)
        .unwrap()
        .unwrap();
    assert_eq!(bound.provider_turn_id.as_deref(), Some("turn-1"));
    assert_eq!(bound.provider_thread_id, None);
    assert_eq!(bound.provider_session_id, None);
    store
        .finish_turn(&context.turn_id, TurnStatus::Interrupted, None)
        .unwrap();
    assert!(
        store
            .bind_provider_turn(&context.conversation, &context.turn_id, "turn-1")
            .is_err()
    );
}

#[test]
fn invalid_context_does_not_leave_a_row() {
    let store = Store::open(Path::new(":memory:")).unwrap();
    let mut context = context(&store, "codex");
    context.backend = RunBackend::OpencodeServer;
    assert!(store.record_run_context(&context).is_err());
    context.backend = RunBackend::Cli;
    context.cwd = "relative".into();
    assert!(store.record_run_context(&context).is_err());
    context.cwd = std::env::current_dir().unwrap();
    context.add_dirs.push("relative".into());
    assert!(store.record_run_context(&context).is_err());
    assert_eq!(
        store
            .run_context(&context.conversation, &context.turn_id)
            .unwrap(),
        None
    );
}

#[test]
fn native_session_cannot_be_rebound_and_legacy_resume_updates_atomically() {
    for (provider, backend) in [
        ("codex", RunBackend::CodexAppServer),
        ("opencode", RunBackend::OpencodeServer),
    ] {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let mut context = context(&store, provider);
        context.backend = backend;
        store.record_run_context(&context).unwrap();
        store.set_session(&context.turn_id, "original").unwrap();
        store.set_session(&context.turn_id, "original").unwrap();
        assert!(store.set_session(&context.turn_id, "foreign").is_err());
        assert_eq!(
            store
                .last_session(&context.conversation, &context.role, provider)
                .unwrap()
                .as_deref(),
            Some("original")
        );
        let bound = store
            .run_context(&context.conversation, &context.turn_id)
            .unwrap()
            .unwrap();
        if provider == "codex" {
            assert_eq!(bound.provider_thread_id.as_deref(), Some("original"));
            assert_eq!(bound.provider_session_id, None);
        } else {
            assert_eq!(bound.provider_session_id.as_deref(), Some("original"));
            assert_eq!(bound.provider_thread_id, None);
        }
    }
}
