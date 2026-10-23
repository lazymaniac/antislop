use super::*;
use base64::Engine;
use std::io::BufRead;
use std::io::Write;
mod stream;

fn http_agent(stream: bool) -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(if stream {
            None
        } else {
            Some(Duration::from_secs(15))
        })
        .timeout_connect(Some(Duration::from_secs(2)))
        .timeout_recv_response(Some(Duration::from_secs(5)))
        .timeout_recv_body(if stream {
            None
        } else {
            Some(Duration::from_secs(20))
        })
        .max_redirects(0)
        .build()
        .into()
}

fn authorization(password: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("opencode:{password}"))
    )
}

fn raw(
    endpoint: &str,
    password: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<ureq::http::Response<ureq::Body>> {
    let agent = http_agent(path.ends_with("/event"));
    let url = format!("{endpoint}{path}");
    let response = match method {
        "GET" => agent
            .get(&url)
            .header("Authorization", authorization(password))
            .call(),
        "POST" => agent
            .post(&url)
            .header("Authorization", authorization(password))
            .send_json(body.unwrap_or(json!({}))),
        _ => return Err(invalid("unsupported OpenCode request method")),
    };
    let mut response = response.map_err(invalid)?;
    let status = response.status().as_u16();
    if status >= 400 {
        let mut bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(8192)
            .read_to_end(&mut bytes)?;
        let body = String::from_utf8_lossy(&bytes);
        let prefix = if path == "/session" && matches!(status, 404 | 405) {
            UNSUPPORTED_PROTOCOL
        } else {
            ""
        };
        return Err(invalid(format!(
            "{prefix}OpenCode {method} {path}: HTTP {status}: {body}"
        )));
    }
    Ok(response)
}

pub(super) fn request(
    session: &Session,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    let Backend::Opencode { endpoint, password } = &session.backend else {
        unreachable!()
    };
    let mut response = raw(endpoint, password, method, path, body)?;
    if response.status().as_u16() == 204 {
        return Ok(Value::Null);
    }
    response.body_mut().read_json().map_err(invalid)
}

pub(super) fn start(core: &Core, spec: &agent::Spec) -> Result<(Process, Backend, String, String)> {
    let configured = agent::command(spec, "")?;
    let mut config: Value = serde_json::from_str(
        &configured
            .get_envs()
            .find(|(key, _)| *key == "OPENCODE_CONFIG_CONTENT")
            .and_then(|(_, value)| value)
            .ok_or_else(|| invalid("missing managed OpenCode configuration"))?
            .to_string_lossy(),
    )?;
    config["default_agent"] = "antislop".into();
    if let Some(model) = &spec.model {
        config["model"] = model.clone().into();
    }
    let password = crate::id();
    let mut command = Command::new(&spec.binary);
    command
        .current_dir(&spec.cwd)
        .args(["serve", "--hostname", "127.0.0.1", "--port", "0"])
        .envs(
            configured
                .get_envs()
                .filter_map(|(key, value)| value.map(|value| (key, value))),
        )
        .env("OPENCODE_CONFIG_CONTENT", config.to_string())
        .env("OPENCODE_SERVER_PASSWORD", &password)
        .env("OPENCODE_SERVER_USERNAME", "opencode");
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let process = Process::spawn(command, Some(tx))?;
    process.register(core, "opencode")?;
    let endpoint = rx.recv_timeout(Duration::from_secs(10)).map_err(|error| {
        invalid(format!(
            "OpenCode serve did not open its endpoint: {error}: {}",
            process.diagnostics()
        ))
    })?;
    let response = if let Some(id) = &spec.resume {
        require(
            id.bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
            "invalid OpenCode session id",
        )?;
        raw(&endpoint, &password, "GET", &format!("/session/{id}"), None)?
    } else {
        raw(
            &endpoint,
            &password,
            "POST",
            "/session",
            Some(json!({"title":"antislop managed session"})),
        )?
    };
    let value: Value = response.into_body().read_json().map_err(invalid)?;
    let identity = value["id"]
        .as_str()
        .ok_or_else(|| invalid("OpenCode did not return a session id"))?
        .to_owned();
    require(
        spec.resume.as_ref().is_none_or(|id| *id == identity),
        "OpenCode resumed a different session",
    )?;
    // Bind the listener now; the gateway thread takes it in gateway().
    let endpoint_gateway = format!("http://127.0.0.1:0/{}", crate::id());
    Ok((
        process,
        Backend::Opencode { endpoint, password },
        identity,
        endpoint_gateway,
    ))
}

pub(super) fn run(
    session: &Session,
    turn: &str,
    prompt: &str,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(agent::Event),
) -> Result<agent::Outcome> {
    stream::run(session, turn, prompt, cancel, emit)
}

pub(super) fn gateway(session: &Arc<Session>) -> Result<()> {
    // The endpoint is set before Session is shared outside prepare().
    // Store the actual gateway address separately from the provider endpoint.
    let server = tiny_http::Server::http("127.0.0.1:0").map_err(invalid)?;
    let address = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| invalid("OpenCode gateway has no TCP address"))?;
    *session.gateway_address.lock().map_err(invalid)? = Some(format!("http://{address}"));
    let weak = Arc::downgrade(session);
    let stop = session.stop.clone();
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            match server.recv_timeout(Duration::from_millis(100)) {
                Ok(Some(request)) => {
                    let weak = weak.clone();
                    std::thread::spawn(move || {
                        let _ = serve(weak, request);
                    });
                }
                Ok(None) => {}
                Err(_) => break,
            }
        }
    });
    Ok(())
}

