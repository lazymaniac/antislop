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
