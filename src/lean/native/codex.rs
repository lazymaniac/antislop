use super::*;
use std::collections::VecDeque;
use std::os::unix::net::{UnixListener, UnixStream};
use tungstenite::{Message, WebSocket};

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
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        let (socket, _) = tungstenite::client("ws://localhost/", stream)
            .map_err(|e| invalid(format!("Codex upstream handshake: {e}")))?;
        socket
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(50)))?;
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
        "sandbox":if spec.access == agent::Access::ReadOnly {"read-only"} else {"workspace-write"},
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
        if socket.exists() {
            break Rpc::connect(&socket)?;
        }
        require(
            Instant::now() < until,
            &format!(
                "Codex app-server did not open its socket: {}",
                process.diagnostics()
            ),
        )?;
        if let Some(status) = process.child.lock().map_err(invalid)?.try_wait()? {
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
            else {json!({"type":"workspaceWrite","writableRoots":[session.spec.cwd],"networkAccess":false})}}))?;
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

fn connection(weak: Weak<Session>, stream: UnixStream) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let mut client = tungstenite::accept(stream)
        .map_err(|e| invalid(format!("Codex gateway handshake: {e}")))?;
    client
        .get_mut()
        .set_read_timeout(Some(Duration::from_millis(25)))?;
    let session = weak.upgrade().ok_or_else(|| invalid("session closed"))?;
    let Backend::Codex { socket, .. } = &session.backend else {
        unreachable!()
    };
    let mut upstream = Rpc::raw(socket)?;
    let connection_id = crate::id();
    let mut queued: Vec<(Value, u64)> = vec![];
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
                                require(queued.len() < 32, "too many queued terminal requests")?;
                                let receipt = session
                                    .enqueue(&format!("native:{connection_id}:{id}"), &text)?;
                                if !queued.iter().any(|(pending, _)| pending == &id) {
                                    queued.push((
                                        id.clone(),
                                        receipt["mail"]
                                            .as_u64()
                                            .ok_or_else(|| invalid("missing mail receipt"))?,
                                    ));
                                }
                                Ok(false)
                            })
                        }
                        "turn/interrupt" => session.interrupt().map(|()| true),
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
                        client
                            .send(Message::Text(
                                json!({"id":id,"result":{}}).to_string().into(),
                            ))
                            .map_err(invalid)?;
                    }
                    Err(error) if !id.is_null() => {
                        client.send(Message::Text(json!({"id":id,"error":{"code":-32602,"message":error.to_string()}}).to_string().into())).map_err(invalid)?;
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
            for (id, mail) in queued.drain(..) {
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
                    client
                        .send(Message::Text(
                            json!({"id":id,"result":{"turn":turn}}).to_string().into(),
                        ))
                        .map_err(invalid)?;
                    if turn["status"] != "inProgress" {
                        client.send(Message::Text(json!({"method":"turn/completed","params":{"threadId":session.identity,"turn":turn}}).to_string().into())).map_err(invalid)?;
                    }
                } else {
                    remaining.push((id, mail));
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
        let directory = tempfile::tempdir_in("/private/tmp").unwrap();
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
                    stream
                        .set_read_timeout(Some(Duration::from_millis(20)))
                        .unwrap();
                    let mut connection = tungstenite::accept(stream).unwrap();
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
    fn gateway_rejects_foreign_threads_and_unsupported_mutations() {
        let fixture = fixture(false);
        fixture.session.start_gateway().unwrap();
        let stream =
            UnixStream::connect(fixture.session.endpoint.strip_prefix("unix://").unwrap()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
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
        let result=client.call("turn/start",json!({"threadId":"thread_fixture","input":[{"type":"text","text":"queued correction"}]})).unwrap();
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
    }
}