fn respond(request: tiny_http::Request, status: u16, value: Value) -> Result<()> {
    let body = if status == 204 {
        String::new()
    } else {
        value.to_string()
    };
    let mut writer = request.into_writer();
    write!(
        writer,
        "HTTP/1.1 {status} Response\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    writer.write_all(body.as_bytes())?;
    writer.flush()?;
    Ok(())
}

fn serve(weak: Weak<Session>, mut incoming: tiny_http::Request) -> Result<()> {
    #[cfg(test)]
    if std::env::var_os("ANTISLOP_NATIVE_DEBUG").is_some() {
        eprintln!("OpenCode gateway: {} {}", incoming.method(), incoming.url());
    }
    let session = weak.upgrade().ok_or_else(|| invalid("session closed"))?;
    let expected = authorization(&session.gateway_password);
    if !incoming
        .headers()
        .iter()
        .any(|h| h.field.equiv("Authorization") && h.value.as_str() == expected)
    {
        return respond(
            incoming,
            401,
            json!({"name":"Unauthorized","data":{"message":"managed terminal authentication required"}}),
        );
    }
    let path = incoming.url().split('?').next().unwrap_or("").to_owned();
    let method = incoming.method().as_str().to_owned();
    let root = format!("/session/{}", session.identity);
    let result = (|| -> Result<(u16, Value)> {
        session.check_alive()?;
        // Query parameters from native clients must never override cwd. The
        // dedicated server is rooted at the managed worktree; omit all queries.
        require(
            !path.contains('%') && !path.contains(".."),
            "encoded or relative native routes are unsupported",
        )?;
        if method == "POST"
            && [format!("{root}/message"), format!("{root}/prompt_async")].contains(&path)
        {
            let mut bytes = vec![];
            incoming
                .as_reader()
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            require(bytes.len() <= 1024 * 1024, "native request exceeds 1 MiB")?;
            let body: Value = serde_json::from_slice(&bytes)?;
            require(
                body["agent"]
                    .as_str()
                    .is_none_or(|agent| agent == "antislop"),
                "the managed agent cannot be changed",
            )?;
            let text = text_input(&body["parts"])?;
            let id = body["messageID"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(crate::id);
            session.enqueue(&format!("native:{}:{id}", session.identity), &text)?;
            return Ok((204, Value::Null));
        }
        if method == "POST" && path == format!("{root}/abort") {
            session.interrupt()?;
            return Ok((200, json!(true)));
        }
        require(
            method == "GET",
            "this operation is disabled in an antislop managed session",
        )?;
        if path == "/project/global/directories" || path == "/permission" || path == "/question" {
            return Ok((200, json!([])));
        }
        if path == "/global/config" {
            return Ok((200, request(&session, "GET", "/config", None)?));
        }
        if path == "/agent" {
            let agents = request(&session, "GET", "/agent", None)?;
            return Ok((
                200,
                Value::Array(
                    agents
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter(|agent| agent["name"] == "antislop")
                        .cloned()
                        .collect(),
                ),
            ));
        }
        if path == "/config" {
            let mut config = request(&session, "GET", "/config", None)?;
            let managed = config["agent"]["antislop"].clone();
            config["agent"] = json!({"antislop":managed});
            config["default_agent"] = "antislop".into();
            return Ok((200, config));
        }
        if path == "/session" {
            return Ok((200, json!([request(&session, "GET", &root, None)?])));
        }
        if path == "/session/status" {
            let value = request(&session, "GET", &path, None)?;
            return Ok((
                200,
                json!({session.identity.clone():value[&session.identity]}),
            ));
        }
        let scoped = path == root
            || ["message", "todo", "diff"].iter().any(|tail| {
                path == format!("{root}/{tail}") || path.starts_with(&format!("{root}/{tail}/"))
            });
        require(
            scoped
                || [
                    "/global/health",
                    "/config",
                    "/config/providers",
                    "/provider",
                    "/provider/auth",
                    "/agent",
                    "/command",
                    "/path",
                    "/vcs",
                    "/project/current",
                    "/lsp",
                    "/formatter",
                    "/mcp",
                    "/experimental/console",
                    "/experimental/capabilities",
                    "/skill",
                ]
                .contains(&path.as_str()),
            "route is outside the managed session",
        )?;
        Ok((200, request(&session, "GET", &path, None)?))
    })();
    if method == "GET" && (path == "/event" || path == "/global/event") {
        let Backend::Opencode { endpoint, password } = &session.backend else {
            unreachable!()
        };
        let response = raw(endpoint, password, "GET", &path, None)?;
        #[cfg(test)]
        if std::env::var_os("ANTISLOP_NATIVE_DEBUG").is_some() {
            eprintln!("OpenCode SSE upstream connected");
        }
        let mut reader = Events {
            reader: BufReader::new(response.into_body().into_reader()),
            pending: std::io::Cursor::new(vec![]),
            identity: session.identity.clone(),
            stop: session.stop.clone(),
        };
        drop(session);
        // tiny_http's normal chunked response buffers 8 KiB. Flush each SSE
        // frame explicitly so a quiet live session cannot stall its TUI.
        let mut writer = incoming.into_writer();
        writer.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n")?;
        writer.flush()?;
        let mut buffer = [0; 8192];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            #[cfg(test)]
            if std::env::var_os("ANTISLOP_NATIVE_DEBUG").is_some() {
                eprintln!("OpenCode SSE frame bytes: {count}");
            }
            write!(writer, "{count:x}\r\n")?;
            writer.write_all(&buffer[..count])?;
            writer.write_all(b"\r\n")?;
            writer.flush()?;
        }
        writer.write_all(b"0\r\n\r\n")?;
        writer.flush()?;
        return Ok(());
    }
    match result {
        Ok((status, value)) => respond(incoming, status, value),
        Err(error) => {
            #[cfg(test)]
            if std::env::var_os("ANTISLOP_NATIVE_DEBUG").is_some() {
                eprintln!("OpenCode gateway error: {method} {path}: {error}");
            }
            respond(
                incoming,
                400,
                json!({"name":"ManagedSessionError","data":{"message":error.to_string()}}),
            )
        }
    }
}

struct Events {
    reader: BufReader<ureq::BodyReader<'static>>,
    pending: std::io::Cursor<Vec<u8>>,
    identity: String,
    stop: Arc<AtomicBool>,
}
impl Read for Events {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        loop {
            let count = self.pending.read(buffer)?;
            if count > 0 {
                return Ok(count);
            }
            if self.stop.load(Ordering::SeqCst) {
                return Ok(0);
            }
            let mut frame = String::new();
            let count = self
                .reader
                .by_ref()
                .take(256 * 1024)
                .read_line(&mut frame)?;
            if count == 0 {
                return Ok(0);
            }
            if let Some(data) = frame.strip_prefix("data:")
                && let Ok(event) = serde_json::from_str::<Value>(data.trim())
            {
                let payload = if event["payload"].is_object() {
                    &event["payload"]
                } else {
                    &event
                };
                let properties = &payload["properties"];
                #[cfg(test)]
                if std::env::var_os("ANTISLOP_NATIVE_DEBUG").is_some() {
                    eprintln!(
                        "OpenCode SSE event: {} status={} error={}",
                        payload["type"], properties["status"], properties["error"]
                    );
                }
                let id = properties["sessionID"]
                    .as_str()
                    .or(properties["info"]["sessionID"].as_str())
                    .or(properties["part"]["sessionID"].as_str())
                    .or(properties["info"]["id"].as_str());
                if id == Some(&self.identity)
                    || matches!(
                        payload["type"].as_str(),
                        Some("server.connected" | "server.heartbeat")
                    )
                {
                    frame.push('\n');
                    self.pending = std::io::Cursor::new(frame.into_bytes());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn http_session(
        core: &Arc<Core>,
        conversation: &str,
        directory: &Path,
        endpoint: String,
    ) -> Arc<Session> {
        let mut command = Command::new("sleep");
        command.arg("60");
        Arc::new(Session {
            core: Arc::downgrade(core),
            conversation: conversation.into(),
            role: "orchestrator".into(),
            spec: agent::Spec {
                provider: ProviderKind::Opencode,
                binary: "/bin/false".into(),
                args: vec![],
                model: None,
                effort: None,
                cwd: directory.into(),
                add_dirs: vec![],
                access: agent::Access::ReadOnly,
                resume: None,
            },
            signature: "fixture".into(),
            identity: "managed".into(),
            endpoint: String::new(),
            gateway_address: Mutex::new(None),
            gateway_password: crate::id(),
            backend: Backend::Opencode {
                endpoint,
                password: "private-server".into(),
            },
            stop: Arc::new(AtomicBool::new(false)),
            active: Mutex::new(None),
            completed: Mutex::new(std::collections::VecDeque::new()),
            terminal: Mutex::new(None),
            gateway_error: Mutex::new(None),
            started: AtomicBool::new(false),
            executing: AtomicBool::new(false),
            user_interrupted: AtomicBool::new(false),
            touched: Mutex::new(Instant::now()),
            scope_round: None,
            process: Process::spawn(command, None).unwrap(),
        })
    }
    #[test]
    fn a_failed_abort_stops_the_backend_and_preserves_its_failure_reason() {
        let directory = tempfile::tempdir_in("/tmp").unwrap();
        let config =
            crate::config::Config::from_toml(include_str!("../../../examples/antislop.toml"))
                .unwrap();
        let core = Core::open(&directory.path().join(".antislop"), config).unwrap();
        let conversation = core
            .store
            .create_conversation(directory.path().to_str().unwrap())
            .unwrap();
        core.store
            .start_turn(&conversation.id, "orchestrator", "opencode", 0, None)
            .unwrap();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", server.server_addr());
        let backend = std::thread::spawn(move || {
            let request = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap();
            assert_eq!(request.url(), "/session/managed/abort");
            request
                .respond(
                    tiny_http::Response::from_string("backend could not acknowledge abort")
                        .with_status_code(503),
                )
                .unwrap();
        });
        let session = http_session(&core, &conversation.id, directory.path(), endpoint);
        session.executing.store(true, Ordering::SeqCst);
        let error = session.interrupt().unwrap_err().to_string();
        assert!(
            error.contains("503") && error.contains("backend could not acknowledge abort"),
            "{error}"
        );
        assert!(session.stop.load(Ordering::SeqCst));
        assert!(session.process.status().unwrap().is_some());
        backend.join().unwrap();
    }
    #[test]
    fn a_lost_accepted_stream_never_resubmits_the_opencode_prompt() {
        let directory = tempfile::tempdir_in("/tmp").unwrap();
        let config =
            crate::config::Config::from_toml(include_str!("../../../examples/antislop.toml"))
                .unwrap();
        let core = Core::open(&directory.path().join(".antislop"), config).unwrap();
        let conversation = core
            .store
            .create_conversation(directory.path().to_str().unwrap())
            .unwrap();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", server.server_addr());
        let backend = std::thread::spawn(move || {
            let request = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .expect("expected backend request");
            assert_eq!(request.url(), "/event");
            let (accepted, submitted) = std::sync::mpsc::sync_channel(1);
            let stream = std::thread::spawn(move || {
                let mut writer = request.into_writer();
                writer.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").unwrap();
                let connected = "data: {\"type\":\"server.connected\",\"properties\":{}}\n\n";
                write!(writer, "{:x}\r\n{connected}\r\n", connected.len()).unwrap();
                writer.flush().unwrap();
                submitted.recv_timeout(Duration::from_secs(5)).unwrap();
                writer.write_all(b"0\r\n\r\n").unwrap();
                writer.flush().unwrap();
                // The prompt was accepted. Lose the event connection without
                // producing a result; no new prompt request is permissible.
            });
            let request = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .expect("expected backend request");
            assert_eq!(request.url(), "/session/managed/prompt_async");
            request.respond(tiny_http::Response::empty(204)).unwrap();
            accepted.send(()).unwrap();
            stream.join().unwrap();
            let request = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .expect("expected backend request");
            assert_eq!(request.url(), "/session/managed/abort");
            request
                .respond(tiny_http::Response::from_string("true"))
                .unwrap();
            assert!(
                server
                    .recv_timeout(Duration::from_millis(100))
                    .unwrap()
                    .is_none()
            );
        });
        let session = http_session(&core, &conversation.id, directory.path(), endpoint);
        let error = session
            .run("turn", "hello", &[], &AtomicBool::new(false), &mut |_| {})
            .unwrap_err()
            .to_string();
        assert!(error.contains(EXECUTION_FAILED));
        assert!(error.contains("event stream disconnected"));
        assert!(session.stop.load(Ordering::SeqCst));
        backend.join().unwrap();
    }
    #[test]
    fn explicit_interrupt_keeps_the_opencode_backend_and_records_control() {
        let directory = tempfile::tempdir_in("/tmp").unwrap();
        let config =
            crate::config::Config::from_toml(include_str!("../../../examples/antislop.toml"))
                .unwrap();
        let core = Core::open(&directory.path().join(".antislop"), config).unwrap();
        let conversation = core
            .store
            .create_conversation(directory.path().to_str().unwrap())
            .unwrap();
        let turn = core
            .store
            .start_turn(&conversation.id, "orchestrator", "opencode", 0, None)
            .unwrap();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", server.server_addr());
        let backend = std::thread::spawn(move || {
            let request = server.recv().unwrap();
            assert_eq!(request.url(), "/event");
            let (sent, submitted) = std::sync::mpsc::sync_channel::<String>(1);
            let (aborted, abort) = std::sync::mpsc::sync_channel(1);
            let stream = std::thread::spawn(move || {
                let mut writer = request.into_writer();
                writer.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"type\":\"server.connected\",\"properties\":{}}\n\n").unwrap();
                writer.flush().unwrap();
                let parent = submitted.recv_timeout(Duration::from_secs(5)).unwrap();
                let event = json!({"type":"message.updated","properties":{"info":{"id":"answer","sessionID":"managed","role":"assistant","parentID":parent}}});
                writeln!(writer, "data: {event}\n").unwrap();
                writer.flush().unwrap();
                abort.recv_timeout(Duration::from_secs(5)).unwrap();
                // Closing the stream races with the user's abort response.
            });
            let mut request = server.recv().unwrap();
            assert_eq!(request.url(), "/session/managed/prompt_async");
            let body: Value = serde_json::from_reader(request.as_reader()).unwrap();
            request.respond(tiny_http::Response::empty(204)).unwrap();
            sent.send(body["messageID"].as_str().unwrap().into())
                .unwrap();
            let request = server.recv().unwrap();
            assert_eq!(request.url(), "/session/managed/abort");
            aborted.send(()).unwrap();
            request
                .respond(tiny_http::Response::from_string("true"))
                .unwrap();
            stream.join().unwrap();
        });
        let session = http_session(&core, &conversation.id, directory.path(), endpoint);
        let running = session.clone();
        let (began, ready) = std::sync::mpsc::sync_channel(1);
        let run = std::thread::spawn(move || {
            running.run(
                &turn.id,
                "hello",
                &[],
                &AtomicBool::new(false),
                &mut |event| {
                    if matches!(event, agent::Event::ProviderTurn { .. }) {
                        began.send(()).unwrap();
                    }
                },
            )
        });
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        session.interrupt().unwrap();
        assert!(
            run.join()
                .unwrap()
                .unwrap_err()
                .to_string()
                .contains("native turn interrupted")
        );
        assert!(!session.stop.load(Ordering::SeqCst));
        session.check_alive().unwrap();
        let controls: u32 = core
            .store
            .db()
            .query_row(
                "SELECT count(*) FROM events WHERE kind='native_control'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(controls, 1);
        backend.join().unwrap();
    }
    #[test]
    fn global_sse_filters_foreign_sessions_and_preserves_managed_errors() {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", server.server_addr());
        let events = [
            json!({"directory":"/project","payload":{"type":"server.connected","properties":{}}}),
            json!({"directory":"/project","payload":{"type":"message.part.updated","properties":{"part":{"sessionID":"foreign","text":"private"}}}}),
            json!({"directory":"/project","payload":{"type":"message.part.updated","properties":{"part":{"sessionID":"managed","text":"live"}}}}),
            json!({"directory":"/project","payload":{"type":"session.error","properties":{"sessionID":"managed","error":{"message":"raw failure"}}}}),
        ];
        let body = events
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect::<String>();
        std::thread::spawn(move || {
            server
                .recv()
                .unwrap()
                .respond(tiny_http::Response::from_string(body).with_header(
                    tiny_http::Header::from_bytes("Content-Type", "text/event-stream").unwrap(),
                ))
                .unwrap();
        });
        let response = raw(&endpoint, "password", "GET", "/global/event", None).unwrap();
        let mut events = Events {
            reader: BufReader::new(response.into_body().into_reader()),
            pending: std::io::Cursor::new(vec![]),
            identity: "managed".into(),
            stop: Arc::new(AtomicBool::new(false)),
        };
        let mut text = String::new();
        events.read_to_string(&mut text).unwrap();
        assert!(text.contains("live"));
        assert!(text.contains("raw failure"));
        assert!(!text.contains("private"));
        assert!(!text.contains("foreign"));
    }
    #[test]
    fn http_gateway_authenticates_and_limits_sessions_and_mutations() {
        let directory = tempfile::tempdir_in("/tmp").unwrap();
        let mut config =
            crate::config::Config::from_toml(include_str!("../../../examples/antislop.toml"))
                .unwrap();
        let selected = config
            .resolve(
                crate::config::WorkKind::Orchestration,
                crate::config::RoleKind::Orchestrator,
            )
            .unwrap()
            .profile;
        let profile = config.profiles.get_mut(&selected).unwrap();
        profile.provider = ProviderKind::Opencode;
        profile.binary = Some("/bin/false".into());
        profile.model = None;
        profile.args.clear();
        profile.command = None;
        config.providers.insert(
            ProviderKind::Opencode,
            crate::config::ProviderConfig {
                binary: "/bin/false".into(),
                args: vec![],
                command: None,
            },
        );
        let core = Core::open(&directory.path().join(".antislop"), config).unwrap();
        let conversation = core
            .store
            .create_conversation(directory.path().to_str().unwrap())
            .unwrap();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", server.server_addr());
        let (started, first_started) = std::sync::mpsc::sync_channel(1);
        let (release, released) = std::sync::mpsc::sync_channel(1);
        let executions = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let executed = executions.clone();
        let backend = std::thread::spawn(move || {
            let request = server
                .recv_timeout(Duration::from_secs(5))
                .unwrap()
                .unwrap();
            assert_eq!(request.url(), "/session/managed");
            request
                .respond(tiny_http::Response::from_string("{\"id\":\"managed\"}"))
                .unwrap();
            let mut released = Some(released);
            let mut previous = None;
            for index in 1..=3 {
                let request = server
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                assert_eq!(request.url(), "/event");
                let (ready, start) = std::sync::mpsc::sync_channel::<String>(1);
                let gate = if index == 1 { released.take() } else { None };
                let started = started.clone();
                let stream = std::thread::spawn(move || {
                    let mut writer = request.into_writer();
                    writer.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n").unwrap();
                    let write_event = |writer: &mut dyn Write, event: Value| {
                        let frame = format!("data: {event}\n\n");
                        write!(writer, "{:x}\r\n{frame}\r\n", frame.len()).unwrap();
                        writer.flush().unwrap();
                    };
                    write_event(
                        &mut writer,
                        json!({"type":"server.connected","properties":{}}),
                    );
                    let user = start.recv_timeout(Duration::from_secs(5)).unwrap();
                    let message = format!("native-message-{index}");
                    write_event(
                        &mut writer,
                        json!({"type":"message.updated","properties":{"info":{"id":message,"sessionID":"managed","role":"assistant","parentID":user}}}),
                    );
                    write_event(
                        &mut writer,
                        json!({"type":"message.part.updated","properties":{"part":{"id":format!("start-{index}"),"sessionID":"managed","messageID":message,"type":"step-start"}}}),
                    );
                    write_event(
                        &mut writer,
                        json!({"type":"message.part.updated","properties":{"part":{"id":format!("text-{index}"),"sessionID":"managed","messageID":message,"type":"text","text":if index==1 {"attached answer"}else{"corrected answer"}}}}),
                    );
                    if let Some(gate) = gate {
                        started.send(()).unwrap();
                        gate.recv_timeout(Duration::from_secs(5)).unwrap();
                    }
                    write_event(
                        &mut writer,
                        json!({"type":"message.part.updated","properties":{"part":{"id":format!("finish-{index}"),"sessionID":"managed","messageID":message,"type":"step-finish","reason":"stop","tokens":{"input":3,"output":2}}}}),
                    );
                    writer.write_all(b"0\r\n\r\n").unwrap();
                    writer.flush().unwrap();
                });
                let mut request = server
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap();
                assert_eq!(request.url(), "/session/managed/prompt_async");
                let body: Value = serde_json::from_reader(request.as_reader()).unwrap();
                assert_eq!(body["agent"], "antislop");
                let prompt = body["parts"][0]["text"].as_str().unwrap();
                match index {
                    1 => assert_eq!(prompt, "hello"),
                    2 => {
                        assert!(prompt.contains("native correction one"));
                        assert!(
                            !prompt.contains("native correction two"),
                            "a stale request was silently expanded"
                        );
                    }
                    _ => assert!(prompt.contains("native correction two")),
                }
                assert_ne!(previous.as_ref(), Some(&body["messageID"]));
                previous = Some(body["messageID"].clone());
                executed.fetch_add(1, Ordering::SeqCst);
                request.respond(tiny_http::Response::empty(204)).unwrap();
                ready
                    .send(body["messageID"].as_str().unwrap().into())
                    .unwrap();
                stream.join().unwrap();
            }
            assert!(
                server
                    .recv_timeout(Duration::from_millis(100))
                    .unwrap()
                    .is_none(),
                "a repeated message caused an extra turn"
            );
        });
        let mut session = http_session(&core, &conversation.id, directory.path(), endpoint);
        let profile = core.config_snapshot().resolve(
            crate::config::WorkKind::Orchestration,
            crate::config::RoleKind::Orchestrator,
        );
        {
            let native = Arc::get_mut(&mut session).unwrap();
            native.spec.effort = profile.as_ref().unwrap().native_effort.clone();
            native.signature = format!("orchestrator:{:?}", native.spec);
        }
        core.native.sessions.lock().unwrap().insert(
            (conversation.id.clone(), "orchestrator".into()),
            session.clone(),
        );
        gateway(&session).unwrap();
        let endpoint = session.gateway_address.lock().unwrap().clone().unwrap();
        assert!(ureq::get(format!("{endpoint}/session")).call().is_err());
        let auth = authorization(&session.gateway_password);
        for path in [
            "/session/foreign",
            "/session/managed/children",
            "/file/content?path=/foreign",
        ] {
            assert!(
                ureq::get(format!("{endpoint}{path}"))
                    .header("Authorization", &auth)
                    .call()
                    .is_err()
            );
        }
        for path in ["/session", "/config", "/session/managed/shell"] {
            assert!(
                ureq::post(format!("{endpoint}{path}"))
                    .header("Authorization", &auth)
                    .send_json(json!({}))
                    .is_err()
            );
        }
        assert!(
            ureq::post(format!("{endpoint}/session/managed/prompt_async"))
                .header("Authorization", &auth)
                .send_json(json!({"parts":[{"type":"image","url":"file:///foreign"}]}))
                .is_err()
        );
        let mut response = ureq::get(format!("{endpoint}/session"))
            .header("Authorization", auth)
            .call()
            .unwrap();
        let value: Value = response.body_mut().read_json().unwrap();
        assert_eq!(value, json!([{"id":"managed"}]));
        let running = core.clone();
        let owner = conversation.id.clone();
        let cwd = directory.path().to_path_buf();
        let first = std::thread::spawn(move || {
            running.turn(
                crate::lean::TurnSetup {
                    conversation: &owner,
                    plan_id: None,
                    role: "orchestrator".into(),
                    profile,
                    cwd,
                    add_dirs: vec![],
                    access: agent::Access::ReadOnly,
                    upto_message: 0,
                    session_policy: crate::lean::SessionPolicy::Role,
                },
                &AtomicBool::new(false),
                |_| "hello".into(),
            )
        });
        first_started.recv_timeout(Duration::from_secs(5)).unwrap();
        let role_lock = core.turn_lock(&conversation.id, "orchestrator");
        for (id, text) in [
            ("first", "native correction one"),
            ("second", "native correction two"),
            ("first", "native correction one"),
        ] {
            let response=ureq::post(format!("{endpoint}/session/managed/prompt_async")).header("Authorization",authorization(&session.gateway_password)).send_json(json!({"messageID":id,"agent":"antislop","parts":[{"type":"text","text":text}]})).unwrap();
            assert_eq!(response.status().as_u16(), 204);
            if id == "first" && core.store.mail(&conversation.id).unwrap().len() == 1 {
                let deadline = Instant::now() + Duration::from_secs(5);
                while Arc::strong_count(&role_lock) < 3 {
                    assert!(
                        Instant::now() < deadline,
                        "orchestrator did not prepare its first request"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
        assert_eq!(executions.load(Ordering::SeqCst), 1);
        assert!(session.executing.load(Ordering::SeqCst));
        let mail = core.store.mail(&conversation.id).unwrap();
        assert_eq!(mail.len(), 2);
        assert!(mail.iter().all(|message| message.delivered_turn.is_none()));
        release.send(()).unwrap();
        assert_eq!(
            first.join().unwrap().unwrap().as_deref(),
            Some("attached answer")
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !core.is_idle(&conversation.id) {
            assert!(
                Instant::now() < deadline,
                "queued OpenCode corrections did not complete"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(executions.load(Ordering::SeqCst), 3);
        let mail = core.store.mail(&conversation.id).unwrap();
        assert_eq!(mail.len(), 2);
        assert!(mail.iter().all(|message| message.delivered_turn.is_some()));
        assert_ne!(mail[0].delivered_turn, mail[1].delivered_turn);
        let sessions = core.store.sessions(&conversation.id).unwrap();
        assert_eq!(sessions.len(), 3);
        assert!(
            sessions
                .iter()
                .all(|session| session.session_id.as_deref() == Some("managed")
                    && session.backend.as_deref() == Some("opencode_server"))
        );
        let turn = core
            .store
            .last_turn(&conversation.id, "orchestrator")
            .unwrap()
            .unwrap();
        assert_eq!(turn.status, store::TurnStatus::Done);
        assert_eq!(turn.session_id.as_deref(), Some("managed"));
        let context = core
            .store
            .run_context(&conversation.id, &turn.id)
            .unwrap()
            .unwrap();
        assert_eq!(context.backend, store::RunBackend::OpencodeServer);
        assert_eq!(context.provider_session_id.as_deref(), Some("managed"));
        assert_eq!(
            context.provider_turn_id.as_deref(),
            Some("native-message-3")
        );
        let usage:u32=core.store.db().query_row("SELECT count(*) FROM events WHERE turn_id=?1 AND kind='agent' AND json_extract(payload,'$.type')='usage' AND json_extract(payload,'$.input')=3 AND json_extract(payload,'$.output')=2",[&turn.id],|row|row.get(0)).unwrap();
        assert_eq!(usage, 1);
        backend.join().unwrap();
    }
}
