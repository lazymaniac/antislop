use super::*;
use std::collections::{HashMap, HashSet};
use std::sync::mpsc;

struct EventsGuard(Arc<AtomicBool>);
impl Drop for EventsGuard {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

fn subscribe(session: &Session) -> Result<(mpsc::Receiver<Result<Value>>, EventsGuard)> {
    let Backend::Opencode { endpoint, password } = &session.backend else {
        unreachable!()
    };
    // Establish the subscription before submitting the prompt. A short turn
    // may otherwise complete before the first event connection is ready.
    let response = raw(endpoint, password, "GET", "/event", None)?;
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    let (send, receive) = mpsc::sync_channel(128);
    std::thread::spawn(move || {
        let mut reader = BufReader::new(response.into_body().into_reader());
        while !stopped.load(Ordering::SeqCst) {
            let event = (|| -> Result<Option<Value>> {
                let mut line = Vec::new();
                let count = reader
                    .by_ref()
                    .take(4 * 1024 * 1024 + 1)
                    .read_until(b'\n', &mut line)?;
                require(count != 0, "OpenCode event stream disconnected")?;
                require(count <= 4 * 1024 * 1024, "OpenCode event exceeds 4 MiB")?;
                let Some(data) = line.strip_prefix(b"data:") else {
                    return Ok(None);
                };
                Ok(Some(serde_json::from_slice(data)?))
            })();
            match event {
                Ok(None) => continue,
                Ok(Some(event)) => {
                    if send.send(Ok(event)).is_err() {
                        break;
                    }
                }
                Err(error) => {
                    let _ = send.send(Err(error));
                    break;
                }
            }
        }
    });
    Ok((receive, EventsGuard(stop)))
}

pub(super) fn run(
    session: &Session,
    turn: &str,
    prompt: &str,
    cancel: &AtomicBool,
    emit: &mut dyn FnMut(agent::Event),
) -> Result<agent::Outcome> {
    let (events, _guard) = subscribe(session)?;
    let message = format!("msg_{}", turn.replace('-', ""));
    let mut body =
        json!({"messageID":message,"agent":"antislop","parts":[{"type":"text","text":prompt}]});
    if let Some(model) = &session.spec.model {
        let (provider, model) = model
            .split_once('/')
            .ok_or_else(|| invalid("OpenCode model must use provider/model"))?;
        body["model"] = json!({"providerID":provider,"modelID":model});
    }
    if let Some(effort) = &session.spec.effort {
        body["variant"] = effort.clone().into();
    }
    let root = format!("/session/{}", session.identity);
    request(session, "POST", &format!("{root}/prompt_async"), Some(body))?;
    emit(agent::Event::Session {
        id: session.identity.clone(),
    });
    let mut state = Turn::new(&session.identity, &message);
    let deadline = Instant::now() + Duration::from_secs(30 * 60);
    let mut retry = None;
    let result = loop {
        if session.user_interrupted.load(Ordering::SeqCst) {
            break Err(invalid("native turn interrupted"));
        }
        if cancel.load(Ordering::SeqCst) {
            break Err(invalid(format!(
                "cancelled{}",
                retry
                    .as_ref()
                    .map(|error| format!("; OpenCode: {error}"))
                    .unwrap_or_default()
            )));
        }
        if Instant::now() >= deadline {
            break Err(invalid("OpenCode native turn timed out after 30 minutes"));
        }
        let event = match events.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(event)) => event,
            Ok(Err(error)) => break Err(error),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                break Err(invalid("OpenCode event reader stopped"));
            }
        };
        let payload = if event["payload"].is_object() {
            &event["payload"]
        } else {
            &event
        };
        if payload["type"] == "session.status"
            && payload["properties"]["sessionID"] == session.identity
            && payload["properties"]["status"]["type"] == "retry"
            && let Some(message) = payload["properties"]["status"]["message"].as_str()
            && retry.as_deref() != Some(message)
        {
            retry = Some(message.to_owned());
            emit(agent::Event::Output {
                name: "OpenCode retry".into(),
                text: message.into(),
                exit_code: None,
            });
        }
        match state.feed(payload, emit) {
            Ok(Some(text)) => {
                break Ok(agent::Outcome {
                    session: Some(session.identity.clone()),
                    text,
                });
            }
            Ok(None) => {}
            Err(error) => break Err(error),
        }
    };
    if session.user_interrupted.load(Ordering::SeqCst) {
        return Err(invalid("native turn interrupted"));
    }
    if result.is_err() {
        let _ = request(session, "POST", &format!("{root}/abort"), Some(json!({})));
    }
    result
}

