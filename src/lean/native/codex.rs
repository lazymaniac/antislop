use super::*;
use std::collections::VecDeque;
use std::os::unix::net::{UnixListener, UnixStream};
use tungstenite::{Message, WebSocket};
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

fn websocket_config() -> tungstenite::protocol::WebSocketConfig {
    // Startup catalogs may exceed tungstenite's default 16 MiB frame size.
    tungstenite::protocol::WebSocketConfig::default()
        .max_frame_size(Some(64 * 1024 * 1024))
        .max_message_size(Some(64 * 1024 * 1024))
}

pub(crate) struct Rpc {
    socket: WebSocket<UnixStream>,
    next: u64,
    pending: VecDeque<Value>,
}

fn timed_out(error: &tungstenite::Error) -> bool {
    matches!(error, tungstenite::Error::Io(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut))
}

impl Rpc {
    fn raw(path: &Path) -> Result<Self> {
        let stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
        stream.set_write_timeout(Some(HANDSHAKE_TIMEOUT))?;
        let (socket, _) = tungstenite::client::client_with_config(
            "ws://localhost/",
            stream,
            Some(websocket_config()),
        )
        .map_err(|e| invalid(format!("Codex upstream handshake: {e}")))?;
        socket
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(50)))?;
        socket
            .get_ref()
            .set_write_timeout(Some(Duration::from_secs(2)))?;
        Ok(Self {
            socket,
            next: 1,
            pending: VecDeque::new(),
        })
    }
    pub(super) fn connect(path: &Path) -> Result<Self> {
        let mut rpc = Self::raw(path)?;
        rpc.call(
            "initialize",
            json!({"clientInfo":{"name":"antislop","version":env!("CARGO_PKG_VERSION")}}),
        )?;
        rpc.send(json!({"method":"initialized","params":{}}))?;
        Ok(rpc)
    }
    fn send(&mut self, value: Value) -> Result<()> {
        self.socket
            .send(Message::Text(value.to_string().into()))
            .map_err(|e| invalid(format!("Codex request write: {e}")))
    }
    fn read(&mut self) -> Result<Option<Value>> {
        match self.socket.read() {
            Ok(Message::Text(text)) => Ok(Some(serde_json::from_str(&text)?)),
            Ok(Message::Close(_)) => Err(invalid("Codex app-server disconnected")),
            Ok(_) => Ok(None),
            Err(error) if timed_out(&error) => Ok(None),
            Err(error) => Err(invalid(format!("Codex upstream read: {error}"))),
        }
    }
    pub(super) fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        let id = format!("antislop_rpc_{}", self.next);
        self.next += 1;
        self.send(json!({"id":id,"method":method,"params":params}))?;
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            require(Instant::now() < until, &format!("Codex {method} timed out"))?;
            let Some(message) = self.read()? else {
                continue;
            };
            if message["id"] == id {
                if let Some(error) = message.get("error") {
                    if error["code"] == -32601 {
                        return Err(invalid(format!(
                            "{UNSUPPORTED_PROTOCOL}Codex {method}: {error}"
                        )));
                    }
                    return Err(invalid(format!("Codex {method}: {error}")));
                }
                return Ok(message["result"].clone());
            }
            require(
                self.pending.len() < 4096,
                "Codex notification queue exceeded its limit",
            )?;
            self.pending.push_back(message);
        }
    }
    fn event(&mut self) -> Result<Option<Value>> {
        if let Some(message) = self.pending.pop_front() {
            return Ok(Some(message));
        }
        self.read()
    }
}

pub(super) fn thread_params(spec: &agent::Spec, id: Option<&str>) -> Value {
    let mut params = json!({"cwd":spec.cwd,"approvalPolicy":"never",
        "sandbox":agent::codex_sandbox(spec.access == agent::Access::ReadOnly),
        "config":{"features.multi_agent":false}});
    if let Some(model) = &spec.model {
        params["model"] = model.clone().into();
    }
    if let Some(effort) = &spec.effort {
        params["config"]["model_reasoning_effort"] = effort.clone().into();
    }
    if !spec.add_dirs.is_empty() {
        params["config"]["sandbox_workspace_write.writable_roots"] = json!(spec.add_dirs);
    }
    if let Some(id) = id {
        params["threadId"] = id.into();
    }
    params
}

