use antislop::lean::{
    ledger::*,
    tui::{
        App, InputBuffer,
        ledger::{Action, FormField, LedgerView},
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;

fn browser() -> LedgerView {
    let spec:EntrySpec=serde_json::from_value(json!({"id":"r1","source":{"source_id":"s1","start":0,"end":4},"interpretation":"Keep the source","criteria":[{"id":"c1","text":"Check actual behavior"}]})).unwrap();
    let mut view = LedgerView::new();
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
    assert_eq!(view.form_mut().unwrap().field, FormField::Qualifiers);
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
    form.fields[FormField::Reason as usize] =
        InputBuffer::with_text("user corrected interpretation");
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
fn stale_reads_are_invalidated_when_switching_record() {
    let mut view = browser();
    assert!(matches!(view.key(key(KeyCode::Tab), "c1"), Action::None));
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

fn screen(view: &mut LedgerView, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| view.draw(frame, frame.area()))
        .unwrap();
    terminal
        .backend()
        .buffer()
        .content()
        .chunks(width as usize)
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn requirements_workspace_separates_records_details_and_preserves_small_terminal_bounds() {
    let mut view = browser();
    let wide = screen(&mut view, 120, 30);
    assert!(wide.contains("Records") && wide.contains("Details"));
    assert!(wide.contains("Constraints & qualifiers") && wide.contains("Original wording"));
    assert!(view.list_area.right() <= view.detail_area.x);
    for size in [(12, 5), (24, 8), (60, 18), (80, 24), (120, 30)] {
        screen(&mut view, size.0, size.1);
        assert!(view.list_area.right() <= size.0 && view.detail_area.bottom() <= size.1);
    }
    let mut second = view.page.as_ref().unwrap().entries[0].clone();
    second.spec.id = "r2".into();
    second.spec.interpretation = "Second requirement".into();
    view.page.as_mut().unwrap().entries.push(second);
    screen(&mut view, 120, 30);
    assert!(matches!(
        view.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: view.list_area.x + 1,
            row: view.list_area.y + 3,
            modifiers: KeyModifiers::NONE
        }),
        Action::Detail
    ));
    assert_eq!(view.selected_entry().unwrap().spec.id, "r2");
}

#[test]
fn semantic_review_shows_before_after_and_keeps_failed_approval_actionable() {
    let mut view = browser();
    let before = view.page.as_ref().unwrap().entries[0].clone();
    let mut after = before.spec.clone();
    after.interpretation = "Preserve every qualifier".into();
    let change = LedgerChange {
        conversation: "c1".into(),
        run_id: "run1".into(),
        expected_spec_revision: 2,
        operation_id: "op1".into(),
        reason: "Clarify the scope".into(),
        entries: vec![after.clone()],
    };
    view.reply("preview",Ok(json!({"change":change,"proposal":{"proposal_id":"p1","diff_hash":"h1","diff":{"before":[before],"after":[after],"affected_tasks":[]}}})));
    let rendered = screen(&mut view, 120, 30);
    assert!(rendered.contains("Before: Keep the source"));
    assert!(rendered.contains("After: Preserve every qualifier"));
    assert!(rendered.contains("Approve changes & continue planning"));
    assert!(!rendered.contains("\"interpretation\""));
    view.reply("applied", Err("connection lost".into()));
    assert!(screen(&mut view, 120, 30).contains("connection lost"));
    assert!(matches!(
        view.key(key(KeyCode::Enter), "c1"),
        Action::Apply { .. }
    ));
    for size in [(12, 5), (24, 8), (80, 24)] {
        screen(&mut view, size.0, size.1);
    }
}

#[test]
fn creating_an_empty_draft_requires_confirmation_and_keeps_the_run_visible() {
    let mut view = browser();
    assert!(matches!(
        view.key(key(KeyCode::Char('N')), "c1"),
        Action::None
    ));
    assert!(screen(&mut view, 80, 24).contains("does not replace"));
    view.key(key(KeyCode::Esc), "c1");
    assert_eq!(view.page.as_ref().unwrap().run.id, "run1");
    view.key(key(KeyCode::Char('N')), "c1");
    assert!(matches!(
        view.key(key(KeyCode::Enter), "c1"),
        Action::Create
    ));
    let mut run = view.page.as_ref().unwrap().run.clone();
    run.id = "draft2".into();
    view.reply("created", Ok(json!(run)));
    assert_eq!(view.page.as_ref().unwrap().run.id, "draft2");
    assert!(
        view.notice
            .as_ref()
            .unwrap()
            .contains("Current planning keeps")
    );
}

#[test]
fn mouse_tabs_and_review_action_match_the_rendered_controls() {
    let mut view = browser();
    screen(&mut view, 120, 30);
    assert!(matches!(
        view.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 20,
            row: 1,
            modifiers: KeyModifiers::NONE
        }),
        Action::None
    ));
    assert!(matches!(
        view.mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 8,
            row: 28,
            modifiers: KeyModifiers::NONE
        }),
        Action::Proposal
    ));
}

