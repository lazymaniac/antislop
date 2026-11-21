mod common;
mod support;

use antislop::{
    config::Config,
    lean::{
        Core,
        daemon::{Client, ControlRequest, HistoryRequest, Request},
        tui::{App, InputBuffer, Update},
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

struct Daemon {
    child: Child,
    client: Client,
}

/// Keeps every fixture, project, database, log, and raw measurement in the
/// durable task-state run while exposing a short relative socket path to the
/// TUI client (Unix domain socket paths have a small platform limit).
struct RunLayout {
    root: PathBuf,
    link: PathBuf,
    relative: PathBuf,
}

impl RunLayout {
    fn new() -> Self {
        let root = std::env::var_os("ANTISLOP_MESSAGE_SEQUENCE_RUN_DIR")
            .map(PathBuf::from)
            .expect("set ANTISLOP_MESSAGE_SEQUENCE_RUN_DIR to a fresh durable run directory");
        std::fs::create_dir_all(&root).unwrap();
        assert!(
            std::fs::read_dir(&root).unwrap().next().is_none(),
            "run directory must be fresh to preserve completed measurements"
        );
        let workspace = std::env::current_dir().unwrap();
        let link = workspace
            .join(".antislop/tmp")
            .join(format!("message-sequence-{}", std::process::id()));
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        if std::fs::symlink_metadata(&link).is_ok() {
            std::fs::remove_file(&link).unwrap();
        }
        std::os::unix::fs::symlink(&root, &link).unwrap();
        let relative = link.strip_prefix(&workspace).unwrap().to_path_buf();
        Self {
            root,
            link,
            relative,
        }
    }
}

impl Drop for RunLayout {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.link);
    }
}

