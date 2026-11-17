use super::*;
use serde_json::json;
use std::os::unix::net::UnixListener;
use std::sync::mpsc;

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