struct Turn {
    session: String,
    user: String,
    assistants: HashSet<String>,
    bound: bool,
    parts: HashMap<String, Value>,
    order: Vec<String>,
    text: HashMap<String, String>,
    seen: HashSet<String>,
    parser: agent::Parser,
}
impl Turn {
    fn new(session: &str, user: &str) -> Self {
        Self {
            session: session.into(),
            user: user.into(),
            assistants: HashSet::new(),
            bound: false,
            parts: HashMap::new(),
            order: Vec::new(),
            text: HashMap::new(),
            seen: HashSet::new(),
            parser: agent::Parser::new(ProviderKind::Opencode),
        }
    }
    fn canonical(&mut self, kind: &str, part: &Value, emit: &mut dyn FnMut(agent::Event)) {
        for event in self
            .parser
            .feed(&json!({"type":kind,"sessionID":self.session,"part":part}))
        {
            match &event {
                agent::Event::Session { .. } => continue,
                agent::Event::Text { .. } | agent::Event::Reasoning { .. } => continue,
                agent::Event::ProviderTurn { .. } if self.bound => continue,
                agent::Event::ProviderTurn { .. } => self.bound = true,
                agent::Event::Tool { .. }
                    if !self.seen.insert(format!("announced:{}", part["id"])) =>
                {
                    continue;
                }
                _ => {}
            }
            emit(event);
        }
        if kind == "tool_use" && part["state"]["status"] == "error" {
            let error = &part["state"]["error"];
            if let Some(text) = error.as_str().or(error["message"].as_str()) {
                emit(agent::Event::Output {
                    name: part["tool"].as_str().unwrap_or("tool").into(),
                    text: agent::clip_tail(text, 8192),
                    exit_code: part["state"]["metadata"]["exit"].as_i64(),
                });
            }
        }
    }
    fn update_text(&mut self, part: &Value, emit: &mut dyn FnMut(agent::Event)) -> Result<()> {
        let Some(text) = part["text"].as_str() else {
            return Ok(());
        };
        let id = part["id"].as_str().unwrap_or_default();
        let delivered = self.text.entry(id.into()).or_default();
        if delivered.starts_with(text) {
            return Ok(());
        }
        let delta = text
            .strip_prefix(delivered.as_str())
            .ok_or_else(|| invalid("OpenCode rewrote a streamed text part"))?;
        if !delta.is_empty() {
            emit(if part["type"] == "reasoning" {
                agent::Event::Reasoning {
                    delta: delta.into(),
                }
            } else {
                agent::Event::Text {
                    delta: delta.into(),
                }
            });
            *delivered = text.into();
        }
        Ok(())
    }
    fn flush_text(&mut self, message: &str, emit: &mut dyn FnMut(agent::Event)) -> Result<()> {
        let parts: Vec<_> = self
            .order
            .iter()
            .filter_map(|id| self.parts.get(id))
            .filter(|part| {
                part["messageID"] == message
                    && matches!(part["type"].as_str(), Some("text" | "reasoning"))
            })
            .cloned()
            .collect();
        for part in parts {
            let id = part["id"].as_str().unwrap_or_default();
            if self.seen.insert(format!("text:{id}")) {
                self.update_text(&part, emit)?;
                self.canonical(part["type"].as_str().unwrap(), &part, emit);
            }
        }
        self.parts.retain(|_, part| part["messageID"] != message);
        self.order.retain(|id| self.parts.contains_key(id));
        self.text.retain(|id, _| self.parts.contains_key(id));
        Ok(())
    }
    fn finish(
        &mut self,
        part: &Value,
        emit: &mut dyn FnMut(agent::Event),
    ) -> Result<Option<String>> {
        let message = part["messageID"].as_str().unwrap_or_default();
        if !self.seen.insert(format!("finish:{message}")) {
            return Ok(None);
        }
        self.flush_text(message, emit)?;
        self.canonical("step_finish", part, emit);
        match self.parser.finish() {
            agent::Finish::Done(text) => Ok(Some(text)),
            agent::Finish::Failed(error) => Err(invalid(error)),
            agent::Finish::Missing => Ok(None),
        }
    }
    fn feed(
        &mut self,
        event: &Value,
        emit: &mut dyn FnMut(agent::Event),
    ) -> Result<Option<String>> {
        require(
            self.seen.len() + self.parts.len() < 16384,
            "OpenCode turn exceeds the event item limit",
        )?;
        let p = &event["properties"];
        let session = p["sessionID"]
            .as_str()
            .or(p["info"]["sessionID"].as_str())
            .or(p["part"]["sessionID"].as_str());
        if session != Some(&self.session) {
            return Ok(None);
        }
        match event["type"].as_str().unwrap_or_default() {
            "session.error" => {
                let error = &p["error"];
                return Err(invalid(format!(
                    "OpenCode: {}",
                    error["data"]["message"]
                        .as_str()
                        .or(error["message"].as_str())
                        .or(error["name"].as_str())
                        .unwrap_or("unknown session error")
                )));
            }
            "message.updated" => {
                let info = &p["info"];
                if info["role"] != "assistant" || info["parentID"] != self.user {
                    return Ok(None);
                }
                let id = info["id"]
                    .as_str()
                    .ok_or_else(|| invalid("OpenCode assistant has no id"))?;
                self.assistants.insert(id.into());
                if !self.bound {
                    self.bound = true;
                    emit(agent::Event::ProviderTurn { id: id.into() });
                }
                if info["time"]["completed"].is_number() && info["finish"].is_string() {
                    if !info["error"].is_null() {
                        return Err(invalid(format!("OpenCode: {}", info["error"])));
                    }
                    return self.finish(
                        &json!({"messageID":id,"reason":info["finish"],"tokens":info["tokens"]}),
                        emit,
                    );
                }
            }
            "message.part.updated" => {
                let part = &p["part"];
                if !part["messageID"]
                    .as_str()
                    .is_some_and(|id| self.assistants.contains(id))
                {
                    return Ok(None);
                }
                let id = part["id"]
                    .as_str()
                    .ok_or_else(|| invalid("OpenCode part has no id"))?;
                if part["type"] != "tool"
                    && self.seen.contains(&format!(
                        "finish:{}",
                        part["messageID"].as_str().unwrap_or_default()
                    ))
                {
                    return Ok(None);
                }
                match part["type"].as_str().unwrap_or_default() {
                    "step-start" if self.seen.insert(format!("start:{id}")) => {
                        self.canonical("step_start", part, emit)
                    }
                    "step-finish" => return self.finish(part, emit),
                    "text" | "reasoning" => {
                        if let Some(old) = self.parts.get(id)
                            && let (Some(old), Some(new)) =
                                (old["text"].as_str(), part["text"].as_str())
                            && old.len() > new.len()
                            && old.starts_with(new)
                        {
                            return Ok(None);
                        }
                        self.update_text(part, emit)?;
                        if !self.parts.contains_key(id) {
                            self.order.push(id.into());
                        }
                        self.parts.insert(id.into(), part.clone());
                    }
                    "tool"
                        if matches!(
                            part["state"]["status"].as_str(),
                            Some("running" | "completed" | "error")
                        ) && self
                            .seen
                            .insert(format!("tool:{id}:{}", part["state"]["status"])) =>
                    {
                        self.canonical("tool_use", part, emit)
                    }
                    _ => {}
                }
            }
            "message.part.delta" if p["field"] == "text" => {
                if let Some(mut part) = p["partID"]
                    .as_str()
                    .and_then(|id| self.parts.get(id))
                    .cloned()
                {
                    let text = format!(
                        "{}{}",
                        part["text"].as_str().unwrap_or_default(),
                        p["delta"].as_str().unwrap_or_default()
                    );
                    part["text"] = text.into();
                    self.update_text(&part, emit)?;
                    self.parts
                        .insert(p["partID"].as_str().unwrap().into(), part);
                }
            }
            _ => {}
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_tool_failure_preserves_its_cause_without_duplicate_output() {
        let mut turn = Turn::new("managed", "user");
        let mut events = vec![];
        let mut emit = |event| events.push(event);
        turn.feed(&assistant("answer", "user"), &mut emit).unwrap();
        let error = part(
            "failed",
            "answer",
            "tool",
            json!({"tool":"read","state":{"status":"error","input":{"filePath":"missing.rs"},"error":"cannot read missing.rs: file not found"}}),
        );
        turn.feed(&error, &mut emit).unwrap();
        turn.feed(&error, &mut emit).unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event,agent::Event::Tool { name,.. } if name == "read"))
                .count(),
            1
        );
        let output: Vec<_> = events
            .iter()
            .filter_map(|event| {
                if let agent::Event::Output { text, .. } = event {
                    Some(text.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(output, ["cannot read missing.rs: file not found"]);
    }
    fn assistant(id: &str, parent: &str) -> Value {
        json!({"type":"message.updated","properties":{"info":{"id":id,"role":"assistant","sessionID":"managed","parentID":parent}}})
    }
    fn part(id: &str, message: &str, kind: &str, fields: Value) -> Value {
        let mut p = json!({"id":id,"messageID":message,"sessionID":"managed","type":kind});
        p.as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        json!({"type":"message.part.updated","properties":{"part":p}})
    }
    #[test]
    fn live_deltas_and_completed_snapshots_are_not_duplicated() {
        let mut turn = Turn::new("managed", "user");
        let mut events = vec![];
        let mut emit = |event| events.push(event);
        for event in [
            assistant("foreign-message", "another-user"),
            part(
                "foreign-text",
                "foreign-message",
                "text",
                json!({"text":"private"}),
            ),
            assistant("answer", "user"),
            part("start", "answer", "step-start", json!({})),
            part("text", "answer", "text", json!({"text":""})),
            json!({"type":"message.part.delta","properties":{"sessionID":"managed","partID":"text","field":"text","delta":"hello 👩‍💻"}}),
            part("text", "answer", "text", json!({"text":"hello"})),
            part("text", "answer", "text", json!({"text":"hello 👩‍💻"})),
            part("text", "answer", "text", json!({"text":"hello 👩‍💻"})),
        ] {
            assert_eq!(turn.feed(&event, &mut emit).unwrap(), None);
        }
        let finish = part(
            "finish",
            "answer",
            "step-finish",
            json!({"reason":"stop","tokens":{"input":10,"output":2,"cache":{"read":3,"write":1}}}),
        );
        assert_eq!(
            turn.feed(&finish, &mut emit).unwrap(),
            Some("hello 👩‍💻".into())
        );
        assert_eq!(turn.feed(&finish, &mut emit).unwrap(), None);
        let text: String = events
            .iter()
            .filter_map(|event| {
                if let agent::Event::Text { delta } = event {
                    Some(delta.as_str())
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(text, "hello 👩‍💻");
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event,agent::Event::ProviderTurn { id } if id == "answer"))
                .count(),
            1
        );
        assert!(events.iter().any(|event| matches!(
            event,
            agent::Event::Usage {
                input: Some(14),
                output: Some(2)
            }
        )));
    }
    #[test]
    fn tool_steps_keep_live_history_and_return_only_the_final_answer() {
        let mut turn = Turn::new("managed", "user");
        let mut events = vec![];
        let mut emit = |event| events.push(event);
        for event in [
            assistant("first", "user"),
            part("start1", "first", "step-start", json!({})),
            part("text1", "first", "text", json!({"text":"working"})),
            part(
                "tool",
                "first",
                "tool",
                json!({"tool":"read","state":{"status":"running","input":{"filePath":"file.rs"}}}),
            ),
            part(
                "tool",
                "first",
                "tool",
                json!({"tool":"read","state":{"status":"completed","input":{"filePath":"file.rs"},"output":"tool output"}}),
            ),
            part(
                "tool",
                "first",
                "tool",
                json!({"tool":"read","state":{"status":"completed","input":{"filePath":"file.rs"},"output":"tool output"}}),
            ),
            part(
                "finish1",
                "first",
                "step-finish",
                json!({"reason":"tool-calls"}),
            ),
            assistant("second", "user"),
            part("start2", "second", "step-start", json!({})),
            part("reason", "second", "reasoning", json!({"text":"thinking"})),
            part("text2", "second", "text", json!({"text":"done"})),
        ] {
            assert_eq!(turn.feed(&event, &mut emit).unwrap(), None)
        }
        // Older/newer servers may signal completion on the assistant itself.
        let end = json!({"type":"message.updated","properties":{"info":{"id":"second","role":"assistant","sessionID":"managed","parentID":"user","finish":"stop","time":{"completed":1},"tokens":{"input":2,"output":1},"error":null}}});
        assert_eq!(turn.feed(&end, &mut emit).unwrap(), Some("done".into()));
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, agent::Event::ProviderTurn { .. }))
                .count(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, agent::Event::Tool { .. }))
                .count(),
            1
        );
        assert_eq!(events.iter().filter(|event|matches!(event,agent::Event::Output { text,.. } if text == "tool output")).count(),1);
        assert!(
            events.iter().any(
                |event| matches!(event,agent::Event::Reasoning { delta } if delta == "thinking")
            )
        );
    }
    #[test]
    fn foreign_sessions_are_ignored_and_managed_errors_preserve_the_cause() {
        let mut turn = Turn::new("managed", "user");
        let mut emit = |_| {};
        let foreign = json!({"type":"session.error","properties":{"sessionID":"foreign","error":{"data":{"message":"foreign"}}}});
        assert!(turn.feed(&foreign, &mut emit).unwrap().is_none());
        let own = json!({"type":"session.error","properties":{"sessionID":"managed","error":{"data":{"message":"connection refused"}}}});
        assert!(
            turn.feed(&own, &mut emit)
                .unwrap_err()
                .to_string()
                .contains("connection refused")
        );
    }
}
