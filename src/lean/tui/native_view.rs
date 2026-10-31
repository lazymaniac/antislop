use super::*;
use crate::lean::terminal::{Input, Screen};
use std::sync::mpsc::{self, SyncSender};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Snapshot {
    pub screen: Option<Screen>,
    pub error: Option<String>,
}
pub(super) enum Command {
    Input(Input),
    Resize(u16, u16),
    Retry,
}

mod app;
mod render;
mod session;
pub(super) use self::render::render;

pub(super) struct View {
    state: Arc<std::sync::Mutex<Snapshot>>,
    commands: SyncSender<Command>,
    stop: Arc<AtomicBool>,
    dimensions: (u16, u16),
    identity: String,
}
#[cfg(test)]
mod tests {
    use super::session::call_input;
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};
    use serde_json::json;
    use std::time::Instant;
    #[test]
    fn resize_while_waiting_for_a_session_is_used_by_the_next_open() {
        std::fs::create_dir_all(".antislop/tmp").unwrap();
        let directory = tempfile::tempdir_in(".antislop/tmp").unwrap();
        let listener =
            std::os::unix::net::UnixListener::bind(crate::lean::daemon::socket(directory.path()))
                .unwrap();
        let (arrived, first) = mpsc::sync_channel(1);
        let (resume, ready) = mpsc::sync_channel(1);
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let request: crate::lean::daemon::RequestEnvelope =
                crate::transport::read(&mut std::io::BufReader::new(stream.try_clone().unwrap()))
                    .unwrap()
                    .unwrap();
            let request = request.request;
            assert!(matches!(
                request,
                Request::Terminal(crate::lean::daemon::TerminalRequest::TerminalOpen {
                    columns: 80,
                    rows: 24,
                    ..
                })
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
                let request: crate::lean::daemon::RequestEnvelope = crate::transport::read(
                    &mut std::io::BufReader::new(stream.try_clone().unwrap()),
                )
                .unwrap()
                .unwrap();
                let request = request.request;
                let detached = matches!(
                    request,
                    Request::Terminal(crate::lean::daemon::TerminalRequest::TerminalDetach { .. })
                );
                let value = match request {
                    Request::Terminal(crate::lean::daemon::TerminalRequest::TerminalOpen {
                        columns,
                        rows,
                        ..
                    }) => {
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
            super::update_channel(256).0,
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
        std::fs::create_dir_all(".antislop/tmp").unwrap();
        let directory = tempfile::tempdir_in(".antislop/tmp").unwrap();
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
        std::fs::create_dir_all(".antislop/tmp").unwrap();
        let directory = tempfile::tempdir_in(".antislop/tmp").unwrap();
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
                let request = &request["request"];
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
            &Request::Terminal(crate::lean::daemon::TerminalRequest::TerminalInput {
                conversation: "conversation".into(),
                role: "worker".into(),
                handle: "view".into(),
                sequence: 7,
                input: Input::Paste {
                    text: "draft 👩‍💻".into(),
                },
            }),
        )
        .unwrap();
        server.join().unwrap();
    }
    #[test]
    fn closing_a_view_delivers_accepted_input_before_detaching() {
        std::fs::create_dir_all(".antislop/tmp").unwrap();
        let directory = tempfile::tempdir_in(".antislop/tmp").unwrap();
        let listener =
            std::os::unix::net::UnixListener::bind(crate::lean::daemon::socket(directory.path()))
                .unwrap();
        let (finished, result) = mpsc::sync_channel(1);
        let server = std::thread::spawn(move || {
            let mut sequences = vec![];
            loop {
                let (mut stream, _) = listener.accept().unwrap();
                let request: crate::lean::daemon::RequestEnvelope = crate::transport::read(
                    &mut std::io::BufReader::new(stream.try_clone().unwrap()),
                )
                .unwrap()
                .unwrap();
                let request = request.request;
                let detach = matches!(
                    request,
                    Request::Terminal(crate::lean::daemon::TerminalRequest::TerminalDetach { .. })
                );
                let data = match request {
                    Request::Terminal(crate::lean::daemon::TerminalRequest::TerminalOpen {
                        ..
                    }) => serde_json::to_value(screen("ready")).unwrap(),
                    Request::Terminal(crate::lean::daemon::TerminalRequest::TerminalInput {
                        sequence,
                        ..
                    }) => {
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
            super::update_channel(256).0,
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