pub(super) fn start(
    core: &Core,
    directory: &Path,
    id: &str,
    spec: &agent::Spec,
) -> Result<(Process, Backend, String, String)> {
    let socket = directory.join(format!("{}.sock", &id[..16]));
    let gateway = directory.join(format!("{}.gate", &id[..16]));
    require(
        socket.as_os_str().len() < 100 && gateway.as_os_str().len() < 100,
        "native Codex socket path exceeds the platform limit; use a shorter project path",
    )?;
    let mut command = Command::new(&spec.binary);
    command
        .current_dir(&spec.cwd)
        .args(["app-server", "--listen"])
        .arg(format!("unix://{}", socket.display()))
        .args(["-c", "features.multi_agent=false"]);
    let process = Process::spawn(command, None)?;
    process.register(core, "codex")?;
    let until = Instant::now() + Duration::from_secs(10);
    let mut rpc = loop {
        match Rpc::connect(&socket) {
            Ok(rpc) => break rpc,
            Err(Error::Io(error))
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                ) => {}
            Err(error) => return Err(error),
        }
        require(
            Instant::now() < until,
            &format!(
                "Codex app-server did not open its socket: {}",
                process.diagnostics()
            ),
        )?;
        if let Some(status) = process.status()? {
            return Err(invalid(format!(
                "Codex app-server exited ({status}): {}",
                process.diagnostics()
            )));
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    let method = if spec.resume.is_some() {
        "thread/resume"
    } else {
        "thread/start"
    };
    let response = rpc.call(method, thread_params(spec, spec.resume.as_deref()))?;
    let identity = response["thread"]["id"]
        .as_str()
        .ok_or_else(|| invalid("Codex did not return a thread id"))?
        .to_owned();
    require(
        spec.resume.as_ref().is_none_or(|id| *id == identity),
        "Codex resumed a different thread",
    )?;
    Ok((
        process,
        Backend::Codex {
            socket,
            rpc: Box::new(Mutex::new(rpc)),
        },
        identity,
        format!("unix://{}", gateway.display()),
    ))
}

pub(super) fn run(
    session: &Session,
    rpc: &mut Rpc,
    antislop_turn: &str,
    prompt: &str,
    mail: &[store::Mail],
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(agent::Event),
) -> Result<agent::Outcome> {
    emit(agent::Event::Session {
        id: session.identity.clone(),
    });
    let response = rpc.call("turn/start", json!({"threadId":session.identity,
        "input":[{"type":"text","text":prompt}], "clientUserMessageId":antislop_turn,
        "cwd":session.spec.cwd,"approvalPolicy":"never", "model":session.spec.model,
        "effort":session.spec.effort,
        "sandboxPolicy":if session.spec.access == agent::Access::ReadOnly {json!({"type":"readOnly"})}
            else {json!({"type":"dangerFullAccess"})}}))?;
    let native_turn = response["turn"]["id"]
        .as_str()
        .ok_or_else(|| invalid("Codex did not return a turn id"))?
        .to_owned();
    session.started.store(true, Ordering::SeqCst);
    let core = session
        .core
        .upgrade()
        .ok_or_else(|| invalid("daemon stopped"))?;
    core.store
        .bind_provider_turn(&session.conversation, antislop_turn, &native_turn)?;
    *session.active.lock().map_err(invalid)? = Some(Active {
        turn: response["turn"].clone(),
        mail: mail.iter().map(|m| m.id).collect(),
    });
    let until = Instant::now() + Duration::from_secs(30 * 60);
    let mut interrupted = None;
    let mut final_text = String::new();
    let mut items = std::collections::HashSet::new();
    loop {
        require(
            Instant::now() < until,
            "Codex native turn timed out after 30 minutes",
        )?;
        if cancel.load(Ordering::SeqCst) && interrupted.is_none() {
            rpc.call(
                "turn/interrupt",
                json!({"threadId":session.identity,"turnId":native_turn}),
            )?;
            interrupted = Some(Instant::now());
        }
        require(
            interrupted.is_none_or(|at| at.elapsed() < Duration::from_secs(5)),
            "Codex did not acknowledge cancellation",
        )?;
        let Some(event) = rpc.event()? else {
            continue;
        };
        if event.get("id").is_some() && event.get("method").is_some() {
            rpc.send(json!({"id":event["id"],"error":{"code":-32601,"message":"approval and interactive tool requests are disabled in managed sessions"}}))?;
            continue;
        }
        let params = &event["params"];
        if params["threadId"]
            .as_str()
            .is_some_and(|id| id != session.identity)
            || params["turnId"]
                .as_str()
                .is_some_and(|id| id != native_turn)
        {
            continue;
        }
        match event["method"].as_str().unwrap_or("") {
            "item/agentMessage/delta" => {
                if let Some(delta) = params["delta"].as_str() {
                    emit(agent::Event::Text {
                        delta: delta.into(),
                    });
                }
            }
            "item/reasoning/summaryTextDelta" | "item/reasoning/textDelta" => {
                if let Some(delta) = params["delta"].as_str() {
                    emit(agent::Event::Reasoning {
                        delta: delta.into(),
                    });
                }
            }
            "item/started" => {
                let item = &params["item"];
                if item["type"] != "agentMessage" && item["type"] != "userMessage" {
                    emit(agent::Event::Tool {
                        name: item["type"].as_str().unwrap_or("tool").into(),
                        detail: agent::clip_tail(item["command"].as_str().unwrap_or(""), 240),
                    });
                }
            }
            "item/completed" => {
                let item = &params["item"];
                let id = item["id"].as_str().unwrap_or("").to_owned();
                if !items.insert(id) {
                    continue;
                }
                if item["type"] == "agentMessage" {
                    if let Some(text) = item["text"].as_str() {
                        final_text = text.into();
                    }
                } else if let Some(output) = item["aggregatedOutput"].as_str() {
                    emit(agent::Event::Output {
                        name: item["type"].as_str().unwrap_or("tool").into(),
                        text: agent::clip_tail(output, 8192),
                        exit_code: item["exitCode"].as_i64(),
                    });
                }
            }
            "thread/tokenUsage/updated" => {
                let usage = &params["tokenUsage"]["last"];
                emit(agent::Event::Usage {
                    input: usage["inputTokens"].as_u64(),
                    output: usage["outputTokens"].as_u64(),
                });
            }
            "turn/completed" if params["turn"]["id"] == native_turn => {
                if params["turn"]["status"] == "interrupted" {
                    return Err(invalid("native turn interrupted"));
                }
                require(
                    params["turn"]["status"] == "completed",
                    &format!(
                        "Codex native turn {}: {}",
                        params["turn"]["status"], params["turn"]["error"]
                    ),
                )?;
                return Ok(agent::Outcome {
                    session: Some(session.identity.clone()),
                    text: final_text,
                });
            }
            "error" if params["willRetry"] == false => {
                return Err(invalid(format!("Codex: {}", params["error"])));
            }
            _ => {}
        }
    }
}

pub(super) fn gateway(session: &Arc<Session>) -> Result<()> {
    let path = session
        .endpoint
        .strip_prefix("unix://")
        .expect("unix gateway");
    let listener = UnixListener::bind(path)?;
    listener.set_nonblocking(true)?;
    let weak = Arc::downgrade(session);
    let stop = session.stop.clone();
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((stream, _)) => {
                    let weak = weak.clone();
                    std::thread::spawn(move || {
                        if let Err(error) = connection(weak.clone(), stream)
                            && let Some(session) = weak.upgrade()
                        {
                            session.attachment_failed(&error);
                        }
                    });
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(25))
                }
                Err(_) => break,
            }
        }
    });
    Ok(())
}

