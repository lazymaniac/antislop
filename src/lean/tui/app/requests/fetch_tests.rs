use super::*;
use crossterm::event::KeyEvent;
use serde_json::json;
use std::os::unix::net::UnixListener;
use std::sync::mpsc;

#[test]
fn reopening_details_restores_independent_workbench_tab_state() {
    use crate::lean::tui::WorkbenchTab;

    let mut app = App::test_app();
    app.navigation
        .update_presentation(|presentation| presentation.tab = WorkbenchTab::Diff);
    for (tab, (scroll, query)) in [
        (WorkbenchTab::Summary, (3, "summary query")),
        (WorkbenchTab::Conversation, (8, "conversation query")),
        (WorkbenchTab::Diff, (13, "diff query")),
    ] {
        let slot = usize::MAX - tab as usize;
        app.details_scroll_memory
            .insert(("worker:t1".into(), slot), scroll);
        app.details_query_memory
            .insert(("worker:t1".into(), slot), query.into());
    }

    app.restore_details("worker:t1");
    let details = app.details.as_mut().unwrap();
    assert_eq!((details.scroll, details.query.as_str()), (13, "diff query"));
    details.prepare_workbench_details(None, WorkbenchTab::Summary);
    assert_eq!(
        (details.scroll, details.query.as_str()),
        (3, "summary query")
    );
    details.prepare_workbench_details(Some(WorkbenchTab::Summary), WorkbenchTab::Conversation);
    assert_eq!(
        (details.scroll, details.query.as_str()),
        (8, "conversation query")
    );
    details.prepare_workbench_details(Some(WorkbenchTab::Conversation), WorkbenchTab::Diff);
    assert_eq!((details.scroll, details.query.as_str()), (13, "diff query"));
}

#[test]
fn delayed_a_to_b_to_a_reuses_the_active_fetch_and_single_worker() {
    let state = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(crate::lean::daemon::socket(state.path())).unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = std::io::BufReader::new(stream);
        let mut requests = 0;
        while let Some(request) = crate::transport::read::<serde_json::Value>(&mut reader).unwrap()
        {
            assert_eq!(request["request"]["op"], "result_document_content");
            assert_eq!(request["request"]["id"], "A");
            if requests == 0 {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            }
            let done = requests == 1;
            let offset = request["request"]["offset"].as_u64().unwrap();
            requests += 1;
            crate::transport::write(
                reader.get_mut(),
                &json!({"data":{"text":"café","next_offset":offset + 1,"done":done},"error":null}),
            )
            .unwrap();
            if done {
                break;
            }
        }
        requests
    });
    let (updates, receiver) = crate::lean::tui::update_channel(8);
    let client = Client::at(state.path());
    let coordinator = ResultFetchCoordinator::new();
    let request = |id: &str, selection| ResultFetchRequest {
        identity: ResultFetchIdentity {
            conversation: "conversation".into(),
            generation: 7,
            key: "orchestrator".into(),
            id: format!("result_document:{id}"),
        },
        selection,
        metadata: json!({"id":id,"path":"output.md","source_commit":"12345678"}),
        client: client.clone(),
        updates: updates.clone(),
    };
    coordinator.select(Some(request("A", 1)));
    started_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    coordinator.select(Some(request("B", 2)));
    coordinator.select(Some(request("A", 3)));
    release_tx.send(()).unwrap();
    let update = receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert!(
        matches!(update, Update::ResultDocument { id, selection: 3, .. } if id == "result_document:A")
    );
    assert_eq!(server.join().unwrap(), 2);
}

#[test]
fn changing_conversation_cancels_partial_content_before_fetching_the_new_session() {
    let state = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(crate::lean::daemon::socket(state.path())).unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = std::io::BufReader::new(stream);
        let request = crate::transport::read::<serde_json::Value>(&mut reader)
            .unwrap()
            .unwrap();
        assert_eq!(request["request"]["conversation"], "session-a");
        started_tx.send(()).unwrap();
        release_rx.recv().unwrap();
        crate::transport::write(
            reader.get_mut(),
            &json!({"data":{"text":"partial A","next_offset":9,"done":false},"error":null}),
        )
        .unwrap();
        drop(reader);

        let (stream, _) = listener.accept().unwrap();
        let mut reader = std::io::BufReader::new(stream);
        let request = crate::transport::read::<serde_json::Value>(&mut reader)
            .unwrap()
            .unwrap();
        assert_eq!(request["request"]["conversation"], "session-b");
        assert_eq!(request["request"]["id"], "B");
        crate::transport::write(
            reader.get_mut(),
            &json!({"data":{"text":"complete B","next_offset":9,"done":true},"error":null}),
        )
        .unwrap();
        2
    });
    let (updates, receiver) = crate::lean::tui::update_channel(8);
    let client = Client::at(state.path());
    let coordinator = ResultFetchCoordinator::new();
    let request = |conversation: &str, id: &str, selection| ResultFetchRequest {
        identity: ResultFetchIdentity {
            conversation: conversation.into(),
            generation: 7,
            key: "orchestrator".into(),
            id: format!("result_document:{id}"),
        },
        selection,
        metadata: json!({"id":id,"path":"output.md","source_commit":"12345678"}),
        client: client.clone(),
        updates: updates.clone(),
    };
    coordinator.select(Some(request("session-a", "A", 1)));
    started_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    coordinator.select(Some(request("session-b", "B", 1)));
    release_tx.send(()).unwrap();
    let update = receiver
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert!(
        matches!(update, Update::ResultDocument { conversation, id, .. } if conversation == "session-b" && id == "result_document:B")
    );
    assert_eq!(server.join().unwrap(), 2);
    assert_eq!(
        std::fs::read_dir(state.path().join("inspection-cache"))
            .unwrap()
            .count(),
        1
    );
}

