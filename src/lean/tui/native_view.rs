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
pub(super) struct View {
    state: Arc<std::sync::Mutex<Snapshot>>,
    commands: SyncSender<Command>,
    stop: Arc<AtomicBool>,
    dimensions: (u16, u16),
}
impl View {
    fn start(client: Client, conversation: String, role: String, columns: u16, rows: u16) -> Self {
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
            while !stopped.load(Ordering::SeqCst) {
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
                match received.recv_timeout(Duration::from_millis(80)) {
                    Ok(Command::Retry) => {
                        if let Some(old) = screen.take() {
                            let _ = client.call(&Request::TerminalDetach {
                                conversation: conversation.clone(),
                                role: role.clone(),
                                handle: old.handle,
                            });
                        }
                        output.lock().expect("native view").error = None;
                        sequence = 0;
                        retry = true;
                        continue;
                    }
                    Ok(command) => {
                        if let Some(current) = &screen {
                            let request = match command {
                                Command::Input(input) => {
                                    sequence += 1;
                                    Request::TerminalInput {
                                        conversation: conversation.clone(),
                                        role: role.clone(),
                                        handle: current.handle.clone(),
                                        sequence,
                                        input,
                                    }
                                }
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
                            if let Err(error) = client.call(&request) {
                                output.lock().expect("native view").error = Some(error.to_string());
                            }
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
                .and_then(|v| v.send(Command::Input(input)));
            if let Err(error) = result {
                self.notice = Some(error.to_string());
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
