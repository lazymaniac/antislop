use architect::{
    context::{Bundle, Reader, Scope, bridge::Bridge},
    hash,
};
use serde_json::{Value, json};
use std::{
    io::{BufReader, Write},
    os::unix::net::UnixStream,
};

#[test]
fn context_bytes_wait_for_owner_commit_and_failed_ack_closes_without_a_page() {
    for accepted in [true, false] {
        let bundle = Bundle::new(
            Scope {
                run_id: "run".into(),
                spec_revision: 1,
                turn_id: "turn".into(),
                input_hash: hash("inputs"),
                prompt_hash: hash("prompt"),
                schema_hash: hash("schema"),
            },
            [("source".into(), "PRIVATE_SOURCE".into())],
        )
        .unwrap();
        let bytes = bundle.bytes().unwrap();
        let reader = Reader::from_bytes(&bytes, &hash(&bytes)).unwrap();
        let bridge = Bridge::start(reader.clone()).unwrap();
        let mut stream = UnixStream::connect(bridge.socket()).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_millis(100)))
            .unwrap();
        let mut input = BufReader::new(stream.try_clone().unwrap());
        writeln!(stream,"{}",json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}})).unwrap();
        stream.flush().unwrap();
        assert!(
            architect::transport::read::<Value>(&mut input)
                .unwrap()
                .is_some()
        );
        writeln!(
            stream,
            "{}",
            json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .unwrap();
        writeln!(stream,"{}",json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_context","arguments":{"bundle_hash":reader.bundle_hash(),"document_hash":hash("PRIVATE_SOURCE"),"offset":0,"max_bytes":32}}})).unwrap();
        stream.flush().unwrap();
        let start = std::time::Instant::now();
        let commit = loop {
            if let Some(commit) = bridge.pending() {
                break commit;
            }
            assert!(start.elapsed() < std::time::Duration::from_secs(3));
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        let before = architect::transport::read::<Value>(&mut input).unwrap_err();
        assert!(
            matches!(before,architect::Error::Io(ref e) if matches!(e.kind(),std::io::ErrorKind::WouldBlock|std::io::ErrorKind::TimedOut))
        );
        commit.acknowledge(accepted);
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(3)))
            .unwrap();
        let response = architect::transport::read::<Value>(&mut input).unwrap();
        if accepted {
            let page: architect::context::Page = serde_json::from_str(
                response.unwrap()["result"]["content"][0]["text"]
                    .as_str()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(page.content, "PRIVATE_SOURCE");
            reader.authenticate(&page).unwrap();
        } else {
            assert!(response.is_none());
        }
        drop(bridge);
    }
}

#[test]
fn bridge_keeps_a_session_open_until_peer_eof_and_closes_all_its_stream_clones() {
    let bundle = Bundle::new(
        Scope {
            run_id: "run".into(),
            spec_revision: 1,
            turn_id: "turn".into(),
            input_hash: hash("inputs"),
            prompt_hash: hash("prompt"),
            schema_hash: hash("schema"),
        },
        [("source".into(), "original".into())],
    )
    .unwrap();
    let bytes = bundle.bytes().unwrap();
    let reader = Reader::from_bytes(&bytes, &hash(&bytes)).unwrap();
    let bridge = Bridge::start(reader).unwrap();
    let mut stream = UnixStream::connect(bridge.socket()).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(3)))
        .unwrap();
    let mut input = BufReader::new(stream.try_clone().unwrap());
    let init = json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"fixture","version":"1"}}});
    writeln!(stream, "{init}").unwrap();
    stream.flush().unwrap();
    let response: Value = architect::transport::read(&mut input).unwrap().unwrap();
    assert!(response["result"]["serverInfo"].is_object());
    writeln!(
        stream,
        "{}",
        json!({"jsonrpc":"2.0","method":"notifications/initialized"})
    )
    .unwrap();
    writeln!(
        stream,
        "{}",
        json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}})
    )
    .unwrap();
    stream.flush().unwrap();
    let response: Value = architect::transport::read(&mut input).unwrap().unwrap();
    assert_eq!(response["result"]["tools"].as_array().unwrap().len(), 1);
    stream.shutdown(std::net::Shutdown::Write).unwrap();
    let eof = architect::transport::read::<Value>(&mut input).unwrap();
    assert!(eof.is_none());
    let socket = bridge.socket().to_owned();
    drop(bridge);
    assert!(!socket.exists());
}
