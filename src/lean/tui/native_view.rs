use super::*;
use crate::lean::terminal::{Input, Screen};
use std::sync::mpsc::{self, SyncSender};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub screen: Option<Screen>,
    pub error: Option<String>,
}
enum Command {
    Input(Input),
    Resize(u16, u16),
    Retry,
}

fn call_input(client: &Client, request: &Request) -> Result<Value> {
    // TerminalInput is deduplicated by handle and sequence. Reconnect with
    // the same request if the daemon accepted it but its reply was lost.
    let mut result = client.call(request);
    for _ in 0..2 {
        let transient = match &result {
            Err(crate::Error::Io(_)) => true,
            Err(crate::Error::Invalid(message)) => {
                message == "the daemon disconnected"
                    || message.starts_with("terminal input not accepted:")
            }
            _ => false,
        };
        if !transient {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
        result = client.call(request);
    }
    result
}
pub(super) struct View {
    state: Arc<std::sync::Mutex<Snapshot>>,
    commands: SyncSender<Command>,
    stop: Arc<AtomicBool>,
    dimensions: (u16, u16),
    identity: String,
}
impl View {
    fn start(
        client: Client,
        conversation: String,
        role: String,
        columns: u16,
        rows: u16,
        updates: mpsc::Sender<Update>,
        identity: String,
    ) -> Self {
        let state = Arc::new(std::sync::Mutex::new(Snapshot::default()));
        let output = state.clone();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let (commands, received) = mpsc::sync_channel(64);
        std::thread::spawn(move || {
            let mut screen: Option<Screen> = None;
            let mut sequence = 0;
            let mut dimensions = (columns, rows);
            let mut retry = false;
            let mut blocked = None::<String>;
            loop {
                // Finish accepted keystrokes before detaching a closed view.
                let closing = stopped.load(Ordering::SeqCst);
                if screen.is_none()
                    && output
                        .lock()
                        .expect("native view")
                        .error
                        .as_deref()
                        .is_none_or(|error| {
                            error.contains("native session has not started")
                                || error.contains("has not reported this session's identifier")
                        })
                {
                    match client
                        .call(&Request::TerminalOpen {
                            conversation: conversation.clone(),
                            role: role.clone(),
                            columns: dimensions.0,
                            rows: dimensions.1,
                            retry,
                        })
                        .and_then(|value| serde_json::from_value(value).map_err(Into::into))
                    {
                        Ok(value) => screen = Some(value),
                        Err(error) => {
                            output.lock().expect("native view").error = Some(error.to_string())
                        }
                    }
                }
                let command = if closing {
                    match received.try_recv() {
                        Ok(command) => Ok(command),
                        Err(_) => break,
                    }
                } else {
                    received.recv_timeout(Duration::from_millis(80))
                };
                match command {
                    Ok(Command::Retry) => {
                        if let Some(old) = screen.take() {
                            let _ = client.call(&Request::TerminalDetach {
                                conversation: conversation.clone(),
                                role: role.clone(),
                                handle: old.handle,
                            });
                        }
                        *output.lock().expect("native view") = Snapshot::default();
                        sequence = 0;
                        blocked = None;
                        retry = true;
                        continue;
                    }
                    Ok(command) => {
                        if let Command::Resize(columns, rows) = &command {
                            dimensions = (*columns, *rows);
                        }
                        if let Command::Input(input) = &command
                            && let Some(error) = &blocked
                        {
                            let _ = updates.send(Update::NativeInputRejected {
                                conversation: conversation.clone(),
                                role: role.clone(),
                                input: input.clone(),
                                error: error.clone(),
                            });
                            continue;
                        }
                        if let Some(current) = &screen {
                            let request = match command {
                                Command::Input(input) => Request::TerminalInput {
                                    conversation: conversation.clone(),
                                    role: role.clone(),
                                    handle: current.handle.clone(),
                                    sequence: sequence + 1,
                                    input,
                                },
                                Command::Resize(columns, rows) => Request::TerminalResize {
                                    conversation: conversation.clone(),
                                    role: role.clone(),
                                    handle: current.handle.clone(),
                                    columns,
                                    rows,
                                },
                                Command::Retry => unreachable!(),
                            };
                            let result = if matches!(request, Request::TerminalInput { .. }) {
                                call_input(&client, &request)
                            } else {
                                client.call(&request)
                            };
                            match result {
                                Ok(_) => {
                                    if matches!(request, Request::TerminalInput { .. }) {
                                        sequence += 1;
                                    }
                                    output.lock().expect("native view").error = None;
                                }
                                Err(error) => {
                                    if let Request::TerminalInput { input, .. } = &request {
                                        blocked = Some(error.to_string());
                                        let _ = updates.send(Update::NativeInputRejected {
                                            conversation: conversation.clone(),
                                            role: role.clone(),
                                            input: input.clone(),
                                            error: error.to_string(),
                                        });
                                    }
                                    output.lock().expect("native view").error =
                                        Some(error.to_string());
                                }
                            }
                        } else if let Command::Input(input) = command {
                            let _ = updates.send(Update::NativeInputRejected {
                                conversation: conversation.clone(),
                                role: role.clone(),
                                input,
                                error: "native terminal is not ready".into(),
                            });
                        }
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
                if let Some(current) = &screen {
                    match client.call(&Request::TerminalRead {
                        conversation: conversation.clone(),
                        role: role.clone(),
                        handle: current.handle.clone(),
                        after: current.revision,
                    }) {
                        Ok(value) if !value.is_null() => {
                            match serde_json::from_value::<Screen>(value) {
                                Ok(next) => screen = Some(next),
                                Err(error) => {
                                    output.lock().expect("native view").error =
                                        Some(error.to_string())
                                }
                            }
                        }
                        Err(error) => {
                            output.lock().expect("native view").error = Some(error.to_string())
                        }
                        _ => {}
                    }
                }
                output.lock().expect("native view").screen = screen.clone();
            }
            if let Some(current) = screen {
                let _ = client.call(&Request::TerminalDetach {
                    conversation,
                    role,
                    handle: current.handle,
                });
            }
        });
        Self {
            state,
            commands,
            stop,
            dimensions: (columns, rows),
            identity,
        }
    }
    fn send(&self, command: Command) -> Result<()> {
        self.commands.try_send(command).map_err(|_| {
            crate::Error::Invalid("native terminal input queue is full or disconnected".into())
        })
    }
    fn resize(&mut self, columns: u16, rows: u16) {
        if self.dimensions != (columns, rows) && self.send(Command::Resize(columns, rows)).is_ok() {
            self.dimensions = (columns, rows);
        }
    }
}
impl Drop for View {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

pub(super) fn render(frame: &mut Frame, area: Rect, screen: &Screen, focused: bool) {
    use ratatui::style::{Color, Modifier, Style};
    let buffer = frame.buffer_mut();
    for cell in &screen.cells {
        if cell.x >= area.width
            || cell.y >= area.height
            || cell.x.saturating_add(cell.width) > area.width
        {
            continue;
        }
        let mut style = Style::default();
        if let Some([r, g, b]) = cell.foreground {
            style = style.fg(Color::Rgb(r, g, b));
        }
        if let Some([r, g, b]) = cell.background {
            style = style.bg(Color::Rgb(r, g, b));
        }
        for (bit, modifier) in [
            (1, Modifier::BOLD),
            (2, Modifier::DIM),
            (4, Modifier::ITALIC),
            (8, Modifier::UNDERLINED),
            (16, Modifier::REVERSED),
        ] {
            if cell.modifiers & bit != 0 {
                style = style.add_modifier(modifier);
            }
        }
        buffer.set_string(area.x + cell.x, area.y + cell.y, &cell.text, style);
    }
    if focused
        && let Some((x, y)) = screen.cursor
        && x < area.width
        && y < area.height
    {
        frame.set_cursor_position((area.x + x, area.y + y));
    }
}

impl App {
    pub(super) fn sync_native_views(&mut self) {
        let mut wanted = vec![];
        // The current round's sessions are shown live; an earlier round's only
        // when the user asks, and then by resuming it.
        if let Some(details) = self
            .details
            .as_ref()
            .filter(|d| d.terminal_visible() && (d.selected_round.is_none() || d.resume_round))
        {
            for (panel, key) in details.session_keys().iter().enumerate() {
                let role = details.composer_recipient(key);
                let area = details.native_areas[panel];
                if area.is_empty() {
                    continue;
                }
                let (role, identity) = match details.selected_round {
                    Some(round) => (format!("{role}#r{round}"), format!("round:{round}")),
                    None => {
                        // A new provider session (or a new task round) is a new
                        // terminal: the view attaches to it afresh.
                        let metadata = details
                            .task
                            .as_ref()
                            .and_then(|task| {
                                task["native_sessions"]
                                    .as_array()
                                    .or_else(|| task["turns"].as_array())
                            })
                            .and_then(|turns| turns.iter().rev().find(|turn| turn["role"] == role));
                        let stored = details
                            .sessions
                            .iter()
                            .rev()
                            .find(|session| session.role == role)
                            .filter(|_| details.task.is_none());
                        let session = metadata
                            .and_then(|turn| turn["session_id"].as_str())
                            .or_else(|| stored.and_then(|session| session.session_id.as_deref()));
                        let round = details
                            .task
                            .as_ref()
                            .filter(|_| crate::lean::task_role(&role).is_some())
                            .map(|task| task["round"].to_string())
                            .unwrap_or_default();
                        (
                            role.clone(),
                            format!("{}:{round}", session.unwrap_or("waiting")),
                        )
                    }
                };
                wanted.push((
                    role,
                    key.clone(),
                    identity,
                    area.width.max(1),
                    area.height.max(1),
                ));
            }
        }
        let prefix = format!("{}:", self.conversation);
        let mut retired = false;
        self.native_views.retain(|key, view| {
            let keep = wanted.iter().any(|(role, _, identity, _, _)| {
                *key == format!("{prefix}{role}") && view.identity == *identity
            });
            retired |= !keep;
            keep
        });
        if let Some(details) = &mut self.details {
            details.native.clear();
            if retired {
                details.native_focus = false;
            }
        }
        for (role, local_key, identity, columns, rows) in wanted {
            let key = format!("{prefix}{role}");
            let controller = self.native_views.entry(key).or_insert_with(|| {
                View::start(
                    self.client.clone(),
                    self.conversation.clone(),
                    role.clone(),
                    columns,
                    rows,
                    self.updates.clone(),
                    identity,
                )
            });
            controller.resize(columns, rows);
            let snapshot = controller.state.lock().expect("native view").clone();
            if let Some(details) = &mut self.details {
                details.native.insert(local_key, snapshot);
            }
        }
    }
    fn native_key_role(&self) -> Option<String> {
        let details = self
            .details
            .as_ref()
            .filter(|d| d.terminal_visible() && (d.selected_round.is_none() || d.resume_round))?;
        details
            .session_keys()
            .get(details.active_panel)
            .map(|key| details.composer_recipient(key))
    }
    pub(super) fn native_focused(&self) -> bool {
        !self.ledger_view.as_ref().is_some_and(|v| v.visible)
            && self.details.as_ref().is_some_and(|d| d.native_focus)
            && self.native_key_role().is_some()
    }
    pub(super) fn native_send(&mut self, input: Input) {
        if let Some(role) = self.native_key_role() {
            let result = self
                .native_views
                .get(&format!("{}:{role}", self.conversation))
                .ok_or_else(|| crate::Error::Invalid("native terminal is not ready".into()))
                .and_then(|v| v.send(Command::Input(input.clone())));
            if let Err(error) = result {
                self.recover_native_input(
                    &self.conversation.clone(),
                    &role,
                    input,
                    &error.to_string(),
                );
            }
        }
    }
    pub(super) fn recover_native_input(
        &mut self,
        conversation: &str,
        role: &str,
        input: Input,
        error: &str,
    ) {
        let text = match input {
            Input::Paste { text } => text,
            Input::Key { event }
                if !event
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                match event.code {
                    KeyCode::Char(ch) => ch.to_string(),
                    KeyCode::Enter => "\n".into(),
                    _ => String::new(),
                }
            }
            _ => String::new(),
        };
        let message = format!(
            "Native input could not be confirmed; text recovered in message draft. Check the terminal before resending: {error}"
        );
        let mut drafts = self.agent_drafts.borrow_mut();
        let draft = drafts
            .entry((conversation.into(), role.into()))
            .or_default();
        draft.input.insert_str(&text);
        draft.error = Some(message.clone());
        drop(drafts);
        if self.conversation == conversation {
            self.notice = Some(message);
            if let Some(details) = self.details.as_mut()
                && details
                    .session_keys()
                    .iter()
                    .any(|key| details.composer_recipient(key) == role)
            {
                details.native_focus = false;
                details.editing = true;
            }
        }
    }
    pub(super) fn native_retry(&mut self) {
        if let Some(role) = self.native_key_role()
            && let Some(view) = self
                .native_views
                .get(&format!("{}:{role}", self.conversation))
        {
            let _ = view.send(Command::Retry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    use serde_json::json;
    use std::time::Instant;
    #[test]
    fn resize_while_waiting_for_a_session_is_used_by_the_next_open() {
        let directory = tempfile::tempdir_in("/tmp").unwrap();
        let listener =
            std::os::unix::net::UnixListener::bind(crate::lean::daemon::socket(directory.path()))
                .unwrap();
        let (arrived, first) = mpsc::sync_channel(1);
        let (resume, ready) = mpsc::sync_channel(1);
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request: Request =
                crate::transport::read(&mut std::io::BufReader::new(stream.try_clone().unwrap()))
                    .unwrap()
                    .unwrap();
            assert!(matches!(
                request,
                Request::TerminalOpen {
                    columns: 80,
                    rows: 24,
                    ..
                }
            ));
            arrived.send(()).unwrap();
            ready.recv_timeout(Duration::from_secs(3)).unwrap();
            crate::transport::write(
                &mut stream,
                &json!({"error":"native session has not started","data":null}),
            )
            .unwrap();
            drop(stream);
            let mut opened = false;
            loop {
                let (mut stream, _) = listener.accept().unwrap();
                let request: Request = crate::transport::read(&mut std::io::BufReader::new(
                    stream.try_clone().unwrap(),
                ))
                .unwrap()
                .unwrap();
                let detached = matches!(request, Request::TerminalDetach { .. });
                let value = match request {
                    Request::TerminalOpen { columns, rows, .. } => {
                        assert_eq!((columns, rows), (40, 8));
                        opened = true;
                        let mut value = screen("ready");
                        value.columns = columns;
                        value.rows = rows;
                        serde_json::to_value(value).unwrap()
                    }
                    _ => Value::Null,
                };
                crate::transport::write(&mut stream, &json!({"data":value,"error":null})).unwrap();
                if detached {
                    assert!(opened);
                    break;
                }
            }
        });
        let mut view = View::start(
            Client::at(directory.path()),
            "conversation".into(),
            "worker".into(),
            80,
            24,
            mpsc::channel().0,
            "fixture".into(),
        );
        first.recv_timeout(Duration::from_secs(3)).unwrap();
        view.resize(40, 8);
        resume.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(screen) = &view.state.lock().unwrap().screen {
                assert_eq!((screen.columns, screen.rows), (40, 8));
                break;
            }
            assert!(
                Instant::now() < deadline,
                "session did not open after resize"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        drop(view);
        server.join().unwrap();
    }
    #[test]
    fn first_attachment_waits_for_the_actual_panel_dimensions() {
        let directory = tempfile::tempdir_in("/tmp").unwrap();
        let (mut app, _) = App::test_app_with_client(Client::at(directory.path()));
        let mut details = Details::new("orchestrator".into());
        details.task = Some(json!({"turns":[{"role":"orchestrator","session_id":"managed"}]}));
        app.details = Some(details);
        app.sync_native_views();
        assert!(app.native_views.is_empty());
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let panel = app.details.as_ref().unwrap().native_areas[0];
        let controller = &app.native_views[&format!("{}:orchestrator", app.conversation)];
        assert_eq!(controller.dimensions, (panel.width, panel.height));
        assert!(panel.width > 1 && panel.height > 1);
    }
    #[test]
    fn overview_follows_session_rotation_without_reusing_a_stale_screen() {
        let mut app = App::test_app();
        let mut details = Details::new("orchestrator".into());
        details.task =
            Some(json!({"round":1,"native_sessions":[{"role":"orchestrator","session_id":"old"}]}));
        details.native_focus = true;
        details.native_areas[0] = Rect::new(1, 1, 78, 20);
        app.details = Some(details);
        let (commands, _received) = mpsc::sync_channel(8);
        let state = Arc::new(std::sync::Mutex::new(Snapshot {
            screen: Some(screen("old-screen")),
            error: None,
        }));
        let stopped = Arc::new(AtomicBool::new(false));
        let key = format!("{}:orchestrator", app.conversation);
        app.native_views.insert(
            key.clone(),
            View {
                state: state.clone(),
                commands,
                stop: stopped.clone(),
                dimensions: (80, 24),
                identity: "old:".into(),
            },
        );
        app.sync_native_views();
        assert!(Arc::ptr_eq(&state, &app.native_views[&key].state));
        assert!(app.details.as_ref().unwrap().native_focus);
        app.details.as_mut().unwrap().task.as_mut().unwrap()["round"] = 2.into();
        // Root-role run indices are history positions, not new sessions.
        app.sync_native_views();
        assert!(Arc::ptr_eq(&state, &app.native_views[&key].state));
        assert!(app.details.as_ref().unwrap().native_focus);
        app.details.as_mut().unwrap().task.as_mut().unwrap()["native_sessions"][0]["session_id"] =
            "new".into();
        app.sync_native_views();
        assert!(stopped.load(Ordering::SeqCst));
        assert_eq!(app.native_views[&key].identity, "new:");
        let details = app.details.as_ref().unwrap();
        assert!(!details.native_focus);
        assert!(details.native["orchestrator"].screen.is_none());
        // A new round with no turn yet must not use stale session metadata.
        app.details.as_mut().unwrap().task.as_mut().unwrap()["native_sessions"] = json!([]);
        app.sync_native_views();
        assert_eq!(app.native_views[&key].identity, "waiting:");
        assert!(
            app.details.as_ref().unwrap().native["orchestrator"]
                .screen
                .is_none()
        );
    }
    #[test]
    fn a_lost_input_acknowledgement_reuses_the_same_sequence() {
        let directory = tempfile::tempdir_in("/tmp").unwrap();
        let listener =
            std::os::unix::net::UnixListener::bind(crate::lean::daemon::socket(directory.path()))
                .unwrap();
        let server = std::thread::spawn(move || {
            for attempt in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let request: Value = crate::transport::read(&mut std::io::BufReader::new(
                    stream.try_clone().unwrap(),
                ))
                .unwrap()
                .unwrap();
                assert_eq!(request["op"], "terminal_input");
                assert_eq!(request["sequence"], 7);
                assert_eq!(request["input"]["text"], "draft 👩‍💻");
                if attempt == 1 {
                    crate::transport::write(&mut stream, &json!({"data":null,"error":null}))
                        .unwrap();
                }
                // First delivery was accepted; emulate losing only its reply.
            }
        });
        call_input(
            &Client::at(directory.path()),
            &Request::TerminalInput {
                conversation: "conversation".into(),
                role: "worker".into(),
                handle: "view".into(),
                sequence: 7,
                input: Input::Paste {
                    text: "draft 👩‍💻".into(),
                },
            },
        )
        .unwrap();
        server.join().unwrap();
    }
    #[test]
    fn closing_a_view_delivers_accepted_input_before_detaching() {
        let directory = tempfile::tempdir_in("/tmp").unwrap();
        let listener =
            std::os::unix::net::UnixListener::bind(crate::lean::daemon::socket(directory.path()))
                .unwrap();
        let (finished, result) = mpsc::sync_channel(1);
        let server = std::thread::spawn(move || {
            let mut sequences = vec![];
            loop {
                let (mut stream, _) = listener.accept().unwrap();
                let request: Request = crate::transport::read(&mut std::io::BufReader::new(
                    stream.try_clone().unwrap(),
                ))
                .unwrap()
                .unwrap();
                let detach = matches!(request, Request::TerminalDetach { .. });
                let data = match request {
                    Request::TerminalOpen { .. } => serde_json::to_value(screen("ready")).unwrap(),
                    Request::TerminalInput { sequence, .. } => {
                        sequences.push(sequence);
                        Value::Null
                    }
                    _ => Value::Null,
                };
                crate::transport::write(&mut stream, &json!({"data":data,"error":null})).unwrap();
                if detach {
                    finished.send(sequences).unwrap();
                    break;
                }
            }
        });
        let view = View::start(
            Client::at(directory.path()),
            "conversation".into(),
            "worker".into(),
            80,
            24,
            mpsc::channel().0,
            "fixture".into(),
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        while view.state.lock().unwrap().screen.is_none() {
            assert!(Instant::now() < deadline, "terminal did not open");
            std::thread::sleep(Duration::from_millis(5));
        }
        for text in ["first", "second", "third"] {
            view.send(Command::Input(Input::Paste { text: text.into() }))
                .unwrap();
        }
        drop(view);
        assert_eq!(
            result.recv_timeout(Duration::from_secs(3)).unwrap(),
            [1, 2, 3]
        );
        server.join().unwrap();
    }
    fn screen(text: &str) -> Screen {
        Screen {
            handle: "view".into(),
            revision: 1,
            columns: 80,
            rows: 24,
            cursor: Some((0, 0)),
            error: None,
            cells: vec![crate::lean::terminal::Cell {
                x: 0,
                y: 0,
                text: text.into(),
                width: 2,
                foreground: Some([255, 0, 0]),
                background: None,
                modifiers: 16,
            }],
        }
    }
    #[test]
    fn native_focus_routes_control_c_escape_and_paste_before_global_shortcuts() {
        let mut app = App::test_app();
        let (commands, received) = mpsc::sync_channel(8);
        app.native_views.insert(
            format!("{}:orchestrator", app.conversation),
            View {
                state: Arc::new(std::sync::Mutex::new(Snapshot::default())),
                commands,
                stop: Arc::new(AtomicBool::new(false)),
                dimensions: (80, 24),
                identity: "fixture".into(),
            },
        );
        app.input.insert_str("background draft");
        let mut details = Details::new("orchestrator".into());
        details.native_focus = true;
        app.details = Some(details);
        assert!(app.key(event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )));
        assert!(app.key(event::KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)));
        assert!(app.details.is_some());
        app.paste("native input");
        assert!(
            matches!(received.try_recv().unwrap(), Command::Input(Input::Key { event }) if event.code == KeyCode::Char('c'))
        );
        assert!(
            matches!(received.try_recv().unwrap(), Command::Input(Input::Key { event }) if event.code == KeyCode::Esc)
        );
        assert!(
            matches!(received.try_recv().unwrap(), Command::Input(Input::Paste { text }) if text == "native input")
        );
        assert_eq!(app.input.text, "background draft");
        assert!(app.key(event::KeyEvent::new(
            KeyCode::Char(']'),
            KeyModifiers::CONTROL
        )));
        assert!(!app.details.as_ref().unwrap().native_focus);
        assert!(!app.key(event::KeyEvent::new(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL
        )));
    }
    #[test]
    fn unconfirmed_native_text_is_recovered_without_overwriting_other_drafts() {
        let mut app = App::test_app();
        let current = app.conversation.clone();
        app.input.insert_str("main draft");
        app.agent_drafts
            .borrow_mut()
            .entry((current.clone(), "orchestrator".into()))
            .or_default()
            .input
            .insert_str("existing ");
        let mut details = Details::new("orchestrator".into());
        details.drafts = app.agent_drafts.clone();
        details.native_focus = true;
        app.details = Some(details);
        for (conversation, text) in [(&current, "👩‍💻"), (&"another".into(), "other text")] {
            app.apply_update(Update::NativeInputRejected {
                conversation: conversation.clone(),
                role: "orchestrator".into(),
                input: Input::Paste { text: text.into() },
                error: "the daemon disconnected".into(),
            });
        }
        let drafts = app.agent_drafts.borrow();
        let draft = &drafts[&(current, "orchestrator".into())];
        assert_eq!(draft.input.text, "existing 👩‍💻");
        assert!(
            draft
                .error
                .as_deref()
                .unwrap()
                .contains("the daemon disconnected")
        );
        assert_eq!(
            drafts[&("another".into(), "orchestrator".into())]
                .input
                .text,
            "other text"
        );
        assert_eq!(app.input.text, "main draft");
        let details = app.details.as_ref().unwrap();
        assert!(!details.native_focus);
        assert!(details.editing);
    }
    #[test]
    fn native_cells_keep_graphemes_bounds_and_no_color_modifiers() {
        let mut terminal = Terminal::new(TestBackend::new(8, 4)).unwrap();
        terminal
            .draw(|frame| {
                render(frame, Rect::new(1, 1, 4, 2), &screen("👩‍💻"), true);
                without_color(frame.buffer_mut());
            })
            .unwrap();
        let cell = &terminal.backend().buffer()[(1, 1)];
        assert_eq!(cell.symbol(), "👩‍💻");
        assert_eq!(cell.fg, Color::Reset);
        assert!(cell.modifier.contains(Modifier::REVERSED));
        terminal
            .draw(|frame| render(frame, Rect::new(1, 1, 1, 1), &screen("👩‍💻"), true))
            .unwrap();
        assert_eq!(terminal.backend().buffer()[(1, 1)].symbol(), " ");
    }
    #[test]
    fn overview_shows_one_terminal_and_old_rounds_resume_on_request() {
        let mut details = Details::new("worker:t1".into());
        let view = crate::lean::tui::view::View::default();
        {
            details.native.insert(
                "worker:t1".into(),
                Snapshot {
                    screen: Some(screen("native-marker")),
                    error: None,
                },
            );
        }
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| details.draw(frame, &view)).unwrap();
        // One agent per tab: the worker's terminal takes the whole panel.
        assert!(details.native_areas[0].width >= 70);
        // An earlier round has no terminal until the user asks for it.
        details.selected_round = Some(1);
        details.native.clear();
        terminal.draw(|frame| details.draw(frame, &view)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(!text.contains("native-marker"));
        assert!(text.contains("o resume this session"));
    }
}
