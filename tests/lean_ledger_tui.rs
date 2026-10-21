use antislop::lean::{
    ledger::*,
    tui::{
        App, InputBuffer,
        ledger::{Action, LedgerView},
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

fn browser() -> LedgerView {
    let spec:EntrySpec=serde_json::from_value(json!({"id":"r1","kind":"requirement","source":{"source_id":"s1","start":0,"end":4},"interpretation":"Keep the source","criteria":[{"id":"c1","text":"Check actual behavior"}]})).unwrap();
    let mut view = LedgerView::new(LedgerKind::Requirement);
    view.page = Some(LedgerPage {
        run: Run {
            id: "run1".into(),
            conversation_id: "c1".into(),
            goal: "goal".into(),
            spec_revision: 2,
            state: "active".into(),
            start_message: 0,
        },
        entries: vec![LedgerEntry {
            run_id: "run1".into(),
            version: 1,
            spec_revision: 1,
            original: "text".into(),
            spec,
            status: "identified".into(),
        }],
        total: 1,
        offset: 0,
        has_more: false,
    });
    view
}
fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}
#[test]
fn unicode_editor_preserves_clusters_and_network_conflict_drafts() {
    let mut view = browser();
    view.key(key(KeyCode::Char('e')), "c1");
    view.form_mut().unwrap().fields[1] = InputBuffer::with_text("a👩‍💻e\u{301}");
    view.key(key(KeyCode::Backspace), "c1");
    assert_eq!(view.form_mut().unwrap().fields[1].text, "a👩‍💻");
    view.key(key(KeyCode::Backspace), "c1");
    assert_eq!(view.form_mut().unwrap().fields[1].text, "a");
    view.paste("👨‍👩‍👧‍👦ż\u{301}");
    let saved = view.form_mut().unwrap().clone();
    view.reply("preview", Err("spec revision conflict".into()));
    assert_eq!(*view.form_mut().unwrap(), saved);
    view.key(key(KeyCode::Esc), "c1");
    view.key(key(KeyCode::Char('e')), "c1");
    assert_eq!(*view.form_mut().unwrap(), saved);
    assert!(matches!(
        view.key(
            KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
            "c1"
        ),
        Action::Prepare { .. }
    ));
}
#[test]
fn fields_support_mouse_selection_and_small_viewports() {
    let mut view = browser();
    view.key(key(KeyCode::Char('e')), "c1");
    for (width, height) in [(1, 1), (11, 4), (12, 5), (24, 8), (80, 24)] {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| view.draw(f, f.area())).unwrap();
    }
    view.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 2,
        row: 6,
        modifiers: KeyModifiers::NONE,
    });
    assert_eq!(view.form_mut().unwrap().field, 2);
    view.paste("all qualifiers\nexcept offline");
    assert_eq!(
        view.form_mut().unwrap().fields[2].text,
        "all qualifiers\nexcept offline"
    );
    view.mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: 28,
        row: 4,
        modifiers: KeyModifiers::NONE,
    });
    let input = &view.form_mut().unwrap().fields[2];
    assert_eq!(input.cursor, 2);
}
#[test]
fn conversation_switches_keep_ledger_cursor_filter_and_background_chat() {
    let mut app = App::test_app();
    app.conversation = "c1".into();
    app.input = InputBuffer::with_text("background draft");
    let mut view = browser();
    view.query = InputBuffer::with_text("r1");
    view.key(key(KeyCode::Char('e')), "c1");
    view.paste(" 👩‍💻");
    let saved = view.clone();
    app.ledger_view = Some(view);
    app.open("c2".into());
    assert!(app.ledger_view.is_none());
    app.open("c1".into());
    assert_eq!(app.ledger_view.as_ref(), Some(&saved));
    assert_eq!(app.input.text, "background draft");
}
#[test]
fn preview_is_required_and_has_no_manual_verified_field() {
    let mut view = browser();
    view.key(key(KeyCode::Char('e')), "c1");
    let form = view.form_mut().unwrap();
    form.fields[13] = InputBuffer::with_text("user corrected interpretation");
    let Action::Prepare { change, .. } = view.key(
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
        "c1",
    ) else {
        panic!("expected preview");
    };
    assert_eq!(change.expected_spec_revision, 2);
    assert!(
        !serde_json::to_value(&change.entries[0])
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("status")
    );
    view.reply("preview",Ok(json!({"change":change,"proposal":{"proposal_id":"p1","diff_hash":"exact-diff","diff":{"affected_tasks":["task-a"]}}})));
    assert!(
        matches!(view.key(key(KeyCode::Enter),"c1"),Action::Apply{hash,..} if hash=="exact-diff")
    );
    view.reply("applied", Err("connection lost".into()));
    assert!(view.preview.is_some());
    assert!(view.form_mut().is_some());
}

#[test]
fn stale_reads_are_invalidated_when_switching_ledger_or_record() {
    let mut view = browser();
    view.pending = true;
    view.pending_kind = "page".into();
    view.request_id = 10;
    assert!(matches!(view.key(key(KeyCode::Tab), "c1"), Action::Fetch));
    assert!(!view.pending);
    assert!(view.request_id > 10);
    assert_eq!(view.kind, LedgerKind::PlanItem);
    view.pending = true;
    view.pending_kind = "detail".into();
    let old = view.request_id;
    view.key(key(KeyCode::Down), "c1");
    assert!(!view.pending);
    assert!(view.request_id > old);
}
#[test]
fn filters_are_retained_separately_for_each_run() {
    let mut view = browser();
    let first = view.page.as_ref().unwrap().run.clone();
    let mut second = first.clone();
    second.id = "run2".into();
    view.query = InputBuffer::with_text("first run filter");
    view.reply("runs", Ok(json!([first, second])));
    assert_eq!(view.page.as_ref().unwrap().run.id, "run2");
    assert!(view.query.is_empty());
    view.query = InputBuffer::with_text("second run filter");
    view.reply("runs", Ok(json!([first, second])));
    assert_eq!(view.page.as_ref().unwrap().run.id, "run1");
    assert_eq!(view.query.text, "first run filter");
    view.reply("runs", Ok(json!([first, second])));
    assert_eq!(view.query.text, "second run filter");
}
#[test]
fn successful_save_preserves_text_typed_while_the_mutation_was_in_flight() {
    let mut view = browser();
    view.key(key(KeyCode::Char('e')), "c1");
    let origin = view.editing.clone().unwrap();
    let snapshot = view.drafts[&origin].clone();
    let change = LedgerChange {
        conversation: "c1".into(),
        run_id: "run1".into(),
        expected_spec_revision: 2,
        operation_id: snapshot.operation_id.clone(),
        reason: "user decision".into(),
        entries: vec![snapshot.spec().unwrap()],
    };
    view.prepared_draft = Some((origin.clone(), snapshot.clone()));
    view.proposed_change = Some(change);
    view.pending = true;
    view.pending_kind = "applied".into();
    view.paste(" new text during save");
    view.reply("applied", Ok(json!({"spec_revision":3})));
    let draft = view.drafts.get(&origin).unwrap();
    assert!(draft.fields[1].text.ends_with("new text during save"));
    assert_eq!(draft.revision, 3);
    assert_ne!(draft.operation_id, snapshot.operation_id);
}
