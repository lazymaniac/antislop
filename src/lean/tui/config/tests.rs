use super::model::kind_names;

use super::*;
#[test]
fn kind_names_rejects_an_empty_variant_without_panicking() {
    #[derive(serde::Deserialize)]
    enum EmptyKind {
        #[serde(rename = "")]
        Empty,
    }
    assert_eq!(
        kind_names::<EmptyKind>().unwrap_err(),
        "Cannot list configuration kinds: empty variant names are unsupported."
    );
}

#[test]
fn kind_names_reports_other_deserialization_errors() {
    let error = kind_names::<u64>().unwrap_err();
    assert!(error.starts_with("Cannot list configuration kinds: "));
    assert!(error.contains("invalid type"), "{error}");
}

#[test]
fn all_fields_round_trip_and_invalid_values_stay_in_draft() {
    let source = include_str!("../../../../examples/antislop.toml");
    let mut editor = ConfigEditor::new(source, "antislop.toml".into()).unwrap();
    assert_eq!(
        Config::from_toml(&editor.source().unwrap()).unwrap(),
        Config::from_toml(source).unwrap()
    );
    let index = editor
        .fields
        .iter()
        .position(|f| f.path.last().unwrap() == "local_commits")
        .unwrap();
    editor.fields[index].input = InputBuffer::with_text("wrong");
    assert!(editor.source().is_none());
    assert!(editor.fields[index].error.is_some());
    assert_eq!(editor.fields[index].input.text, "wrong");
}
#[test]
fn optional_overrides_commands_and_field_errors_round_trip() {
    let mut editor = ConfigEditor::new(
        include_str!("../../../../examples/antislop.toml"),
        "antislop.toml".into(),
    )
    .unwrap();
    let binary = editor
        .fields
        .iter()
        .position(|f| f.path == ["providers", "codex", "binary"])
        .unwrap();
    let command = editor
        .fields
        .iter()
        .position(|f| f.path == ["providers", "codex", "command"])
        .unwrap();
    editor.fields[binary].input.clear();
    editor.fields[command].input = InputBuffer::with_text(r#"["wrapper", "path with spaces"]"#);
    let source = editor.source().unwrap();
    let parsed = Config::from_toml(&source).unwrap();
    assert_eq!(
        parsed.providers[&crate::config::ProviderKind::Codex]
            .command
            .as_ref()
            .unwrap()
            .0,
        ["wrapper", "path with spaces"]
    );
    editor.fields[command].input = InputBuffer::with_text("[]");
    assert!(editor.source().is_none());
    assert!(editor.fields[command].error.is_some());
    let route = editor
        .fields
        .iter()
        .position(|f| f.path == ["orchestrator", "profile"])
        .unwrap();
    editor.fields[command].input = InputBuffer::with_text(r#"["wrapper"]"#);
    editor.fields[route].input = InputBuffer::with_text("unknown-profile");
    assert!(editor.source().is_none());
    assert!(editor.fields[route].error.is_some());
}

#[test]
fn sections_navigate_without_changing_the_configuration_draft() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let source = include_str!("../../../../examples/antislop.toml");
    let mut editor = ConfigEditor::new(source, "project.toml".into()).unwrap();
    let initial = editor.selected;
    let section = editor.fields[initial].path[0].clone();
    editor.key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL));
    assert_ne!(editor.fields[editor.selected].path[0], section);
    assert_eq!(
        Config::from_toml(&editor.source().unwrap()).unwrap(),
        Config::from_toml(source).unwrap()
    );
}

#[test]
fn d3_configuration_screen_shows_sections_fields_errors_and_actions() {
    use ratatui::{Terminal, backend::TestBackend};
    let mut editor = ConfigEditor::new(
        include_str!("../../../../examples/antislop.toml"),
        "project.toml".into(),
    )
    .unwrap();
    editor.fields[editor.selected].error = Some("invalid value".into());
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| editor.draw_with_theme(frame, &Theme::from_env(), None))
        .unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|cell| cell.symbol())
        .collect();
    assert!(text.contains("Configuration (project): project.toml"));
    assert!(text.contains("providers"));
    assert!(text.contains("profiles"));
    assert!(text.contains("ERROR: invalid value"));
    assert!(text.contains("Ctrl+S save/apply"));
    assert!(text.contains("Ctrl+R refresh"));
}

#[test]
fn selected_long_field_keeps_its_cursor_segment_visible() {
    use ratatui::{Terminal, backend::TestBackend};
    let mut editor = ConfigEditor::new(
        include_str!("../../../../examples/antislop.toml"),
        "project.toml".into(),
    )
    .unwrap();
    let value = "x".repeat(200);
    editor.fields[editor.selected].input = InputBuffer::with_text(&value);
    let mut terminal = Terminal::new(TestBackend::new(100, 16)).unwrap();
    terminal
        .draw(|frame| editor.draw_with_theme(frame, &Theme::from_env(), None))
        .unwrap();
    let (x, y) = terminal.get_cursor_position().unwrap().into();
    assert!(x < 100 && y < 16);
    let row: String = (0..100)
        .map(|column| terminal.backend().buffer()[(column, y)].symbol())
        .collect();
    assert!(row.contains("xxx"));
}
#[test]
fn document_patching_preserves_comments_and_reports_invalid_boolean_at_the_field() {
    let source = include_str!("../../../../examples/antislop.toml").replace(
        "local_commits = true",
        "local_commits = true # keep this comment",
    );
    let mut editor = ConfigEditor::new(&source, "antislop.toml".into()).unwrap();
    let index = editor
        .fields
        .iter()
        .position(|f| f.path == ["policy", "local_commits"])
        .unwrap();
    editor.fields[index].input = InputBuffer::with_text("false");
    assert!(
        editor
            .source()
            .unwrap()
            .contains("local_commits = false # keep this comment")
    );
    editor.fields[index].input = InputBuffer::with_text("18446744073709551615");
    assert!(editor.source().is_none());
    assert!(editor.fields[index].error.is_some());
}
