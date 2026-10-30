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
        .position(|f| f.path.last().unwrap() == "max_parallel")
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
fn document_patching_preserves_comments_and_reports_integer_overflow_at_the_field() {
    let source = include_str!("../../../../examples/antislop.toml")
        .replace("max_parallel = 4", "max_parallel = 4 # keep this comment");
    let mut editor = ConfigEditor::new(&source, "antislop.toml".into()).unwrap();
    let index = editor
        .fields
        .iter()
        .position(|f| f.path == ["policy", "max_parallel"])
        .unwrap();
    editor.fields[index].input = InputBuffer::with_text("2");
    assert!(
        editor
            .source()
            .unwrap()
            .contains("max_parallel = 2 # keep this comment")
    );
    editor.fields[index].input = InputBuffer::with_text("18446744073709551615");
    assert!(editor.source().is_none());
    assert!(editor.fields[index].error.is_some());
}
