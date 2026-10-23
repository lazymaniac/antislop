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
}
impl View {
    fn start(
        client: Client,
        conversation: String,
        role: String,
        columns: u16,
        rows: u16,
        updates: mpsc::Sender<Update>,
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
                        .is_none_or(|error| error.contains("waiting for the first native turn"))
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
                                Command::Resize(columns, rows) => {
                                    dimensions = (columns, rows);
                                    Request::TerminalResize {
                                        conversation: conversation.clone(),
                                        role: role.clone(),
                                        handle: current.handle.clone(),
                                        columns,
                                        rows,
                                    }
                                }
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
        if let Some(details) = self.details.as_ref().filter(|d| {
            d.tab == 0 && d.selected_round.is_none() && !d.native_history && !d.show_info
        }) {
            for (panel, key) in details.session_keys().iter().enumerate() {
                let role = details.composer_recipient(key);
                let backend = details
                    .task
                    .as_ref()
                    .and_then(|task| {
                        task["native_sessions"]
                            .as_array()
                            .or_else(|| task["turns"].as_array())
                    })
                    .and_then(|turns| turns.iter().rev().find(|turn| turn["role"] == role))
                    .and_then(|turn| turn["backend"].as_str())
                    .or_else(|| {
                        details
                            .sessions
                            .iter()
                            .rev()
                            .find(|session| session.role == role)
                            .and_then(|session| session.backend.as_deref())
                    });
                if matches!(backend, Some("codex_app_server" | "opencode_server")) {
                    let area = details.native_areas[panel];
                    wanted.push((role, key.clone(), area.width.max(1), area.height.max(1)));
                }
            }
        }
        let prefix = format!("{}:", self.conversation);
        self.native_views.retain(|key, _| {
            wanted
                .iter()
                .any(|(role, _, _, _)| *key == format!("{prefix}{role}"))
        });
        for (role, local_key, columns, rows) in wanted {
            let key = format!("{prefix}{role}");
            let controller = self.native_views.entry(key).or_insert_with(|| {
                View::start(
                    self.client.clone(),
                    self.conversation.clone(),
                    role.clone(),
                    columns,
                    rows,
                    self.updates.clone(),
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
        let details = self.details.as_ref().filter(|d| {
            d.tab == 0 && d.selected_round.is_none() && !d.native_history && !d.show_info
        })?;
        details
            .session_keys()
            .get(details.active_panel)
            .map(|key| details.composer_recipient(key))
    }
    pub(super) fn native_focused(&self) -> bool {
        self.details.as_ref().is_some_and(|d| d.native_focus) && self.native_key_role().is_some()
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
                details.native_history = true;
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
    fn a_lost_input_acknowledgement_reuses_the_same_sequence() {
        let directory = tempfile::tempdir_in("/private/tmp").unwrap();
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
        let directory = tempfile::tempdir_in("/private/tmp").unwrap();
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
        assert!(details.editing && details.native_history);
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
    fn overview_splits_at_eighty_columns_and_old_rounds_keep_json_history() {
        let mut details = Details::new("worker:t1".into());
        let view = crate::lean::tui::view::View::default();
        for key in ["worker:t1", "verifier:t1"] {
            details.native.insert(
                key.into(),
                Snapshot {
                    screen: Some(screen("native-marker")),
                    error: None,
                },
            );
        }
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        terminal.draw(|frame| details.draw(frame, &view)).unwrap();
        assert_ne!(details.native_areas[0].x, details.native_areas[1].x);
        assert!(details.native_areas[0].width < 40);
        details.selected_round = Some(1);
        terminal.draw(|frame| details.draw(frame, &view)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|c| c.symbol())
            .collect::<String>();
        assert!(!text.contains("native-marker"));
    }
}