#[test]
fn progress_and_failure_remain_visible_while_requirements_are_open() {
    let mut app = App::test_app();
    app.ledger_view = Some(browser());
    app.input = InputBuffer::with_text("unsent background draft");
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    for (seq, kind, payload, expected) in [
        (
            1,
            "planning_progress",
            json!({"text":"Checking requirements independently"}),
            "Checking requirements independently",
        ),
        (
            2,
            "requirements_failed",
            json!({"error":"Source changed","retry":"Review the proposed changes"}),
            "Source changed",
        ),
        (
            3,
            "plan_proposed",
            json!({"plan":"plan1","tasks":[]}),
            "Plan ready for review",
        ),
    ] {
        app.view
            .apply(&json!({"seq":seq,"kind":kind,"payload":payload}));
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let text = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(text.contains(expected), "{text}");
        assert_eq!(app.input.text, "unsent background draft");
    }
}

#[test]
fn simplified_form_maps_named_requirement_fields() {
    let mut view = browser();
    view.key(key(KeyCode::Char('e')), "c1");
    let form = view.form_mut().unwrap();
    for (field, text) in [
        (FormField::Interpretation, "Preserve the required method"),
        (FormField::Qualifiers, "except offline"),
        (
            FormField::Criteria,
            "accept | required | integration | independently check",
        ),
        (FormField::Parent, "parent"),
        (FormField::Dependencies, "dependency"),
        (FormField::ImplementationOwners, "worker"),
        (FormField::VerificationOwners, "verifier"),
        (FormField::Tasks, "task"),
        (FormField::Disposition, "superseded_by_user"),
        (FormField::Reason, "user correction"),
        (FormField::Source, "new source"),
        (FormField::Mandatory, "no"),
        (FormField::Position, "7"),
    ] {
        form.fields[field as usize] = InputBuffer::with_text(text);
    }
    let spec = form.spec().unwrap();
    assert_eq!(spec.id, "r1");
    assert_eq!(spec.interpretation, "Preserve the required method");
    assert_eq!(spec.qualifiers, ["except offline"]);
    assert_eq!(spec.criteria[0].id, "accept");
    assert!(spec.criteria[0].mandatory && spec.criteria[0].integration);
    assert_eq!(spec.parent.as_deref(), Some("parent"));
    assert_eq!(spec.depends_on, ["dependency"]);
    assert_eq!(spec.implementation_owners, ["worker"]);
    assert_eq!(spec.verification_owners, ["verifier"]);
    assert_eq!(spec.task_ids, ["task"]);
    assert_eq!(spec.disposition, Disposition::SupersededByUser);
    assert!(!spec.mandatory);
    assert_eq!(spec.position, 7);
    let Action::Prepare { change, source } = view.key(
        KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL),
        "c1",
    ) else {
        panic!("expected preview")
    };
    assert_eq!(change.reason, "user correction");
    assert_eq!(source, "new source");
}