impl Daemon {
    fn start(project: &Path, config: &Path, state: &Path, registry: &Path) -> Self {
        common::assert_test_socket(state, Path::new("."));
        let child = Command::new(env!("CARGO_BIN_EXE_antislop"))
            .args(["__core"])
            .arg(project)
            .arg(config)
            .arg(state)
            .env("ANTISLOP_CONFIG_DIR", registry)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("start isolated test daemon");
        let daemon = Self {
            child,
            client: Client::at(state),
        };
        common::wait_until("isolated daemon socket", || {
            daemon
                .client
                .call(&Request::Control(ControlRequest::Identity))
                .is_ok()
        });
        daemon
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self
            .client
            .call(&Request::Control(ControlRequest::ForceShutdown));
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A byte-preserving Unix-socket proxy. Its one-shot gates let a test hold or
/// drop a real daemon reply after the request has crossed into the Core.
struct Proxy {
    state: PathBuf,
    drop_reply: Arc<AtomicBool>,
    hold_reply: Arc<AtomicBool>,
    release_reply: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    received: mpsc::Receiver<Value>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Proxy {
    fn start(state: &Path, upstream: &Path) -> Self {
        let socket = antislop::lean::daemon::socket(state);
        if let Some(parent) = socket.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let upstream = antislop::lean::daemon::socket(upstream);
        let drop_reply = Arc::new(AtomicBool::new(false));
        let hold_reply = Arc::new(AtomicBool::new(false));
        let release_reply = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let drop_flag = drop_reply.clone();
        let hold_flag = hold_reply.clone();
        let release_flag = release_reply.clone();
        let stop_flag = stop.clone();
        let (tx, received) = mpsc::channel();
        let thread = thread::spawn(move || {
            while !stop_flag.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((downstream, _)) => {
                        let upstream = upstream.clone();
                        let drop_flag = drop_flag.clone();
                        let hold_flag = hold_flag.clone();
                        let release_flag = release_flag.clone();
                        let tx = tx.clone();
                        thread::spawn(move || {
                            let _ = proxy_one(
                                downstream,
                                &upstream,
                                &drop_flag,
                                &hold_flag,
                                &release_flag,
                                &tx,
                            );
                        });
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            state: state.to_path_buf(),
            drop_reply,
            hold_reply,
            release_reply,
            stop,
            received,
            thread: Some(thread),
        }
    }

    fn drop_next_reply(&self) {
        self.drop_reply.store(true, Ordering::SeqCst);
    }

    fn hold_next_send_reply(&self) {
        self.hold_reply.store(true, Ordering::SeqCst);
    }

    fn release_reply(&self) {
        self.release_reply.store(true, Ordering::SeqCst);
    }

    fn request(&self, op: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let request = self
                .received
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| {
                    panic!("timed out waiting for {op}; raw receive error: {error}")
                });
            if request["op"] == op {
                return request;
            }
        }
    }

    fn client(&self) -> Client {
        Client::at(&self.state)
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(antislop::lean::daemon::socket(&self.state));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn proxy_one(
    mut downstream: UnixStream,
    upstream_path: &Path,
    drop_reply: &AtomicBool,
    hold_reply: &AtomicBool,
    release_reply: &AtomicBool,
    received: &mpsc::Sender<Value>,
) -> std::io::Result<()> {
    let mut request_reader = BufReader::new(downstream.try_clone()?);
    let request_bytes = read_transport_message(&mut request_reader)?;
    let request = serde_json::from_slice::<Value>(&request_bytes)
        .ok()
        .map(|envelope| envelope["request"].clone());
    if let Some(request) = &request {
        let _ = received.send(request.clone());
    }
    let mut upstream = UnixStream::connect(upstream_path)?;
    upstream.write_all(&request_bytes)?;
    upstream.flush()?;
    let mut upstream_reader = BufReader::new(upstream);
    let response = read_transport_message(&mut upstream_reader)?;
    if request
        .as_ref()
        .is_some_and(|request| request["op"] == "send")
        && drop_reply.swap(false, Ordering::SeqCst)
    {
        return Ok(());
    }
    if request
        .as_ref()
        .is_some_and(|request| request["op"] == "send")
        && hold_reply.swap(false, Ordering::SeqCst)
    {
        while !release_reply.load(Ordering::SeqCst) {
            thread::sleep(Duration::from_millis(5));
        }
        release_reply.store(false, Ordering::SeqCst);
    }
    downstream.write_all(&response)?;
    downstream.flush()
}

fn read_transport_message(reader: &mut impl BufRead) -> std::io::Result<Vec<u8>> {
    let mut message = Vec::new();
    loop {
        let mut frame = Vec::new();
        if reader.read_until(b'\n', &mut frame)? == 0 {
            break;
        }
        let chunk = serde_json::from_slice::<Value>(&frame)
            .ok()
            .and_then(|value| value.get("antislop_chunk").cloned());
        let done = chunk.as_ref().is_none_or(|chunk| chunk["done"] == true);
        message.extend_from_slice(&frame);
        if done {
            break;
        }
    }
    Ok(message)
}

fn install_fake_providers(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let codex = dir.join("codex");
    let agy = dir.join("agy");
    let gate = dir.join("release-worker");
    let plan = r#"Plan.
```plan
{"tasks":[{"id":"t1","title":"Worker one","goal":"Inspect the fixture","depends_on":[],"change_kind":"none","kind":"analysis","reviewer":{"focus":"Verify the result"}},{"id":"t2","title":"Worker two","goal":"Inspect the second fixture","depends_on":[],"change_kind":"none","kind":"analysis","reviewer":{"focus":"Verify the result"}}]}
```"#;
    let plan_path = dir.join("plan.txt");
    std::fs::write(&plan_path, plan).unwrap();
    let codex_source = r##"#!/usr/bin/env python3
import json,sys,re
p=sys.argv[-1]
if any(x in p for x in ('You review a plan','You audit','You review')):
 ids=re.findall(r'^- ([A-Za-z0-9_.-]+): ',p,re.M)
 answer='```verdict\n'+json.dumps({'approved':True,'findings':[],'coverage':{i:{'status':'ok','note':'checked'} for i in ids}})+'\n```'
elif 'You are the planner' in p:
 answer=open('@PLAN@').read()
else:
 answer='The reviewed plan is ready for the user to resume.'
print(json.dumps({'type':'thread.started','thread_id':'seq-test'}))
print(json.dumps({'type':'item.completed','item':{'type':'agent_message','text':answer}}))
print(json.dumps({'type':'turn.completed','usage':{}}))
"##
    .replace("@PLAN@", &plan_path.to_string_lossy());
    let agy_source = r##"#!/usr/bin/env python3
import json,sys,time,os,re,select
p=sys.argv[-1]
gate='@GATE@'
log='@INPUT_LOG@'
def note(value):
 with open(log,'a') as out: out.write(value+'\n')
if any(x in p for x in ('You review a plan','You audit','You review')):
 ids=re.findall(r'^- ([A-Za-z0-9_.-]+): ',p,re.M)
 answer='```verdict\n'+json.dumps({'approved':True,'findings':[],'coverage':{i:{'status':'ok','note':'checked'} for i in ids}})+'\n```'
else:
 note('START')
 while not os.path.exists(gate):
  try:
   ready,_,_=select.select([sys.stdin],[],[],.02)
   if ready:
    line=sys.stdin.readline()
    if line:
     note('INPUT:'+line.rstrip())
     if '\x1b' in line: note('ESCAPE:'+str(line.count('\x1b')))
  except KeyboardInterrupt:
   note('INTERRUPT')
 answer='worker completed after messages'
print(json.dumps({'event':'init','conversation_id':'seq-worker'}))
print(json.dumps({'event':'result','result':{'conversation_id':'seq-worker','status':'SUCCESS','response':answer,'usage':{}}}))
"##
    .replace("@GATE@", &gate.to_string_lossy())
    .replace("@INPUT_LOG@", &dir.join("provider-input.log").to_string_lossy());
    for (path, source) in [(&codex, codex_source), (&agy, agy_source)] {
        std::fs::write(path, source).unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        common::interactive(path);
    }
    let template = include_str!("../examples/antislop.toml");
    let config = template
        .replace(
            "binary = \"codex\"",
            &format!("binary = {:?}", codex.to_string_lossy()),
        )
        .replace(
            "binary = \"agy\"",
            &format!("binary = {:?}", agy.to_string_lossy()),
        );
    let path = dir.join("antislop.toml");
    std::fs::write(&path, config).unwrap();
    path
}

fn pump_send(app: &mut App, updates: &antislop::lean::tui::UpdateReceiver) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let update = updates
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|_| "timed out waiting for send result".to_owned())?;
        if let Update::SendResult { result, .. } = &update {
            let result = result.clone();
            app.apply_update(update);
            return result;
        }
        app.apply_update(update);
    }
}

fn event_values(client: &Client, conversation: &str) -> Vec<Value> {
    let mut after = 0;
    let mut events = Vec::new();
    loop {
        let page = client
            .call(&Request::History(HistoryRequest::Events {
                conversation: conversation.to_owned(),
                after,
                wait_ms: 0,
            }))
            .unwrap();
        let Some(items) = page["events"].as_array() else {
            break;
        };
        if items.is_empty() {
            break;
        }
        after = items.last().unwrap()["seq"].as_u64().unwrap();
        events.extend(items.iter().cloned());
    }
    events
}

fn refresh_app_from_core(app: &mut App, client: &Client, conversation: &str) {
    let reply = client
        .call(&Request::History(HistoryRequest::Events {
            conversation: conversation.to_owned(),
            after: 0,
            wait_ms: 0,
        }))
        .unwrap();
    app.apply_update(Update::Events {
        conversation: conversation.to_owned(),
        generation: app.generation,
        events: reply["events"].as_array().cloned().unwrap_or_default(),
        idle: reply["idle"] == true,
        observer: reply["observer"] == true,
    });
}

fn wait_agent_confirmation(
    app: &mut App,
    updates: &antislop::lean::tui::UpdateReceiver,
    operation: &str,
) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        let update = updates
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("agent message confirmation");
        if let Update::AgentMessage {
            operation: confirmed,
            result,
            ..
        } = &update
            && confirmed == operation
        {
            let result = result.clone().expect("Core confirms agent message");
            app.apply_update(update);
            return result;
        }
        app.apply_update(update);
    }
    panic!("timed out waiting for agent message confirmation {operation}");
}