#[test]
fn session_owner_fetch_coalesces_repeated_keys_and_keeps_only_latest_target() {
    let state = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(crate::lean::daemon::socket(state.path())).unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let mut targets = Vec::new();
        for index in 0..2 {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream);
            let request = crate::transport::read::<serde_json::Value>(&mut reader)
                .unwrap()
                .unwrap();
            assert_eq!(request["request"]["op"], "sessions");
            targets.push(
                request["request"]["conversation"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            );
            if index == 0 {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            }
            crate::transport::write(reader.get_mut(), &json!({"data":[],"error":null})).unwrap();
        }
        targets
    });
    let (updates, receiver) = crate::lean::tui::update_channel(8);
    let mut app = App::test_app();
    app.client = Client::at(state.path());
    app.updates = updates;
    app.conversation = "A".into();
    app.fetch_workbench_sessions();
    started_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    for _ in 0..1000 {
        app.fetch_workbench_sessions();
    }
    assert_eq!(app.workbench_sessions_request_id, 1);
    app.conversation = "B".into();
    app.generation += 1;
    app.fetch_workbench_sessions();
    app.conversation = "C".into();
    app.generation += 1;
    app.fetch_workbench_sessions();
    assert_eq!(app.workbench_sessions_request_id, 1);
    release_tx.send(()).unwrap();
    app.apply_update(
        receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap(),
    );
    assert_eq!(app.workbench_sessions_request_id, 2);
    app.apply_update(
        receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap(),
    );
    assert!(app.workbench_sessions_inflight.is_none());
    assert_eq!(server.join().unwrap(), ["A", "C"]);
}

#[test]
fn conversation_picker_and_delete_return_before_daemon_reply_and_keep_new_context() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let state = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(crate::lean::daemon::socket(state.path())).unwrap();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        for operation in ["conversations", "delete_session"] {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream);
            let request = crate::transport::read::<serde_json::Value>(&mut reader)
                .unwrap()
                .unwrap();
            assert_eq!(request["request"]["op"], operation);
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            let data = if operation == "conversations" {
                json!([{"id":"other","goal":"Other session"}])
            } else {
                json!({})
            };
            crate::transport::write(reader.get_mut(), &json!({"data":data,"error":null})).unwrap();
        }
    });
    let (updates, receiver) = crate::lean::tui::update_channel(8);
    let mut app = App::test_app();
    app.client = Client::at(state.path());
    app.updates = updates;
    app.input = InputBuffer::with_text("unsent 👩‍💻");
    // These handlers must return while the server is held, without a timing threshold.
    app.open_conversation_picker();
    started_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert!(
        app.picker
            .as_ref()
            .unwrap()
            .status
            .as_ref()
            .unwrap()
            .contains("Loading")
    );
    app.picker.as_mut().unwrap().query = "Other".into();
    release_tx.send(()).unwrap();
    app.apply_update(
        receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap(),
    );
    assert_eq!(app.picker.as_ref().unwrap().query, "Other");
    app.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE));
    started_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    let current = app.conversation.clone();
    app.open("other".into());
    assert_eq!(
        app.conversation, current,
        "cannot enter a session being deleted"
    );
    app.projects = vec![
        workspace::ProjectConn {
            path: state.path().join("project-a"),
            name: "A".into(),
            client: app.client.clone(),
            branch: None,
            conversations: Vec::new(),
        },
        workspace::ProjectConn {
            path: state.path().join("project-b"),
            name: "B".into(),
            client: Client::at(&state.path().join("other-state")),
            branch: None,
            conversations: Vec::new(),
        },
    ];
    app.switch_project(1, Some("other".into()));
    assert_eq!(app.active_project, 0);
    assert_eq!(app.client.state_dir(), state.path());
    assert_eq!(app.conversation, current);
    app.projects.clear();

    app.picker = Some(Picker::new(Kind::Commands));
    app.picker.as_mut().unwrap().query = "new query".into();
    app.picker.as_mut().unwrap().selected = 3;
    release_tx.send(()).unwrap();
    app.apply_update(
        receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap(),
    );
    assert_eq!(app.picker.as_ref().unwrap().query, "new query");
    assert_eq!(app.picker.as_ref().unwrap().selected, 3);
    assert_eq!(app.input.text, "unsent 👩‍💻");
    assert!(app.conversation_deletions.is_empty());
    app.picker = Some(Picker::new(Kind::Conversations));
    app.apply_conversations(
        app.client.state_dir().to_string_lossy().into_owned(),
        app.conversations_request_id,
        Ok(json!([{"id":"other", "goal":"Stale deleted session"}])),
    );
    assert!(
        !app.picker
            .as_ref()
            .unwrap()
            .conversations
            .iter()
            .any(|item| item.id == "other")
    );
    server.join().unwrap();
}