fn client_send(client: &mut WebSocket<UnixStream>, value: Value) -> Result<()> {
    let text = value.to_string();
    #[cfg(test)]
    if std::env::var_os("ANTISLOP_NATIVE_DEBUG").is_some() {
        eprintln!("Codex gateway response bytes: {}", text.len());
    }
    match client.send(Message::Text(text.into())) {
        Ok(()) => return Ok(()),
        Err(error) if timed_out(&error) => {}
        Err(error) => return Err(invalid(format!("Codex gateway write: {error}"))),
    }
    let until = Instant::now() + Duration::from_secs(5);
    loop {
        match client.flush() {
            Ok(()) => return Ok(()),
            Err(error) if timed_out(&error) && Instant::now() < until => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(error) => return Err(invalid(format!("Codex gateway flush: {error}"))),
        }
    }
}

pub(super) fn delivered_turn(session: &Session, upstream: &mut Rpc, turn: &str) -> Result<Value> {
    let core = session
        .core
        .upgrade()
        .ok_or_else(|| invalid("daemon stopped"))?;
    let context = core
        .store
        .run_context(&session.conversation, turn)?
        .ok_or_else(|| invalid("delivered native message has no run context"))?;
    require(
        context.provider_thread_id.as_deref() == Some(&session.identity),
        "delivered message belongs to another native thread; inspect JSON history",
    )?;
    let native = context
        .provider_turn_id
        .ok_or_else(|| invalid("delivered message has no native turn identifier"))?;
    let mut cursor = Value::Null;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        require(
            Instant::now() < deadline,
            "native receipt history lookup timed out; retry or inspect JSON history",
        )?;
        let page=upstream.call("thread/turns/list",json!({"threadId":session.identity,"cursor":cursor,"limit":10,"sortDirection":"desc","itemsView":"full"}))?;
        if let Some(turn) = page["data"]
            .as_array()
            .and_then(|turns| turns.iter().find(|turn| turn["id"] == native))
        {
            return Ok(turn.clone());
        }
        let next = page["nextCursor"].clone();
        require(
            !next.is_null() && next != cursor,
            "delivered turn is not available in native history; retry or inspect JSON history",
        )?;
        cursor = next;
    }
}

fn check_steer_target(session: &Session, expected: &Value) -> Result<()> {
    let active = session
        .active
        .lock()
        .map_err(invalid)?
        .as_ref()
        .map(|active| active.turn["id"].clone());
    if let Some(active) = active {
        return require(
            active == *expected,
            "native steer target is no longer the active turn",
        );
    }
    // The interactive CLI can still show the just-completed turn as active
    // while its completion notification is in transit. Queue that correction
    // against the latest run; never mutate or restart the completed execution.
    let core = session
        .core
        .upgrade()
        .ok_or_else(|| invalid("daemon stopped"))?;
    let latest = core
        .store
        .last_turn(&session.conversation, &session.role)?
        .ok_or_else(|| invalid("native steer has no target turn"))?;
    let context = core.store.run_context(&session.conversation, &latest.id)?;
    require(
        context.is_some_and(|context| {
            context.provider_thread_id.as_deref() == Some(&session.identity)
                && context.provider_turn_id.as_deref() == expected.as_str()
                && expected.is_string()
        }),
        "native steer target is not the latest turn in this session",
    )
}