#[test]
fn composer_broadcast_retries_by_operation_and_keeps_the_session_draft() {
    let run = RunLayout::new();
    let config = install_fake_providers(&run.root);
    let project = run.root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    common::initialize_git_project(&project);
    let state = run.relative.join("runtime");
    let registry = run.root.join("registry");
    let core = Core::open(&state, Config::load(&config).unwrap()).unwrap();
    let conversation = support::fixture_conversation(&core, &project).unwrap().id;
    drop(core);
    let daemon = Daemon::start(&project, &config, &state, &registry);
    let _release_worker = common::ReleaseOnDrop(run.root.join("release-worker"));
    let proxy_state = run.relative.join("proxy");
    let proxy = Proxy::start(&proxy_state, &state);
    let (mut app, updates) = App::test_app_with_client(proxy.client());
    app.open(conversation.clone());
    app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    app.input = InputBuffer::with_text("sequence fixture request");
    proxy.drop_next_reply();
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let initial = proxy.request("send");
    assert_eq!(initial["op"], "send");
    assert_eq!(initial["conversation"], conversation);
    assert!(
        pump_send(&mut app, &updates).is_err(),
        "the dropped acknowledgement must be visible"
    );
    assert!(
        app.input.text.contains("sequence fixture request"),
        "failed send must preserve its draft"
    );
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let retry = proxy.request("send");
    assert_eq!(
        retry["operation_id"], initial["operation_id"],
        "retry must reuse the operation correlation id"
    );
    pump_send(&mut app, &updates).unwrap();
    let plans = || {
        daemon
            .client
            .call(&Request::Conversation(
                antislop::lean::daemon::ConversationRequest::Plans {
                    conversation: conversation.clone(),
                    offset: None,
                    limit: None,
                },
            ))
            .unwrap()
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    while plans()["total"] != 1 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        plans()["total"],
        1,
        "no plan; events={:#?}",
        event_values(&daemon.client, &conversation)
    );
    assert_eq!(
        plans()["plans"][0]["tasks"].as_array().map(Vec::len),
        Some(2)
    );
    let events = event_values(&daemon.client, &conversation);
    assert_eq!(
        events
            .iter()
            .filter(|event| event["kind"] == "message" && event["payload"]["author"] == "user")
            .count(),
        2,
        "one scope confirmation and one original send; the lost-ack retry adds no message"
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event["kind"] == "message"
                && event["payload"]["text"] == "sequence fixture request")
            .count(),
        1
    );
    eprintln!(
        "sequence retry: initial={initial} retry={retry} same_operation_id={} original_messages=1",
        initial["operation_id"] == retry["operation_id"]
    );
    common::wait_until("independent plan review approves", || {
        event_values(&daemon.client, &conversation)
            .iter()
            .any(|event| {
                event["kind"] == "plan_proposed" && event["payload"]["review"]["approved"] == true
            })
    });
    let proposal_id = plans()["plans"][0]["id"].as_str().unwrap().to_owned();
    daemon
        .client
        .call(&Request::Conversation(
            antislop::lean::daemon::ConversationRequest::Resume {
                conversation: conversation.clone(),
                operation_id: None,
                plan: proposal_id,
                revision_token: None,
            },
        ))
        .unwrap();
    common::wait_until("both fake workers start and block at the barrier", || {
        let Some(active_plan_id) = plans()["active_plan_id"].as_str().map(str::to_owned) else {
            return false;
        };
        daemon
            .client
            .call(&Request::Conversation(
                antislop::lean::daemon::ConversationRequest::Plan {
                    conversation: conversation.clone(),
                    plan: active_plan_id,
                },
            ))
            .is_ok_and(|plan| {
                plan["tasks"].as_array().is_some_and(|tasks| {
                    tasks.len() == 2 && tasks.iter().all(|task| task["status"] == "working")
                })
            })
    });
    common::wait_until("both fake provider terminals are active", || {
        std::fs::read_to_string(run.root.join("provider-input.log"))
            .is_ok_and(|log| log.lines().filter(|line| *line == "START").count() == 2)
    });
    let (mut workbench_app, workbench_updates) = App::test_app_with_client(proxy.client());
    workbench_app.open(conversation.clone());
    refresh_app_from_core(&mut workbench_app, &daemon.client, &conversation);
    assert_eq!(workbench_app.view.tasks.len(), 2);
    assert_eq!(
        workbench_app
            .view
            .tasks
            .iter()
            .map(|task| task.id.as_str())
            .collect::<std::collections::BTreeSet<_>>(),
        ["t1", "t2"].into_iter().collect(),
        "the eligibility fixture contains exactly its two workers"
    );
    assert!(
        workbench_app
            .view
            .tasks
            .iter()
            .all(|task| task.uuid.is_some())
    );
    let other_conversation = daemon
        .client
        .call(&Request::Control(ControlRequest::StartIn {
            project: project.to_string_lossy().into_owned(),
        }))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(
        other_conversation, conversation,
        "late reply must target another session"
    );
    app.open(other_conversation.clone());
    app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    app.input = InputBuffer::with_text("late reply draft");
    proxy.hold_next_send_reply();
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let late_request = proxy.request("send");
    assert_eq!(late_request["conversation"], other_conversation);
    app.open(conversation.clone());
    app.input = InputBuffer::with_text("draft in the active session");
    proxy.release_reply();
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut late_reply_applied = false;
    while Instant::now() < deadline && !late_reply_applied {
        let update = updates
            .recv_timeout(Duration::from_secs(1))
            .expect("late send reply");
        if matches!(&update, Update::SendResult { conversation: id, .. } if id == &other_conversation)
        {
            late_reply_applied = true;
        }
        app.apply_update(update);
    }
    assert!(
        late_reply_applied,
        "proxy must release the real daemon response after switching sessions"
    );
    assert_eq!(app.conversation, conversation);
    assert_eq!(app.input.text, "draft in the active session");
    app.input.clear();
    app.key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('@'), KeyModifiers::NONE));
    assert!(app.picker.is_some(), "@ opens the real recipient picker");
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.input.text, "@all ");
    for ch in "broadcast from composer".chars() {
        app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
    }
    let plan_count_before_broadcast = plans()["total"].as_u64().unwrap();
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let broadcast = proxy.request("send");
    let broadcast_operation = broadcast["operation_id"]
        .as_str()
        .expect("composer send carries an operation correlation id");
    assert!(!broadcast_operation.is_empty());
    let confirmation = pump_send(&mut app, &updates);
    let mail = daemon
        .client
        .call(&Request::History(HistoryRequest::Mail {
            conversation: conversation.clone(),
        }))
        .unwrap();
    let broadcast_plan_id = plans()["plans"][0]["id"].as_str().unwrap().to_owned();
    let broadcast_plan = daemon
        .client
        .call(&Request::Conversation(
            antislop::lean::daemon::ConversationRequest::Plan {
                conversation: conversation.clone(),
                plan: broadcast_plan_id,
            },
        ))
        .unwrap();
    let mut expected_recipients = broadcast_plan["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| format!("worker:{}", task["id"].as_str().unwrap()))
        .collect::<Vec<_>>();
    expected_recipients.sort();
    let mut active_recipients = workbench_app
        .view
        .tasks
        .iter()
        .map(|task| format!("worker:{}", task.uuid.as_deref().unwrap()))
        .collect::<Vec<_>>();
    active_recipients.sort();
    assert_eq!(
        expected_recipients, active_recipients,
        "the Core plan and real workbench fixture agree on eligible broadcast recipients"
    );
    let entries = mail.as_array().unwrap();
    let mut actual_recipients = entries
        .iter()
        .filter(|entry| entry["text"] == "broadcast from composer")
        .map(|entry| entry["recipient"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    actual_recipients.sort();
    assert_eq!(actual_recipients, expected_recipients);
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry["text"] == "broadcast from composer")
            .count(),
        expected_recipients.len(),
        "each eligible worker receives one correlated broadcast mail"
    );
    assert!(
        entries
            .iter()
            .filter(|entry| entry["text"] == "broadcast from composer")
            .all(|entry| entry["state"] == "queued" && entry["delivered_turn"].is_null()),
        "queued is a receipt, not evidence that a worker executed the broadcast"
    );
    assert!(
        entries
            .iter()
            .all(|entry| entry["recipient"] != "orchestrator"
                || entry["text"] != "broadcast from composer"),
        "@all must use the broadcast recipients instead of the normal orchestrator route"
    );
    assert_eq!(
        confirmation,
        Ok(()),
        "daemon acknowledgement for composer request"
    );
    assert_eq!(
        app.conversation, conversation,
        "end view remains on original conversation"
    );
    assert!(
        app.input.is_empty(),
        "successful ordinary send clears the composer"
    );
    assert_eq!(
        plans()["total"],
        plan_count_before_broadcast,
        "broadcast does not create another orchestrator plan"
    );
    eprintln!(
        "sequence broadcast: picker=@all request={broadcast} confirmation={confirmation:?} recipients={actual_recipients:?} mail={:?}",
        entries
            .iter()
            .filter(|entry| entry["text"] == "broadcast from composer")
            .collect::<Vec<_>>()
    );

    // Use a fresh frontend for the real task workbench so no picker/modal
    // state from the earlier late-reply session switch can intercept keys.
    let mut task_ids = workbench_app
        .view
        .tasks
        .iter()
        .map(|task| task.id.clone())
        .collect::<Vec<_>>();
    task_ids.sort();
    for (task_id, mode, toggles, text) in [
        (&task_ids[0], "steer", 1, "steer while the worker is active"),
        (&task_ids[1], "interrupt", 2, "halt active worker"),
    ] {
        if workbench_app
            .details
            .as_ref()
            .is_some_and(|details| details.editing)
        {
            workbench_app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        }
        assert!(
            workbench_app
                .details
                .as_ref()
                .is_none_or(|details| !details.editing),
            "close the previous agent composer through App input before selecting another task"
        );
        workbench_app.open_details(format!("worker:{task_id}"));
        let deadline = Instant::now() + Duration::from_secs(15);
        while workbench_app
            .details
            .as_ref()
            .is_none_or(|details| details.task.is_none())
            && Instant::now() < deadline
        {
            let update = workbench_updates
                .recv_timeout(Duration::from_millis(250))
                .expect("real task details from daemon");
            workbench_app.apply_update(update);
        }
        let task = workbench_app
            .details
            .as_ref()
            .and_then(|details| details.task.clone())
            .expect("workbench loaded the real task record");
        if workbench_app.picker.is_some() {
            workbench_app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        }
        assert!(
            workbench_app.picker.is_none(),
            "close any restored picker before editing"
        );
        if workbench_app.details.as_ref().unwrap().searching {
            workbench_app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        }
        assert!(!workbench_app.details.as_ref().unwrap().searching);
        assert!(
            workbench_app.details.as_ref().unwrap().overview(),
            "agent composer shortcut requires the Summary view"
        );
        assert!(
            workbench_app.details.as_ref().unwrap().task_uuid.is_some(),
            "Core task event must resolve the recipient UUID"
        );
        assert!(
            !workbench_app.details.as_ref().unwrap().editing,
            "agent composer begins closed"
        );
        // 'a' is the real Summary-view action that opens the agent composer.
        // Keep the transition on App::key so a shortcut/routing regression
        // fails before any draft or send assertions can mask it.
        workbench_app.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        assert!(
            workbench_app.details.as_ref().unwrap().editing,
            "agent composer opens from the Summary-view 'a' shortcut: tab={}, task_uuid={:?}, focus={:?}, selected_round={:?}, native_focus={}, native_input={}, diff={}",
            workbench_app.details.as_ref().unwrap().tab,
            workbench_app.details.as_ref().unwrap().task_uuid,
            workbench_app.navigation.focus,
            workbench_app.details.as_ref().unwrap().selected_round,
            workbench_app.details.as_ref().unwrap().native_focus,
            workbench_app.navigation.presentation().native_input,
            workbench_app.diff.is_some()
        );
        for ch in text.chars() {
            workbench_app.key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        for _ in 0..toggles {
            workbench_app.key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
        }
        let draft_key = workbench_app.details.as_ref().unwrap().composer_key();
        assert_eq!(
            workbench_app
                .details
                .as_ref()
                .unwrap()
                .drafts
                .borrow()
                .get(&draft_key)
                .map(|composer| composer.input.text.as_str()),
            Some(text),
            "workbench keystrokes build the agent draft before send"
        );
        workbench_app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let request = proxy.request("send_to_agent");
        let message = &request["message"];
        assert_eq!(message["conversation"], conversation);
        assert_eq!(message["plan"], workbench_app.view.plan.as_deref().unwrap());
        assert_eq!(
            message["recipient"],
            format!("worker:{}", task["id"].as_str().unwrap())
        );
        assert_eq!(message["delivery"], mode);
        assert_eq!(message["text"], text);
        assert_eq!(message["expected_round"], task["round"]);
        assert_eq!(message["expected_run"], task["_latest_run"]);
        let operation = message["operation_id"].as_str().unwrap().to_owned();
        assert!(!operation.is_empty());
        let receipt = wait_agent_confirmation(&mut workbench_app, &workbench_updates, &operation);
        assert_eq!(
            receipt["state"], "steered",
            "{mode} must reach the active provider turn"
        );
        let delivered_turn = receipt["delivered_turn"]
            .as_str()
            .filter(|turn| !turn.is_empty())
            .expect("Core binds the delivery to a real running turn")
            .to_owned();
        let history = daemon
            .client
            .call(&Request::History(HistoryRequest::Mail {
                conversation: conversation.clone(),
            }))
            .unwrap();
        let delivered = history.as_array().unwrap().iter().find(|entry| {
            entry["text"] == text
                && entry["recipient"] == message["recipient"]
                && entry["id"] == receipt["mail"]
                && entry["delivered_turn"] == delivered_turn
        });
        assert!(
            delivered.is_some(),
            "Core history must bind {mode} to the running turn: {history}"
        );
        assert!(
            event_values(&daemon.client, &conversation)
                .iter()
                .any(|event| {
                    event["kind"] == "mail_delivered"
                        && event["payload"]["state"] == "steered"
                        && event["payload"]["recipient"] == message["recipient"]
                        && event["payload"]["ids"]
                            .as_array()
                            .is_some_and(|ids| ids.contains(&receipt["mail"]))
                })
        );
        let details = workbench_app.details.as_ref().unwrap();
        let draft_key = details.composer_key();
        let drafts = details.drafts.borrow();
        assert_eq!(drafts[&draft_key].delivery.as_deref(), Some("steered"));
        assert!(drafts[&draft_key].input.is_empty());
        eprintln!(
            "sequence {mode}: request={message} receipt={receipt} delivered_turn={delivered_turn}"
        );
    }
    let provider_input = std::fs::read_to_string(run.root.join("provider-input.log")).unwrap();
    assert!(
        provider_input.contains("steer while the worker is active"),
        "provider input: {provider_input:?}"
    );
    assert!(
        provider_input.contains("halt active worker"),
        "provider input: {provider_input:?}"
    );
    assert!(
        provider_input.contains("ESCAPE:"),
        "interrupt must send the terminal Escape control before its message: {provider_input:?}"
    );
    let running_plan = workbench_app.view.plan.clone().unwrap();
    refresh_app_from_core(&mut workbench_app, &daemon.client, &conversation);
    assert_eq!(workbench_app.conversation, conversation);
    let core_plan = daemon
        .client
        .call(&Request::Conversation(
            antislop::lean::daemon::ConversationRequest::Plan {
                conversation: conversation.clone(),
                plan: running_plan,
            },
        ))
        .unwrap();
    let core_statuses = core_plan["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|task| {
            (
                task["id"].as_str().unwrap(),
                task["status"].as_str().unwrap(),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(workbench_app.view.tasks.len(), core_statuses.len());
    assert!(
        workbench_app.view.tasks.iter().all(|task| {
            core_statuses.get(task.uuid.as_deref().unwrap_or_default())
                == Some(&task.status.as_str())
        }),
        "final workbench status matches current Core status: core={core_statuses:?}, view={:?}",
        workbench_app.view.tasks
    );
    let final_conversation = daemon
        .client
        .call(&Request::History(HistoryRequest::History {
            conversation: conversation.clone(),
            after: 0,
            limit: Some(100),
        }))
        .unwrap();
    assert!(
        final_conversation["events"]
            .as_array()
            .is_some_and(|events| { events.iter().any(|event| event["kind"] == "mail_delivered") }),
        "final conversation history contains the real delivery confirmations: {final_conversation}"
    );
    eprintln!(
        "sequence final view: conversation={} workbench_statuses={core_statuses:?} delivery_events={}",
        workbench_app.conversation,
        final_conversation["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|event| event["kind"] == "mail_delivered")
            .count()
    );
    std::fs::write(run.root.join("release-worker"), "release").unwrap();
    assert_eq!(
        broadcast["conversation"], conversation,
        "the composer request is correlated to the session that opened the picker"
    );
    assert_eq!(
        broadcast["text"], "@all broadcast from composer",
        "the selected @all composer address reaches the daemon in the submitted draft"
    );
    assert_eq!(
        broadcast["text"]
            .as_str()
            .and_then(|text| text.strip_prefix("@all ")),
        Some("broadcast from composer"),
        "the Core broadcast body is the selected composer draft after address parsing"
    );
}
