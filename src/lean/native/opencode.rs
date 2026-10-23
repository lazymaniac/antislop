use super::*;
use base64::Engine;
use std::io::BufRead;
use std::io::Write;

fn http_agent(stream: bool) -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(if stream {
            None
        } else {
            Some(Duration::from_secs(15))
        })
        .timeout_connect(Some(Duration::from_secs(2)))
        .timeout_recv_body(Some(Duration::from_secs(20)))
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
    response.map_err(invalid)
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
    _turn: &str,
    prompt: &str,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(agent::Event),
) -> Result<agent::Outcome> {
    let Backend::Opencode { endpoint, password } = &session.backend else {
        unreachable!()
    };
    let mut spec = session.spec.clone();
    spec.resume = Some(session.identity.clone());
    let configured = agent::command(&spec, prompt)?;
    let args: Vec<_> = configured.get_args().map(|arg| arg.to_owned()).collect();
    let mut command = Command::new(&spec.binary);
    command
        .arg("run")
        .arg("--attach")
        .arg(endpoint)
        .args(&args[1..])
        .current_dir(&spec.cwd)
        .envs(
            configured
                .get_envs()
                .filter_map(|(key, value)| value.map(|value| (key, value))),
        )
        .env("OPENCODE_SERVER_PASSWORD", password)
        .env("OPENCODE_SERVER_USERNAME", "opencode");
    let mut bound = false;
    let result = agent::run_process(
        &spec,
        cancel,
        &mut |event| {
            if matches!(event, agent::Event::ProviderTurn { .. }) {
                if bound {
                    return;
                }
                bound = true;
            }
            emit(event);
        },
        command,
    );
    if result.is_err() {
        let _ = request(
            session,
            "POST",
            &format!("/session/{}/abort", session.identity),
            Some(json!({})),
        );
    }
    if session.user_interrupted.load(Ordering::SeqCst) {
        return Err(invalid("native turn interrupted"));
    }
    let result = result?;
    require(
        result.session.as_deref() == Some(&session.identity),
        "OpenCode attached turn returned a different session",
    )?;
    Ok(result)
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
    request.respond(
        tiny_http::Response::from_string(value.to_string())
            .with_status_code(status)
            .with_header(
                tiny_http::Header::from_bytes("Content-Type", "application/json").expect("header"),
            ),
    )?;
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
        writer.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n")?;
        writer.flush()?;
        let mut buffer = [0; 8192];
        loop {
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            writer.write_all(&buffer[..count])?;
            writer.flush()?;
        }
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
        let directory = tempfile::tempdir_in("/private/tmp").unwrap();
        let config =
            crate::config::Config::from_toml(include_str!("../../../examples/antislop.toml"))
                .unwrap();
        let core = Core::open(&directory.path().join(".antislop"), config).unwrap();
        let script = directory.path().join("fake-opencode");
        std::fs::write(&script,"#!/bin/sh\nprintf '%s\\n' \"$@\" > .antislop/attached-args\necho '{\"type\":\"step_start\",\"sessionID\":\"managed\",\"part\":{\"messageID\":\"native-message\"}}'\necho '{\"type\":\"text\",\"part\":{\"text\":\"attached answer\"}}'\necho '{\"type\":\"step_finish\",\"part\":{\"reason\":\"stop\"}}'\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let conversation = core
            .store
            .create_conversation(directory.path().to_str().unwrap())
            .unwrap();
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}", server.server_addr());
        std::thread::spawn(move || {
            if let Ok(Some(request)) = server.recv_timeout(Duration::from_secs(5)) {
                assert_eq!(request.url(), "/session/managed");
                request
                    .respond(tiny_http::Response::from_string("{\"id\":\"managed\"}"))
                    .unwrap();
            }
        });
        let mut command = Command::new("sleep");
        command.arg("60");
        let session = Arc::new(Session {
            core: Arc::downgrade(&core),
            conversation: conversation.id,
            role: "orchestrator".into(),
            spec: agent::Spec {
                provider: ProviderKind::Opencode,
                binary: script,
                args: vec![],
                model: None,
                effort: None,
                cwd: directory.path().into(),
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
            process: Process::spawn(command, None).unwrap(),
        });
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
        let mut events = vec![];
        let result = session
            .run(
                "turn",
                "hello",
                &[],
                &AtomicBool::new(false),
                &mut |event| events.push(event),
            )
            .unwrap();
        assert_eq!(result.text, "attached answer");
        assert_eq!(result.session.as_deref(), Some("managed"));
        assert!(
            events.iter().any(
                |event| matches!(event,agent::Event::ProviderTurn{id} if id=="native-message")
            )
        );
        let args =
            std::fs::read_to_string(directory.path().join(".antislop/attached-args")).unwrap();
        assert!(args.starts_with("run\n--attach\n"));
        assert!(args.contains("--session\nmanaged\n"));
        assert!(args.contains("--agent\nantislop\n"));
    }
}