fn connection(weak: Weak<Session>, stream: UnixStream) -> Result<()> {
    stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT))?;
    stream.set_write_timeout(Some(HANDSHAKE_TIMEOUT))?;
    let mut client = tungstenite::accept_with_config(stream, Some(websocket_config()))
        .map_err(|e| invalid(format!("Codex gateway handshake: {e}")))?;
    client
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(25)))?;
    client
        .get_ref()
        .set_write_timeout(Some(Duration::from_secs(2)))?;
    let session = weak.upgrade().ok_or_else(|| invalid("session closed"))?;
    let Backend::Codex { socket, .. } = &session.backend else {
        unreachable!()
    };
    let mut upstream = Rpc::raw(socket)?;
    let connection_id = crate::id();
    let mut queued: Vec<(Value, u64, bool)> = vec![];
    drop(session);
    loop {
        let session = weak.upgrade().ok_or_else(|| invalid("session closed"))?;
        session.check_alive()?;
        match client.read() {
            Ok(Message::Text(text)) => {
                let mut message: Value = serde_json::from_str(&text)?;
                let id = message["id"].clone();
                let method = message["method"].as_str().unwrap_or("").to_owned();
                #[cfg(test)]
                if std::env::var_os("ANTISLOP_NATIVE_DEBUG").is_some() {
                    eprintln!("Codex gateway request: {method}");
                }
                let params = &message["params"];
                let foreign = params["threadId"]
                    .as_str()
                    .is_some_and(|id| id != session.identity);
                let result = if foreign {
                    Err(invalid("cannot change the managed thread"))
                } else {
                    match method.as_str() {
                        "turn/start" | "turn/steer" => {
                            text_input(&params["input"]).and_then(|text| {
                                let steer = method == "turn/steer";
                                if steer {
                                    check_steer_target(&session, &params["expectedTurnId"])?;
                                }
                                require(queued.len() < 32, "too many queued terminal requests")?;
                                let operation = if let Some(message) = params["clientUserMessageId"]
                                    .as_str()
                                    .filter(|id| !id.is_empty())
                                {
                                    require(
                                        message.len() <= 256,
                                        "native client message identifier exceeds 256 bytes",
                                    )?;
                                    format!("native:message:{message}")
                                } else {
                                    format!("native:{connection_id}:{id}")
                                };
                                let receipt = session.enqueue(&operation, &text)?;
                                let mail = receipt["mail"]
                                    .as_u64()
                                    .ok_or_else(|| invalid("missing mail receipt"))?;
                                let cached = session
                                    .completed
                                    .lock()
                                    .map_err(invalid)?
                                    .iter()
                                    .any(|(id, _)| *id == mail);
                                if let Some(turn) = receipt["delivered_turn"].as_str()
                                    && !cached
                                {
                                    // A daemon restart or the bounded in-memory
                                    // cache expiring must not lose a durable ack.
                                    let turn = delivered_turn(&session, &mut upstream, turn)?;
                                    let mut completed =
                                        session.completed.lock().map_err(invalid)?;
                                    completed.push_back((mail, turn));
                                    while completed.len() > 128 {
                                        completed.pop_front();
                                    }
                                }
                                if !queued.iter().any(|(pending, _, _)| pending == &id) {
                                    queued.push((id.clone(), mail, steer));
                                }
                                Ok(false)
                            })
                        }
                        "turn/interrupt" => params["turnId"]
                            .as_str()
                            .ok_or_else(|| invalid("native interrupt requires a turnId"))
                            .and_then(|id| session.interrupt_turn(Some(id)))
                            .map(|()| true),
                        "thread/resume" => {
                            message["params"] =
                                thread_params(&session.spec, Some(&session.identity));
                            upstream.send(message.clone()).map(|()| false)
                        }
                        "thread/list" => {
                            let response = upstream.call(
                                "thread/read",
                                json!({"threadId":session.identity,"includeTurns":false}),
                            )?;
                            client_send(
                                &mut client,
                                json!({"id":id,"result":{"data":[response["thread"]],"nextCursor":null,"backwardsCursor":null}}),
                            )?;
                            Ok(false)
                        }
                        "thread/loaded/list" => {
                            client_send(
                                &mut client,
                                json!({"id":id,"result":{"data":[session.identity],"nextCursor":null}}),
                            )?;
                            Ok(false)
                        }
                        "initialize"
                        | "initialized"
                        | "thread/read"
                        | "thread/turns/list"
                        | "thread/items/list"
                        | "thread/goal/get"
                        | "model/list"
                        | "config/read"
                        | "configRequirements/read"
                        | "collaborationMode/list"
                        | "hooks/list"
                        | "plugin/list"
                        | "app/installed"
                        | "account/bedrock/checkGovCloudRequirements"
                        | "account/read"
                        | "account/rateLimits/read"
                        | "mcpServerStatus/list"
                        | "skills/list"
                        | "app/list" => upstream.send(message.clone()).map(|()| false),
                        _ => Err(invalid(format!(
                            "{method} is not supported in an antislop managed session"
                        ))),
                    }
                };
                match result {
                    Ok(true) => {
                        client_send(&mut client, json!({"id":id,"result":{}}))?;
                    }
                    Err(error) if !id.is_null() => {
                        #[cfg(test)]
                        if std::env::var_os("ANTISLOP_NATIVE_DEBUG").is_some() {
                            eprintln!("Codex gateway rejected {method}: {error}");
                        }
                        client_send(
                            &mut client,
                            json!({"id":id,"error":{"code":-32602,"message":error.to_string()}}),
                        )?;
                    }
                    _ => {}
                }
            }
            Ok(Message::Close(_)) => return Ok(()),
            Ok(_) => {}
            Err(error) if timed_out(&error) => {}
            Err(error) => return Err(invalid(format!("Codex gateway read: {error}"))),
        }
        {
            let active = session.active.lock().map_err(invalid)?;
            let completed = session.completed.lock().map_err(invalid)?;
            let mut remaining = vec![];
            for (id, mail, steer) in queued.drain(..) {
                let turn = active
                    .as_ref()
                    .filter(|a| a.mail.contains(&mail))
                    .map(|a| &a.turn)
                    .or_else(|| {
                        completed
                            .iter()
                            .find(|(m, _)| *m == mail)
                            .map(|(_, turn)| turn)
                    });
                if let Some(turn) = turn {
                    let result = if steer {
                        json!({"turnId":turn["id"]})
                    } else {
                        json!({"turn":turn})
                    };
                    client_send(&mut client, json!({"id":id,"result":result}))?;
                    if turn["status"] != "inProgress" {
                        client_send(
                            &mut client,
                            json!({"method":"turn/completed","params":{"threadId":session.identity,"turn":turn}}),
                        )?;
                    }
                } else {
                    remaining.push((id, mail, steer));
                }
            }
            queued = remaining;
        }
        if let Some(message) = upstream.event()? {
            let foreign = message["params"]["threadId"]
                .as_str()
                .is_some_and(|id| id != session.identity);
            if !foreign {
                client_send(&mut client, message)?;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_catalog_can_exceed_sixteen_megabytes() {
        let directory = tempfile::tempdir_in("/tmp").unwrap();
        let path = directory.path().join("catalog.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let length = 17 * 1024 * 1024;
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut socket = tungstenite::accept(stream).unwrap();
            socket
                .send(Message::Text(
                    json!({"catalog":"x".repeat(length)}).to_string().into(),
                ))
                .unwrap();
        });
        let mut rpc = Rpc::raw(&path).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(message) = rpc.read().unwrap() {
                assert_eq!(message["catalog"].as_str().unwrap().len(), length);
                break;
            }
            assert!(Instant::now() < deadline, "catalog response timed out");
        }
        server.join().unwrap();
    }
    use std::sync::atomic::AtomicU64;
    struct Fixture {
        directory: tempfile::TempDir,
        core: Arc<Core>,
        session: Arc<Session>,
        executions: Arc<AtomicU64>,
        shutdown: Arc<AtomicBool>,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.shutdown.store(true, Ordering::SeqCst);
        }
    }
    fn fixture(fail: bool) -> Fixture {
        let directory = tempfile::tempdir_in("/tmp").unwrap();
        let state = directory.path().join(".antislop");
        let mut config =
            crate::config::Config::from_toml(include_str!("../../../examples/antislop.toml"))
                .unwrap();
        config
            .providers
            .get_mut(&ProviderKind::Codex)
            .unwrap()
            .binary = "/bin/false".into();
        let core = Core::open(&state, config).unwrap();
        let conversation = core
            .store
            .create_conversation(directory.path().to_str().unwrap())
            .unwrap();
        let socket = directory.path().join("server.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let executions = Arc::new(AtomicU64::new(0));
        let executed = executions.clone();
        let shutdown = Arc::new(AtomicBool::new(false));
        let stopped = shutdown.clone();
        let interrupted = Arc::new(AtomicBool::new(false));
        std::thread::spawn(move || {
            while !stopped.load(Ordering::SeqCst) {
                let stream = match listener.accept() {
                    Ok((s, _)) => s,
                    Err(_) => {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                };
                let executed = executed.clone();
                let stopped = stopped.clone();
                let interrupted = interrupted.clone();
                std::thread::spawn(move || {
                    stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT)).unwrap();
                    let mut connection = tungstenite::accept(stream).unwrap();
                    connection
                        .get_ref()
                        .set_read_timeout(Some(Duration::from_millis(20)))
                        .unwrap();
                    let mut running: Option<(String, Instant)> = None;
                    while !stopped.load(Ordering::SeqCst) {
                        if let Some((id, at)) = &running
                            && at.elapsed() > Duration::from_millis(100)
                        {
                            let cancelled = interrupted.swap(false, Ordering::SeqCst);
                            let mut events = vec![];
                            if !cancelled {
                                events.push(json!({"method":"item/agentMessage/delta","params":{"threadId":"thread_fixture","turnId":id,"delta":"answer"}}));
                                events.push(json!({"method":"item/completed","params":{"threadId":"thread_fixture","turnId":id,"item":{"type":"agentMessage","id":format!("item_{id}"),"text":"answer"}}}));
                                events.push(json!({"method":"thread/tokenUsage/updated","params":{"threadId":"thread_fixture","tokenUsage":{"last":{"inputTokens":10,"outputTokens":2}}}}));
                            }
                            events.push(json!({"method":"turn/completed","params":{"threadId":"thread_fixture","turn":{"id":id,"status":if cancelled{"interrupted"}else{"completed"},"items":[],"error":null}}}));
                            for event in events {
                                if connection
                                    .send(Message::Text(event.to_string().into()))
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            running = None;
                        }
                        match connection.read() {
                            Ok(Message::Text(text)) => {
                                let message: Value = serde_json::from_str(&text).unwrap();
                                let id = message["id"].clone();
                                let result = match message["method"].as_str().unwrap_or("") {
                                    "initialize" => {
                                        json!({"userAgent":"fixture","platformFamily":"unix","platformOs":"macos"})
                                    }
                                    "initialized" => continue,
                                    "thread/start" | "thread/resume" => {
                                        json!({"thread":{"id":"thread_fixture","turns":[]}})
                                    }
                                    "thread/turns/list" => {
                                        let turns:Vec<_>=(1..=executed.load(Ordering::SeqCst)).rev().map(|index|json!({"id":format!("turn_{index}"),"status":"completed","items":[{"id":format!("answer_{index}"),"type":"agentMessage","text":"answer"}],"error":null})).collect();
                                        json!({"data":turns,"nextCursor":null})
                                    }
                                    "turn/start" => {
                                        assert_eq!(message["params"]["threadId"], "thread_fixture");
                                        let turn = format!(
                                            "turn_{}",
                                            executed.fetch_add(1, Ordering::SeqCst) + 1
                                        );
                                        if fail {
                                            return;
                                        }
                                        running = Some((turn.clone(), Instant::now()));
                                        json!({"turn":{"id":turn,"status":"inProgress","items":[],"error":null}})
                                    }
                                    "turn/interrupt" => {
                                        interrupted.store(true, Ordering::SeqCst);
                                        json!({})
                                    }
                                    _ => json!({}),
                                };
                                if connection
                                    .send(Message::Text(
                                        json!({"id":id,"result":result}).to_string().into(),
                                    ))
                                    .is_err()
                                {
                                    return;
                                }
                            }
                            Err(error) if timed_out(&error) => {}
                            Ok(Message::Close(_)) => break,
                            Err(_) => break,
                            _ => {}
                        }
                    }
                });
            }
        });
        let mut command = Command::new("sleep");
        command.arg("60");
        let process = Process::spawn(command, None).unwrap();
        let spec = agent::Spec {
            provider: ProviderKind::Codex,
            binary: "codex".into(),
            args: vec![],
            model: None,
            effort: None,
            cwd: directory.path().into(),
            add_dirs: vec![],
            access: agent::Access::ReadOnly,
            resume: None,
        };
        let rpc = Rpc::connect(&socket).unwrap();
        let session = Arc::new(Session {
            core: Arc::downgrade(&core),
            conversation: conversation.id,
            role: "orchestrator".into(),
            signature: "fixture".into(),
            spec,
            identity: "thread_fixture".into(),
            endpoint: format!("unix://{}", directory.path().join("gate.sock").display()),
            gateway_address: Mutex::new(None),
            gateway_password: crate::id(),
            backend: Backend::Codex {
                socket,
                rpc: Box::new(Mutex::new(rpc)),
            },
            stop: Arc::new(AtomicBool::new(false)),
            active: Mutex::new(None),
            completed: Mutex::new(VecDeque::new()),
            terminal: Mutex::new(None),
            gateway_error: Mutex::new(None),
            started: AtomicBool::new(true),
            executing: AtomicBool::new(false),
            user_interrupted: AtomicBool::new(false),
            touched: Mutex::new(Instant::now()),
            scope_round: None,
            process,
        });
        Fixture {
            directory,
            core,
            session,
            executions,
            shutdown,
        }
    }
    fn turn(fixture: &Fixture) -> String {
        let s = &fixture.session;
        let turn = fixture
            .core
            .store
            .start_turn(&s.conversation, &s.role, "codex", 0, None)
            .unwrap();
        fixture
            .core
            .store
            .record_run_context(&store::RunContext {
                turn_id: turn.id.clone(),
                conversation: s.conversation.clone(),
                role: s.role.clone(),
                provider: "codex".into(),
                profile: "fixture".into(),
                cwd: s.spec.cwd.clone(),
                add_dirs: vec![],
                access: store::RunAccess::ReadOnly,
                backend: store::RunBackend::CodexAppServer,
                provider_thread_id: Some(s.identity.clone()),
                provider_session_id: None,
                provider_turn_id: None,
            })
            .unwrap();
        turn.id
    }
    #[test]
    fn round_rotation_retires_gateway_and_process_before_fallback() {
        let mut fixture = fixture(false);
        {
            let session = Arc::get_mut(&mut fixture.session).unwrap();
            session.spec.binary = "/bin/false".into();
            let mut stable = session.spec.clone();
            stable.resume = None;
            session.signature = format!("orchestrator:round-one:{stable:?}");
        }
        let session = &fixture.session;
        fixture.core.native.sessions.lock().unwrap().insert(
            (session.conversation.clone(), session.role.clone()),
            session.clone(),
        );
        session.start_gateway().unwrap();
        let reused = fixture
            .core
            .native
            .prepare_scoped(
                &fixture.core,
                &session.conversation,
                &session.role,
                &session.spec,
                "round-one",
            )
            .unwrap()
            .unwrap();
        assert!(Arc::ptr_eq(&reused, session));
        let next = fixture
            .core
            .native
            .prepare_scoped(
                &fixture.core,
                &session.conversation,
                &session.role,
                &session.spec,
                "round-two",
            )
            .unwrap();
        assert!(next.is_none());
        assert!(session.stop.load(Ordering::SeqCst));
        assert!(
            session
                .process
                .child
                .lock()
                .unwrap()
                .try_wait()
                .unwrap()
                .is_some()
        );
        assert!(session.check_alive().is_err());
        assert!(
            fixture
                .core
                .native
                .get(&session.conversation, &session.role)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn structured_turns_share_the_managed_thread_and_keep_provider_turn_ids() {
        let fixture = fixture(false);
        let mut ids = vec![];
        for _ in 0..2 {
            let turn = turn(&fixture);
            let mut events = vec![];
            let result = fixture
                .session
                .run(&turn, "hello", &[], &AtomicBool::new(false), &mut |event| {
                    events.push(event)
                })
                .unwrap();
            assert_eq!(result.session.as_deref(), Some("thread_fixture"));
            assert_eq!(result.text, "answer");
            assert!(
                events
                    .iter()
                    .any(|event| matches!(event,agent::Event::Text{delta} if delta=="answer"))
            );
            assert!(events.iter().any(|event| matches!(
                event,
                agent::Event::Usage {
                    input: Some(10),
                    output: Some(2)
                }
            )));
            ids.push(
                fixture
                    .core
                    .store
                    .run_context(&fixture.session.conversation, &turn)
                    .unwrap()
                    .unwrap()
                    .provider_turn_id
                    .unwrap(),
            );
            fixture
                .core
                .store
                .finish_turn(&turn, store::TurnStatus::Done, None)
                .unwrap();
        }
        assert_ne!(ids[0], ids[1]);
        assert_eq!(fixture.executions.load(Ordering::SeqCst), 2);
    }
    #[test]
    fn interrupted_native_turn_does_not_destroy_the_session() {
        let fixture = fixture(false);
        let id = turn(&fixture);
        let error = fixture
            .session
            .run(&id, "hello", &[], &AtomicBool::new(true), &mut |_| {})
            .unwrap_err();
        assert!(error.to_string().contains("native turn interrupted"));
        assert!(!fixture.session.stop.load(Ordering::SeqCst));
        fixture
            .core
            .store
            .finish_turn(&id, store::TurnStatus::Interrupted, None)
            .unwrap();
        let id = turn(&fixture);
        assert!(
            fixture
                .session
                .run(&id, "next", &[], &AtomicBool::new(false), &mut |_| {})
                .is_ok()
        );
    }
    #[test]
    fn backend_failure_never_reexecutes_the_prompt() {
        let fixture = fixture(true);
        let id = turn(&fixture);
        assert!(
            fixture
                .session
                .run(&id, "hello", &[], &AtomicBool::new(false), &mut |_| {})
                .is_err()
        );
        assert!(fixture.session.stop.load(Ordering::SeqCst));
        assert_eq!(fixture.executions.load(Ordering::SeqCst), 1);
    }
    #[test]
    fn steering_at_completion_uses_only_the_latest_managed_turn() {
        let fixture = fixture(false);
        let id = turn(&fixture);
        fixture
            .core
            .store
            .bind_provider_turn(&fixture.session.conversation, &id, "completed")
            .unwrap();
        assert!(check_steer_target(&fixture.session, &json!("completed")).is_ok());
        assert!(check_steer_target(&fixture.session, &Value::Null).is_err());
        assert!(check_steer_target(&fixture.session, &json!("foreign")).is_err());
        let _next = turn(&fixture);
        assert!(check_steer_target(&fixture.session, &json!("completed")).is_err());
    }
    #[test]
    fn gateway_controls_target_the_active_turn_and_queued_steering_has_its_rpc_shape() {
        let fixture = fixture(false);
        let _turn = turn(&fixture);
        let conversation = fixture.session.conversation.clone();
        let lock = fixture.core.turn_lock(&conversation, "orchestrator");
        let _guard = lock.lock().unwrap();
        fixture.session.executing.store(true, Ordering::SeqCst);
        *fixture.session.active.lock().unwrap() = Some(Active {
            turn: json!({"id":"current","status":"inProgress"}),
            mail: vec![],
        });
        fixture.session.start_gateway().unwrap();
        let mut client = Rpc::connect(Path::new(
            fixture.session.endpoint.strip_prefix("unix://").unwrap(),
        ))
        .unwrap();
        assert!(
            client
                .call(
                    "turn/interrupt",
                    json!({"threadId":"thread_fixture","turnId":"old"})
                )
                .is_err()
        );
        assert!(client.call("turn/steer",json!({"threadId":"thread_fixture","expectedTurnId":"old","input":[{"type":"text","text":"stale"}]})).is_err());
        assert!(!fixture.session.user_interrupted.load(Ordering::SeqCst));
        assert!(fixture.core.store.mail(&conversation).unwrap().is_empty());
        let controls: u32 = fixture
            .core
            .store
            .db()
            .query_row(
                "SELECT count(*) FROM events WHERE kind='native_control'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(controls, 0);
        client.send(json!({"id":"queued","method":"turn/steer","params":{"threadId":"thread_fixture","expectedTurnId":"current","clientUserMessageId":"correction","input":[{"type":"text","text":"next correction"}]}})).unwrap();
        client.call("thread/loaded/list", json!({})).unwrap();
        let mail = fixture.core.store.mail(&conversation).unwrap();
        assert_eq!(mail.len(), 1);
        assert_eq!(fixture.executions.load(Ordering::SeqCst), 0);
        fixture.session.completed.lock().unwrap().push_back((
            mail[0].id,
            json!({"id":"next","status":"completed","items":[]}),
        ));
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            assert!(
                Instant::now() < deadline,
                "queued steering was not acknowledged"
            );
            if let Some(reply) = client.event().unwrap()
                && reply["id"] == "queued"
            {
                assert_eq!(reply["result"]["turnId"], "next");
                assert!(reply["result"].get("turn").is_none());
                break;
            }
        }
        fixture.core.cancel(&conversation);
    }
    #[test]
    fn reconnecting_a_native_client_does_not_enqueue_its_message_twice() {
        let fixture = fixture(false);
        let _turn = turn(&fixture);
        let conversation = fixture.session.conversation.clone();
        let lock = fixture.core.turn_lock(&conversation, "orchestrator");
        let _guard = lock.lock().unwrap();
        fixture.session.start_gateway().unwrap();
        let path = Path::new(fixture.session.endpoint.strip_prefix("unix://").unwrap());
        for _ in 0..2 {
            let mut client = Rpc::connect(path).unwrap();
            client.send(json!({"id":"submit","method":"turn/start","params":{"threadId":"thread_fixture","clientUserMessageId":"stable-message","input":[{"type":"text","text":"one correction"}]}})).unwrap();
            // A later response on the same connection proves submit reached
            // the gateway before we simulate losing its turn acknowledgement.
            client.call("thread/loaded/list", json!({})).unwrap();
            client.socket.close(None).unwrap();
        }
        let messages = fixture.core.store.mail(&conversation).unwrap();
        assert_eq!(
            messages
                .iter()
                .filter(|mail| mail.sender == "user" && mail.text == "one correction")
                .count(),
            1
        );
        assert_eq!(fixture.executions.load(Ordering::SeqCst), 0);
        let mut client = Rpc::connect(path).unwrap();
        assert!(client.call("turn/start",json!({"threadId":"thread_fixture","clientUserMessageId":"stable-message","input":[{"type":"text","text":"different correction"}]})).is_err());
        fixture.core.cancel(&conversation);
    }
    #[test]
    fn gateway_rejects_foreign_threads_and_unsupported_mutations() {
        let fixture = fixture(false);
        fixture.session.start_gateway().unwrap();
        let stream =
            UnixStream::connect(fixture.session.endpoint.strip_prefix("unix://").unwrap()).unwrap();
        stream.set_read_timeout(Some(HANDSHAKE_TIMEOUT)).unwrap();
        let (mut client, _) = tungstenite::client("ws://localhost/", stream).unwrap();
        for (index, method, params) in [
            ("thread/resume", json!({"threadId":"foreign"})),
            (
                "config/value/write",
                json!({"keyPath":"sandbox_mode","value":"danger-full-access"}),
            ),
            (
                "turn/start",
                json!({"threadId":"thread_fixture","input":[{"type":"image","url":"foreign"}]}),
            ),
        ]
        .into_iter()
        .enumerate()
        .map(|(i, (m, p))| (i, m, p))
        {
            client
                .send(Message::Text(
                    json!({"id":index+1,"method":method,"params":params})
                        .to_string()
                        .into(),
                ))
                .unwrap();
            let response = client.read().unwrap();
            let response: Value = serde_json::from_str(response.to_text().unwrap()).unwrap();
            assert!(response.get("error").is_some());
        }
        assert_eq!(fixture.executions.load(Ordering::SeqCst), 0);
    }
    #[test]
    fn pty_retains_unfinished_text_across_detach_and_scopes_handles() {
        let mut fixture = fixture(false);
        let script = fixture.directory.path().join("native-cli");
        std::fs::write(&script,"#!/bin/sh\nprintf 'native-ready\\n'\nwhile IFS= read -r text; do printf '%s\\n' \"$text\"; done\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        Arc::get_mut(&mut fixture.session).unwrap().spec.binary = script;
        let role = fixture.session.role.clone();
        let conversation = fixture.session.conversation.clone();
        fixture.core.native.sessions.lock().unwrap().insert(
            (conversation.clone(), role.clone()),
            fixture.session.clone(),
        );
        let first = fixture
            .core
            .terminal_open(&conversation, &role, 40, 8, false)
            .unwrap();
        let mut revision = first.revision;
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            assert!(Instant::now() < deadline);
            if let Some(screen) = fixture
                .core
                .terminal_read(&conversation, &role, &first.handle, revision)
                .unwrap()
            {
                revision = screen.revision;
                if screen
                    .cells
                    .iter()
                    .map(|cell| cell.text.as_str())
                    .collect::<String>()
                    .contains("native-ready")
                {
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let paste = super::super::super::terminal::Input::Paste {
            text: "draft 🐢".into(),
        };
        fixture
            .core
            .terminal_input(&conversation, &role, &first.handle, 1, paste.clone())
            .unwrap();
        fixture
            .core
            .terminal_input(&conversation, &role, &first.handle, 1, paste)
            .unwrap();
        assert!(
            fixture
                .core
                .terminal_input(
                    &conversation,
                    &role,
                    &first.handle,
                    3,
                    super::super::super::terminal::Input::Paste { text: "gap".into() }
                )
                .is_err()
        );
        fixture
            .core
            .terminal_detach(&conversation, &role, &first.handle)
            .unwrap();
        assert!(
            fixture
                .core
                .terminal_read(&conversation, &role, &first.handle, 0)
                .is_err()
        );
        std::thread::sleep(Duration::from_millis(50));
        let second = fixture
            .core
            .terminal_open(&conversation, &role, 40, 8, false)
            .unwrap();
        assert_eq!(
            second
                .cells
                .iter()
                .map(|c| c.text.as_str())
                .collect::<String>()
                .matches("draft 🐢")
                .count(),
            1
        );
        assert!(
            fixture
                .core
                .terminal_read("foreign", &role, &second.handle, 0)
                .is_err()
        );
        fixture
            .core
            .terminal_resize(&conversation, &role, &second.handle, 20, 5)
            .unwrap();
        let resized = fixture
            .core
            .terminal_read(&conversation, &role, &second.handle, 0)
            .unwrap()
            .unwrap();
        assert_eq!((resized.columns, resized.rows), (20, 5));
    }
    #[test]
    fn gateway_messages_join_the_core_queue_and_execute_in_the_same_session() {
        let mut fixture = fixture(false);
        {
            let session = Arc::get_mut(&mut fixture.session).unwrap();
            session.spec.binary = "/bin/false".into();
            session.spec.effort = Some("high".into());
            session.signature = format!("orchestrator:{:?}", session.spec);
        }
        let conversation = fixture.session.conversation.clone();
        fixture.core.native.sessions.lock().unwrap().insert(
            (conversation.clone(), "orchestrator".into()),
            fixture.session.clone(),
        );
        fixture.session.start_gateway().unwrap();
        let mut client = Rpc::connect(Path::new(
            fixture.session.endpoint.strip_prefix("unix://").unwrap(),
        ))
        .unwrap();
        client
            .call("thread/resume", json!({"threadId":"thread_fixture"}))
            .unwrap();
        let core = fixture.core.clone();
        let id = conversation.clone();
        let cwd = fixture.directory.path().to_path_buf();
        let first = std::thread::spawn(move || {
            core.turn(
                crate::lean::TurnSetup {
                    session_policy: crate::lean::SessionPolicy::Role,
                    conversation: &id,
                    plan_id: None,
                    role: "orchestrator".into(),
                    profile: core.config_snapshot().resolve(
                        crate::config::WorkKind::Orchestration,
                        crate::config::RoleKind::Orchestrator,
                    ),
                    cwd,
                    add_dirs: vec![],
                    access: agent::Access::ReadOnly,
                    upto_message: 0,
                },
                &AtomicBool::new(false),
                |_| "first".into(),
            )
            .unwrap()
        });
        let until = Instant::now() + Duration::from_secs(3);
        while fixture.executions.load(Ordering::SeqCst) == 0 {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(2));
        }
        let result=client.call("turn/start",json!({"threadId":"thread_fixture","clientUserMessageId":"durable-correction","input":[{"type":"text","text":"queued correction"}]})).unwrap();
        assert_eq!(result["turn"]["id"], "turn_2");
        assert_eq!(first.join().unwrap(), Some("answer".into()));
        let until = Instant::now() + Duration::from_secs(5);
        while !fixture.core.is_idle(&conversation) {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(fixture.executions.load(Ordering::SeqCst), 2);
        let messages = fixture.core.store.mail(&conversation).unwrap();
        let queued = messages
            .iter()
            .filter(|m| m.sender == "user" && m.text == "queued correction")
            .collect::<Vec<_>>();
        assert_eq!(queued.len(), 1);
        assert!(queued[0].delivered_turn.is_some());
        let sessions = fixture.core.store.sessions(&conversation).unwrap();
        assert!(
            sessions
                .iter()
                .all(|s| s.backend.as_deref() == Some("codex_app_server"))
        );
        fixture.session.completed.lock().unwrap().clear();
        let mut reconnected = Rpc::connect(Path::new(
            fixture.session.endpoint.strip_prefix("unix://").unwrap(),
        ))
        .unwrap();
        reconnected
            .call("thread/resume", json!({"threadId":"thread_fixture"}))
            .unwrap();
        let restored = reconnected.call("turn/start",json!({"threadId":"thread_fixture","clientUserMessageId":"durable-correction","input":[{"type":"text","text":"queued correction"}]})).unwrap();
        assert_eq!(restored["turn"]["id"], "turn_2");
        assert_eq!(restored["turn"]["items"][0]["text"], "answer");
        assert_eq!(fixture.executions.load(Ordering::SeqCst), 2);
        assert_eq!(
            fixture
                .core
                .store
                .mail(&conversation)
                .unwrap()
                .iter()
                .filter(|mail| mail.sender == "user" && mail.text == "queued correction")
                .count(),
            1
        );
    }
}