#[test]
fn deletion_replies_keep_other_pending_decisions_and_their_selection() {
    let mut app = App::test_app();
    let first = palette::ConversationItem {
        id: "first".into(),
        project: "p".into(),
        title: "First".into(),
    };
    let second = palette::ConversationItem {
        id: "second".into(),
        project: "p".into(),
        title: "Second".into(),
    };
    let mut picker = Picker::new(Kind::Conversations);
    picker.conversations = vec![first.clone(), second.clone()];
    picker.selected = 1;
    picker.status = Some("Deleting session...".into());
    picker.delete_target = Some(second.clone());
    app.picker = Some(picker);
    app.conversation_deletions
        .insert((first.project.clone(), first.id.clone()));
    app.conversation_deletions
        .insert((second.project.clone(), second.id.clone()));
    app.apply_conversation_deleted(
        first.project.clone(),
        first.id.clone(),
        Err("RAW first deletion failure".into()),
    );
    assert!(app.notice.as_ref().unwrap().contains("first"));
    assert!(
        app.notice
            .as_ref()
            .unwrap()
            .contains("RAW first deletion failure")
    );
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.delete_target, Some(second.clone()));
    assert_eq!(picker.status.as_deref(), Some("Deleting session..."));
    assert_eq!(picker.selected, 1);
    app.apply_conversation_deleted(second.project.clone(), second.id.clone(), Ok(json!({})));
    assert!(app.notice.as_ref().unwrap().contains("second"));
    let picker = app.picker.as_ref().unwrap();
    assert!(picker.delete_target.is_none());
    assert_eq!(picker.conversations, vec![first]);
    assert!(app.conversation_deletions.is_empty());
}

#[test]
fn round_shortcuts_fetch_each_selected_snapshot_and_scrolling_does_not_refetch_it() {
    let state = tempfile::tempdir().unwrap();
    let listener = UnixListener::bind(crate::lean::daemon::socket(state.path())).unwrap();
    let uuid = "550e8400-e29b-41d4-a716-446655440000";
    let server = std::thread::spawn(move || {
        for round in [1, 2] {
            for op in ["task_rounds", "task_round", "result_documents"] {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = std::io::BufReader::new(stream);
                let request = crate::transport::read::<Value>(&mut reader)
                    .unwrap()
                    .unwrap();
                assert_eq!(request["request"]["op"], op);
                let data = match op {
                    "task_rounds" => json!([{"round":1},{"round":2}]),
                    "task_round" => {
                        assert_eq!(request["request"]["round"], round);
                        json!({"id":uuid,"round":round,"status":"done","report":format!("Retained report for round {round}"),"turns":[]})
                    }
                    _ => {
                        json!([{"id":"latest-output","path":"latest.md","source_commit":"latest-commit"}])
                    }
                };
                crate::transport::write(reader.get_mut(), &json!({"data":data,"error":null}))
                    .unwrap();
            }
        }
    });
    let (mut app, updates) = App::test_app_with_client(Client::at(state.path()));
    app.view.plan = Some("plan".into());
    app.view.tasks.push(view::TaskRow {
        id: "t1".into(),
        uuid: Some(uuid.into()),
        round: 2,
        ..Default::default()
    });
    app.navigation.screen = Screen::Workbench;
    app.navigation.target = WorkbenchTarget::Agent("worker:t1".into());
    app.navigation
        .update_presentation(|presentation| presentation.tab = WorkbenchTab::Results);
    let mut details = Details::new("worker:t1".into());
    details.task_uuid = Some(uuid.into());
    details.available_rounds = vec![1, 2];
    details.prepare_workbench_details(None, WorkbenchTab::Results);
    app.details = Some(details);
    for (key, round) in [(',', 1), ('.', 2)] {
        app.key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE));
        let update = updates
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert!(matches!(&update, Update::Task { round: actual, .. } if *actual == round));
        app.apply_update(update);
        let details = app.details.as_ref().unwrap();
        assert_eq!(details.task.as_ref().unwrap()["round"], round);
        let document = details
            .documents
            .get("result")
            .expect("round report loaded");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let page = loop {
            if let Some(page) = document.viewport(80, 30, 0, false).unwrap() {
                break page;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        };
        assert!(
            page.lines
                .iter()
                .any(|(_, text)| text.contains(&format!("Retained report for round {round}")))
        );
        let request = app.task_request_id;
        app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        assert_eq!(
            app.task_request_id, request,
            "scrolling reuses the selected snapshot"
        );
    }
    server.join().unwrap();
}
