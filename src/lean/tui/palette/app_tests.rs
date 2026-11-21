use super::*;

#[test]
fn picker_backspace_removes_one_grapheme_and_keeps_chat_draft_isolated() {
    use super::super::App;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut app = App::test_app();
    app.input.insert_str("chat draft");
    let mut picker = Picker::new(Kind::Commands);
    picker.query = "find 👩‍💻".into();
    app.picker = Some(picker);

    app.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));

    assert_eq!(app.picker.as_ref().unwrap().query, "find ");
    assert_eq!(app.input.text, "chat draft");
}

#[test]
fn help_key_ownership_preserves_picker_query_and_chat_draft() {
    use super::super::App;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut app = App::test_app();
    app.input.insert_str("unsent");
    app.picker = Some(Picker::new(Kind::Commands));
    app.picker.as_mut().unwrap().query = "plan".into();
    app.show_help(Help::Chat);

    app.key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));

    assert_eq!(app.input.text, "unsent");
    assert_eq!(app.picker.as_ref().unwrap().query, "plan");
    assert_eq!(app.help, Some(Help::Chat));
}

#[test]
fn recommendation_notice_preserves_focus_and_explicit_picker_preselects_rigor() {
    use super::super::{App, InputBuffer, Update};
    use crate::lean::tasks::Team;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};
    use serde_json::json;

    let mut app = App::test_app();
    app.input = InputBuffer::with_text("Unsent 👩‍💻 draft");
    let mut search = Picker::new(Kind::History);
    search.query = "saved query".into();
    app.picker = Some(search);
    app.apply_update(Update::Events {
        conversation: app.conversation.clone(),
        generation: app.generation,
        idle: false,
        observer: false,
        events: vec![json!({"seq":1,"kind":"team_selected","payload":{
            "team":"security-fix","reason":"Security-sensitive change", "origin":"planner"
        }})],
    });
    assert_eq!(app.picker.as_ref().unwrap().query, "saved query");
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.sync_rigor_recommendation();
    assert!(app.picker.is_none());
    app.run_workflow(Command::OpenRigorPicker);
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.kind, Kind::Rigor);
    assert_eq!(
        picker.items(&app.view)[picker.selected].1,
        Action::Run(Command::SelectTeam(Team::SecurityFix))
    );
    assert_eq!(
        picker.recommendation.as_ref().unwrap().1,
        "Security-sensitive change"
    );
    for (width, height) in [(80, 24), (30, 12), (12, 5)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        if width == 80 {
            let screen = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(screen.contains("Orchestrator recommends: security-fix"));
            assert!(screen.contains("Security-sensitive change"));
            assert!(screen.contains("small-change"));
        }
    }
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(
        picker.items(&app.view)[picker.selected].1,
        Action::Run(Command::SelectTeam(Team::CodeReview))
    );
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(
        picker.items(&app.view)[picker.selected].1,
        Action::Run(Command::SelectTeam(Team::NoQualityGates))
    );
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(
        picker.items(&app.view)[picker.selected].1,
        Action::Run(Command::SelectTeam(Team::Research))
    );
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    app.sync_rigor_recommendation();
    assert!(
        app.picker.is_none(),
        "a redrawn plan does not reopen the recommendation"
    );
    assert_eq!(app.input.text, "Unsent 👩‍💻 draft");
}

#[test]
fn user_team_selection_does_not_open_a_recommendation() {
    use super::super::App;
    use serde_json::json;
    let mut app = App::test_app();
    app.view
        .apply(&json!({"seq":1,"kind":"team_selected","payload":{
            "team":"small-change","reason":"User selection", "origin":"user"
        }}));
    app.sync_rigor_recommendation();
    assert!(app.picker.is_none());
}

#[test]
fn recommendation_enter_sends_confirmation_or_an_alternative_rigor() {
    use super::super::{App, InputBuffer};
    use crate::lean::{
        daemon::{Client, socket},
        tasks::Team,
    };
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use serde_json::json;

    for alternative in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let listener = std::os::unix::net::UnixListener::bind(socket(directory.path())).unwrap();
        let (requests, received) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream);
            let request: serde_json::Value = crate::transport::read(&mut reader).unwrap().unwrap();
            requests.send(request).unwrap();
            crate::transport::write(reader.get_mut(), &json!({"data":{},"error":null})).unwrap();
        });
        let mut app = App::test_app();
        app.client = Client::at(directory.path());
        app.input = InputBuffer::with_text("Preserved draft 👩‍💻");
        app.view.team = Some(Team::SmallChange);
        app.view.team_recommended = true;
        app.sync_rigor_recommendation();
        assert!(app.picker.is_none());
        app.run_workflow(Command::OpenRigorPicker);
        if alternative {
            app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
        }
        app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let request = received
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let request = &request["request"];
        assert_eq!(request["op"], "select_team");
        assert_eq!(
            request["team"],
            if alternative {
                "feature"
            } else {
                "small-change"
            }
        );
        assert_eq!(request["conversation"], app.conversation);
        assert!(app.picker.is_none());
        assert_eq!(app.input.text, "Preserved draft 👩‍💻");
        server.join().unwrap();
    }
}
