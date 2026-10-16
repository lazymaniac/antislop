use architect::tui::{
    app::{App, Effect, Focus, View},
    backend::Action,
    editor::Editor,
    model::{Agent, Message, Snapshot},
    render,
    sanitize::sanitize,
};
use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::{Terminal, backend::TestBackend};
use serde_json::json;
fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
}
fn control(c: char) -> Event {
    Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL))
}
fn agent(id: &str, parent: Option<&str>) -> Agent {
    Agent {
        id: id.into(),
        parent: parent.map(str::to_owned),
        role: format!("role-{id}"),
        status: "queued".into(),
        ..Agent::default()
    }
}
fn app() -> App {
    let mut app = App::default();
    app.update(Snapshot {
        run_id: "R1".into(),
        agents: vec![
            agent("parent", None),
            agent("child", Some("parent")),
            agent("other", None),
        ],
        ..Snapshot::default()
    });
    app
}
fn screen(app: &mut App, w: u16, h: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
    terminal.draw(|f| render::draw(f, app)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..h)
        .map(|y| (0..w).map(|x| buffer[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}
#[test]
fn native_step_rejections_show_pending_work_and_never_a_completed_review() {
    let mut document = json!({"id":"R1","status":"running","session_ledger":{
        "sessions":{"S":{"id":"S","state":"sent","current_turn_id":"T","scope":{"agent_id":"A","provider":"agy","role":"verifier"}}},
        "turns":{"T":{"id":"T","session_id":"S","provider_completed":false,"native_rejections":[{"step_index":4,"reason":"malformed_function_call"},{"step_index":6,"reason":"unclassified_native_step"}]}},
        "turn_order":["T"]
    }});
    let mut app = App::default();
    app.update(Snapshot::from_document(&document, &json!({})));
    assert_eq!(app.snapshot.native_rejections, vec![("A".into(), 2)]);
    let rendered = screen(&mut app, 180, 45);
    assert!(
        rendered.contains("A: rejected native steps: 2"),
        "{rendered}"
    );
    assert!(rendered.contains("Same session; awaiting a valid native result"));
    document["session_ledger"]["turns"]["T"]["provider_completed"] = json!(true);
    app.update(Snapshot::from_document(&document, &json!({})));
    assert!(app.snapshot.native_rejections.is_empty());
    assert!(!screen(&mut app, 180, 45).contains("rejected native steps"));
    document["session_ledger"]["turns"]["T"]["provider_completed"] = json!(false);
    document["session_ledger"]["sessions"]["S"]["state"] = json!("failed");
    app.update(Snapshot::from_document(&document, &json!({})));
    assert!(app.snapshot.native_rejections.is_empty());
}

#[test]
fn grapheme_editor_handles_combining_emoji_wide_lines_and_boundaries() {
    let mut editor = Editor::default();
    editor.set("a\u{301}界👩‍💻\nxy".into());
    editor.key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
    assert_eq!(editor.position(), (1, 0));
    editor.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(editor.cursor(), 0);
    editor.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    assert_eq!(&editor.text[..editor.cursor()], "a\u{301}");
    editor.key(KeyEvent::new(KeyCode::Delete, KeyModifiers::NONE));
    assert_eq!(editor.text, "a\u{301}👩‍💻\nxy");
    editor.key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
    editor.key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
    assert_eq!(editor.text, "a\u{301}\nxy");
    assert_eq!(editor.position(), (0, 1));
    let (viewport, x, y) = editor.viewport(1, 1);
    assert_eq!(x, 0);
    assert_eq!(y, 0);
    assert!(viewport.is_empty());
    editor.insert("\nhello");
    assert!(editor.text.is_char_boundary(editor.cursor()));
}

#[test]
fn editor_wraps_words_and_preserves_the_message() {
    let mut editor = Editor::default();
    let text = "hello world again\n\nlast line";
    editor.set(text.into());
    assert_eq!(
        editor.viewport(12, 6),
        ("hello world \nagain\n\nlast line".into(), 9, 3)
    );
    assert_eq!(editor.text, text);
    assert_eq!(editor.viewport(20, 6).0, text);
    editor.set("hello world".into());
    assert_eq!(editor.viewport(5, 4), ("hello\nworld\n".into(), 0, 2));
}

#[test]
fn editor_wraps_long_words_and_unicode_and_scrolls_to_the_cursor() {
    let mut editor = Editor::default();
    editor.set("abcdefghijk".into());
    assert_eq!(editor.viewport(4, 2), ("efgh\nijk".into(), 3, 1));
    editor.set("a\u{301}界👩‍💻z".into());
    assert_eq!(editor.viewport(3, 3), ("a\u{301}界\n👩‍💻z\n".into(), 0, 2));
    assert_eq!(editor.viewport(0, 3), (String::new(), 0, 0));
    assert_eq!(editor.viewport(3, 0), (String::new(), 0, 0));
}

#[test]
fn editor_arrows_move_between_wrapped_rows() {
    let mut editor = Editor::default();
    editor.set("hello world again".into());
    editor.set_wrap_width(12);
    editor.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
    assert_eq!(&editor.text[..editor.cursor()], "hello");
    assert_eq!(editor.viewport(12, 3).1, 5);
    editor.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    assert_eq!(editor.cursor(), editor.text.len());
    editor.set_wrap_width(20);
    assert!(editor.on_first_row());
    assert!(editor.on_last_row());
}

#[test]
fn chat_arrows_navigate_wrapped_draft_before_recalling_history() {
    let mut app = app();
    app.handle(key(KeyCode::Char('i')));
    app.history.push("older message".into());
    app.editor.set("hello world again".into());
    app.editor.set_wrap_width(12);
    app.handle(key(KeyCode::Up));
    assert_eq!(app.editor.text, "hello world again");
    app.handle(key(KeyCode::Up));
    assert_eq!(app.editor.text, "older message");
    app.handle(key(KeyCode::Up));
    app.handle(key(KeyCode::Down));
    assert_eq!(app.editor.text, "older message");
    app.handle(key(KeyCode::Down));
    assert_eq!(app.editor.text, "hello world again");
}

#[test]
fn chat_render_wraps_the_draft_and_keeps_cursor_after_visible_text() {
    let mut app = app();
    app.handle(key(KeyCode::Char('i')));
    let draft = format!("{}end", "1234567890".repeat(5));
    app.editor.set(draft.clone());
    let mut terminal = Terminal::new(TestBackend::new(40, 24)).unwrap();
    terminal.draw(|f| render::draw(f, &mut app)).unwrap();
    let cursor = terminal.get_cursor_position().unwrap();
    let buffer = terminal.backend().buffer();
    let before_cursor = (0..cursor.x)
        .map(|x| buffer[(x, cursor.y)].symbol())
        .collect::<String>();
    assert!(before_cursor.ends_with("end"), "{before_cursor:?}");
    assert!(!app.editor.on_first_row());
    app.handle(key(KeyCode::Up));
    assert_eq!(app.editor.text, draft);
    assert!(app.editor.on_first_row());
}

#[test]
fn graph_arrows_follow_spatial_neighbors_and_filter_keeps_matching_nodes() {
    let mut app = app();
    app.focus = Focus::Team;
    app.handle(key(KeyCode::Right));
    assert_eq!(app.selected_agent_id.as_deref(), Some("child"));
    app.handle(key(KeyCode::Left));
    assert_eq!(app.selected_agent_id.as_deref(), Some("parent"));
    app.handle(key(KeyCode::Up));
    assert_eq!(app.selected_agent_id.as_deref(), Some("other"));
    app.handle(key(KeyCode::Down));
    assert_eq!(app.selected_agent_id.as_deref(), Some("parent"));
    app.team_filter = "child".into();
    assert_eq!(app.nodes().len(), 1);
    assert_eq!(app.nodes()[0].id, "child");
}

#[test]
fn selection_is_identity_stable_across_reordered_events_and_new_agents() {
    let mut app = app();
    app.selected_agent_id = Some("child".into());
    let mut snapshot = app.snapshot.clone();
    snapshot.agents.reverse();
    snapshot.agents.insert(0, agent("aaa", None));
    app.update(snapshot);
    assert_eq!(app.selected_agent_id.as_deref(), Some("child"));
    app.handle(key(KeyCode::Tab));
    app.handle(key(KeyCode::Enter));
    assert_eq!(app.view, View::Agent { tab: 0 });
    app.handle(key(KeyCode::Esc));
    assert_eq!(app.view, View::Main);
}
#[test]
fn chat_editor_recalls_sent_messages_with_up_and_down() {
    let mut app = app();
    app.handle(key(KeyCode::Char('i')));
    let send = |app: &mut App, text: &str| {
        app.handle(Event::Paste(text.into()));
        app.handle(control('s'));
        app.action_result(
            &Action::SendMessage {
                target_agent_id: "orchestrator".into(),
                text: text.into(),
            },
            Ok("accepted".into()),
        );
    };
    send(&mut app, "first");
    send(&mut app, "second\nline");
    send(&mut app, "second\nline");
    assert_eq!(app.history, ["first", "second\nline"]);
    app.handle(Event::Paste("draft".into()));
    app.handle(key(KeyCode::Up));
    assert_eq!(app.editor.text, "second\nline");
    // Up inside a multi-line entry moves the cursor before recalling older.
    app.handle(key(KeyCode::Up));
    assert_eq!(app.editor.text, "second\nline");
    app.handle(key(KeyCode::Up));
    assert_eq!(app.editor.text, "first");
    app.handle(key(KeyCode::Up));
    assert_eq!(app.editor.text, "first");
    app.handle(key(KeyCode::Down));
    assert_eq!(app.editor.text, "second\nline");
    app.handle(key(KeyCode::Down));
    assert_eq!(app.editor.text, "draft");
    app.handle(key(KeyCode::Down));
    assert_eq!(app.editor.text, "draft");
}
#[test]
fn editor_never_detaches_on_q_and_enter_is_multiline_send_is_explicit() {
    let mut app = app();
    app.handle(key(KeyCode::Char('i')));
    assert!(app.editing);
    assert!(app.handle(key(KeyCode::Char('q'))).is_empty());
    app.handle(key(KeyCode::Enter));
    app.handle(Event::Paste("Hi 界\u{1b}[31m!".into()));
    assert_eq!(app.editor.text, "q\nHi 界!");
    let sent = app.handle(control('s'));
    assert_eq!(
        sent,
        vec![Effect::Action(Action::SendMessage {
            target_agent_id: "orchestrator".into(),
            text: "q\nHi 界!".into()
        })]
    );
    assert!(app.handle(control('s')).is_empty());
    app.handle(key(KeyCode::Char('x')));
    app.action_result(
        &Action::SendMessage {
            target_agent_id: "orchestrator".into(),
            text: "q\nHi 界!".into(),
        },
        Ok("accepted".into()),
    );
    assert!(
        app.editor.text.ends_with('x'),
        "receipt must not discard editing done after send"
    );
    app.handle(key(KeyCode::Esc));
    assert_eq!(app.handle(key(KeyCode::Char('q'))), vec![Effect::Detach]);
}
#[test]
fn approvals_pin_displayed_scope_and_require_confirmation() {
    let mut app = app();
    app.snapshot
        .scope_hashes
        .insert("start".into(), "old-scope".into());
    app.handle(control('p'));
    app.palette_index = 4;
    assert!(app.handle(key(KeyCode::Enter)).is_empty());
    assert!(matches!(app.view, View::Confirm { .. }));
    let mut newer = app.snapshot.clone();
    newer
        .scope_hashes
        .insert("start".into(), "new-scope".into());
    app.update(newer);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        vec![Effect::Action(Action::ApproveStart {
            scope_hash: "old-scope".into()
        })]
    );
}
#[test]
fn views_resize_narrow_toggle_ascii_no_color_and_render_actual_delivery() {
    let mut app = app();
    app.no_color = true;
    app.ascii = true;
    app.snapshot.messages.push(Message {
        id: "M1".into(),
        sender: "user".into(),
        target: "parent".into(),
        text: "Hello 界".into(),
        state: "queued_next_turn".into(),
        ..Message::default()
    });
    let wide = screen(&mut app, 120, 30);
    assert!(wide.contains("Orchestrator"));
    assert!(wide.contains("Team: graph"));
    assert!(wide.contains("queued_next_turn"));
    assert!(!wide.contains("Observed message:"));
    assert!(wide.is_ascii());
    app.handle(Event::Resize(60, 18));
    let narrow = screen(&mut app, 60, 18);
    assert!(narrow.contains("Orchestrator"));
    assert!(!narrow.contains("Team: graph"));
    app.handle(key(KeyCode::Tab));
    assert!(screen(&mut app, 60, 18).contains("Team: graph"));
    app.handle(key(KeyCode::Enter));
    assert!(screen(&mut app, 120, 30).contains("Overview"));
    app.handle(key(KeyCode::Right));
    assert!(screen(&mut app, 120, 30).contains("Session"));
    for (w, h) in [(1, 1), (12, 5), (40, 10), (200, 60)] {
        screen(&mut app, w, h);
    }
}
#[test]
fn malicious_provider_output_cannot_emit_terminal_escape_sequences() {
    let input = "hello\x1b[2J\x1b]52;c;secret\x07world\x1bPpayload\x1b\\!\u{202e}evil\r\0\u{9b}31mred\u{9d}title\u{9c}";
    assert_eq!(sanitize(input), "helloworld!evilred");
    assert_eq!(sanitize("safe\x1b]unfinished"), "safe");
    assert_eq!(sanitize("a\tb\nc"), "a    b\nc");
    let mut app = app();
    app.snapshot.diagnostic = input.into();
    let drawn = screen(&mut app, 120, 30);
    assert!(!drawn.contains('\u{1b}'));
    assert!(!drawn.contains("secret"));
}
#[test]
fn persisted_ledgers_project_receipts_refs_checks_and_session_output() {
    let run = json!({"id":"run","stage":"execution","status":"running","spec":{"workspace":"/project","config":{"config":{"policy":{"mode":"autonomous","revision":9}}},"requirements":[{"id":"R1","original_text":"Original requirement","criteria":{"C1":"case"}}]},"team":{"agents":[{"id":"A","role":"implementer","session_id":"S","profile":{"provider":"codex","model":"test"}}],"tasks":[{"id":"T","owner_id":"A","description":"build","state":"running","stage":"execution"}]},"session_ledger":{"sessions":{"S":{"id":"S","scope":{"agent_id":"A"},"state":"sent"}},"turns":{"T":{"id":"T","session_id":"S","state":"sent","output":"actual output"}}},"communication_ledger":{"messages":{"M":{"id":"M","sequence":1,"sender":{"kind":"user"},"recipient_agent_id":"A","state":"queued_next_turn","content_ref":{"document":"message:M","hash":"h"}}}},"message_bodies":{"M":"real message"},"checklist_version":"v2","reviews":[{"executor_session":"S","artifact_hash":"artifact-hash","checks":[{"check_id":"C1","status":"blocked","evidence_ids":["E1"],"rationale":"need evidence"}]}],"risk_coverage":{"risk":{"executor_role":"implementer"}},"quality_gates":{"gate":{"status":"blocked"}}});
    let snapshot = Snapshot::from_document(
        &run,
        &json!({"scope_hashes":{"start":"scope"},"sequence":42}),
    );
    assert_eq!(snapshot.mode, "autonomous");
    assert_eq!(snapshot.policy_revision, 9);
    assert_eq!(snapshot.messages[0].state, "queued_next_turn");
    assert_eq!(snapshot.messages[0].text, "real message");
    assert_eq!(snapshot.messages[0].sender, "user");
    assert!(snapshot.agents[0].details["Session"].contains("actual output"));
    assert!(snapshot.agents[0].details["Checks"].contains("artifact-hash"));
    assert!(snapshot.run_details["Requirements"].contains("Original requirement"));
    assert!(snapshot.run_details["Quality gates"].contains("blocked"));
}
#[test]
fn large_and_cyclic_teams_render_without_recursion_or_agent_caps() {
    let mut app = app();
    app.snapshot.agents = (0..5000)
        .map(|i| {
            agent(
                &format!("a{i:05}"),
                if i == 0 { None } else { Some("a00000") },
            )
        })
        .collect();
    app.selected_agent_id = Some("a04999".into());
    assert_eq!(app.nodes().len(), 5000);
    let text = screen(&mut app, 120, 25);
    assert!(text.contains("role-a04999"));
    assert!(app.graph.positions["a04999"].1 > 4900);
    app.snapshot.agents = vec![agent("a", Some("b")), agent("b", Some("a"))];
    assert_eq!(app.nodes().len(), 2);
    screen(&mut app, 100, 20);
}

#[test]
fn disappearing_team_approval_keeps_palette_selection_in_bounds_and_pins_confirmation() {
    let mut app = app();
    app.snapshot.reviewed_team_round = Some("round".into());
    app.snapshot
        .scope_hashes
        .insert("accept_team".into(), "old-scope".into());
    app.handle(control('p'));
    app.palette_index = app.palette().len() - 1;
    app.handle(key(KeyCode::Enter));
    let mut snapshot = app.snapshot.clone();
    snapshot.reviewed_team_round = None;
    app.update(snapshot);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        vec![Effect::Action(Action::AcceptTeam {
            round_id: "round".into(),
            scope_hash: "old-scope".into()
        })]
    );
    app.handle(control('p'));
    assert!(app.palette_index < app.palette().len());
    app.handle(key(KeyCode::Enter));
}

#[test]
fn task_verifier_confirmation_keeps_exact_target_and_scope_across_refresh() {
    let mut app = app();
    let control_snapshot = json!({"task_verification_retries":{"task-context":"old-scope"}});
    let snapshot = Snapshot::from_document(&json!({"id":"R1"}), &control_snapshot);
    app.update(snapshot);
    app.handle(control('p'));
    app.palette_index = app
        .palette()
        .iter()
        .position(|(_, action)| {
            matches!(action,
        Some(Action::RetryTaskVerification { task_id, .. }) if task_id == "task-context")
        })
        .unwrap();
    app.handle(key(KeyCode::Enter));
    let mut snapshot = app.snapshot.clone();
    snapshot.task_verification_retries.clear();
    app.update(snapshot);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        vec![Effect::Action(Action::RetryTaskVerification {
            task_id: "task-context".into(),
            scope_hash: "old-scope".into(),
        })]
    );
}

#[test]
fn source_refresh_confirmation_preserves_task_scope_when_readiness_changes() {
    let mut app = app();
    app.snapshot
        .task_source_refreshes
        .insert("task-context".into(), "captured-source-scope".into());
    assert!(matches!(
        app.next_approval().unwrap().0,
        Action::RefreshTaskSources { .. }
    ));
    assert!(app.handle(key(KeyCode::Char('a'))).is_empty());
    let mut newer = app.snapshot.clone();
    newer.task_source_refreshes.clear();
    app.update(newer);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        vec![Effect::Action(Action::RefreshTaskSources {
            task_id: "task-context".into(),
            scope_hash: "captured-source-scope".into()
        })]
    );
}

#[test]
fn progress_overlay_shows_observed_check_instead_of_a_not_started_native_verifier() {
    let document = json!({"id":"R1","status":"running","stage":"execution","task_checks":[{
        "state":"intent","task_id":"task-code","plan":{"id":"test-behavior"}}],
        "team":{"agents":[{"id":"executor","role":"implementer"},{"id":"verifier","role":"implementer"}],
        "tasks":[{"id":"task-code","owner_id":"executor","verifier_id":"verifier","state":"verifying","description":"Current code"}]}});
    let mut app = app();
    app.update(Snapshot::from_document(&document, &json!({})));
    assert_eq!(app.snapshot.agents[1].status, "waiting_agent");
    let display = screen(&mut app, 120, 30);
    assert!(display.contains("check test-behavior"));
    assert!(display.contains("1 active"));
}

#[test]
fn broadcast_composer_sends_the_explicit_runtime_target() {
    let mut app = app();
    app.handle(key(KeyCode::Char('b')));
    assert_eq!(app.message_target, "*");
    assert!(app.editing);
    for ch in "Team report requested".chars() {
        app.handle(key(KeyCode::Char(ch)));
    }
    let effects = app.handle(control('s'));
    assert!(effects.iter().any(
        |effect| matches!(effect,Effect::Action(Action::SendMessage{target_agent_id,text})
        if target_agent_id=="*" && text=="Team report requested")
    ));
}

#[test]
fn graph_is_spatial_and_progress_overlay_uses_actual_task_activity() {
    let mut app = app();
    app.snapshot.stage = "execution".into();
    app.snapshot.status = "running".into();
    app.snapshot.agents[1].status = "running".into();
    app.snapshot.agents[1].task = "Implement import".into();
    app.snapshot.agents[1].group = "execution".into();
    let text = screen(&mut app, 160, 36);
    if std::env::var_os("ARCHITECT_GRAPH_PREVIEW").is_some() {
        println!("{text}");
    }
    assert!(text.contains("Team: graph"));
    assert!(text.contains("Current step | 1 active"));
    assert!(text.contains("Implement import"));
    assert_ne!(
        app.graph.positions["parent"].0,
        app.graph.positions["child"].0
    );
    assert_eq!(
        app.graph.positions["parent"].1,
        app.graph.positions["child"].1
    );
    let positions = app.graph.positions.clone();
    let mut next = app.snapshot.clone();
    next.agents.reverse();
    next.agents.push(agent("new", None));
    app.update(next);
    for (id, position) in positions {
        assert_eq!(app.graph.positions[&id], position);
    }
    app.snapshot.status = "paused".into();
    let paused = screen(&mut app, 160, 36);
    assert!(paused.contains("execution: paused"));
    assert!(paused.contains("Current step | paused"));
    assert!(!paused.contains("1 active"));
}

#[test]
fn comment_composer_binds_element_and_keeps_chat_draft_separate() {
    use architect::comments::{Element, ElementKind, ElementRef};
    let mut app = app();
    app.focus = Focus::Team;
    app.editor.set("Unsent message draft".into());
    let target = ElementRef {
        kind: ElementKind::Task,
        id: "task-id".into(),
        version_hash: "a".repeat(64),
    };
    app.snapshot.comment_targets.push(Element {
        reference: target.clone(),
        label: "Task: task-id".into(),
        agent_ids: ["parent".into()].into(),
        context: json!({"id":"task-id"}),
    });
    app.handle(key(KeyCode::Char('c')));
    assert_eq!(app.view, View::CommentTargets);
    assert!(screen(&mut app, 120, 30).contains("Task: task-id"));
    app.handle(key(KeyCode::Enter));
    app.handle(Event::Paste("Review this task".into()));
    let actions = app.handle(control('s'));
    assert_eq!(
        actions,
        vec![Effect::Action(Action::AddElementComment {
            target: target.clone(),
            text: "Review this task".into()
        })]
    );
    assert_eq!(app.editor.text, "Unsent message draft");
    app.action_result(
        &Action::AddElementComment {
            target,
            text: "Review this task".into(),
        },
        Ok("saved".into()),
    );
    assert_eq!(app.editor.text, "Unsent message draft");
    assert!(app.comment_target.is_none());
}

#[test]
fn progress_tracks_the_actual_orchestration_phase_after_team_installation() {
    let mut document = json!({"id":"run","status":"running","stage":"delivery","spec":{},
        "team":{"agents":[{"id":"orchestrator","role":"orchestrator"},{"id":"orchestrator:verifier","role":"orchestrator"}],"tasks":[]},
        "orchestration_rounds":[{"id":"round","status":"running","executor_session":null,"verifier_session":null}]});
    let snapshot = Snapshot::from_document(&document, &json!({}));
    assert_eq!(snapshot.agents[0].status, "running");
    assert_eq!(snapshot.agents[0].task, "Respond to user comments/context");
    assert_eq!(snapshot.agents[0].group, "orchestration");
    assert_ne!(snapshot.agents[1].status, "verifying");
    document["orchestration_rounds"][0]["executor_session"] = json!("completed-executor");
    let snapshot = Snapshot::from_document(&document, &json!({}));
    assert_ne!(snapshot.agents[0].status, "running");
    assert_eq!(snapshot.agents[1].status, "verifying");
    assert_eq!(snapshot.agents[1].task, "Verify orchestrator reply");
    document["status"] = json!("paused");
    assert_eq!(
        Snapshot::from_document(&document, &json!({})).agents[1].status,
        "paused"
    );
}

#[test]
fn baseline_overlay_does_not_claim_any_wave_executor_is_already_running() {
    let document = json!({"id":"R1","status":"running","stage":"execution","task_checks":[{
        "state":"intent","task_id":"task-code","plan":{"id":"reproduce", "phase":"before_execution"}}],
        "team":{"agents":[{"id":"executor","role":"implementer"},{"id":"sibling","role":"implementer"},{"id":"waiting-verifier","role":"implementer"}],
        "tasks":[{"id":"task-code","owner_id":"executor","state":"running","description":"Current code"},
            {"id":"other-code","owner_id":"sibling","state":"running","description":"Sibling code"},
            {"id":"verify-other","verifier_id":"waiting-verifier","state":"verifying","description":"Pending independent review"}]}});
    let mut app = app();
    app.update(Snapshot::from_document(&document, &json!({})));
    assert!(
        app.snapshot
            .agents
            .iter()
            .all(|a| a.status == "waiting_agent")
    );
    let display = screen(&mut app, 120, 30);
    assert!(display.contains("baseline:reproduce"));
    assert!(display.contains("1 active"));
}

#[test]
fn stage_result_confirmation_pins_result_text_tasks_and_scope_across_refresh() {
    use architect::team::TaskStage;
    let result = json!({"task_id":"context","task_attempt":1,"task_hash":"task-hash","artifact_id":"A1","artifact_hash":"old-artifact-hash","review_hash":"old-review-hash","dependency_hashes":{}});
    let control_snapshot = json!({"team_stage_results":{"planning":{"stage":"planning","task_ids":["context"],"results":[result],"scope_hash":"old-result-scope"}}});
    let document = json!({"id":"R1","stage":"discovery","status":"awaiting_result",
        "team":{"tasks":[{"id":"context","description":"Inspect original sources","owner_id":"author","verifier_id":"reviewer","review":{"criteria":{"criterion":true}}}]},
        "artifacts":[{"id":"A1","hash":"old-artifact-hash","content":json!({"artifact":"Original source observation","edits":[]}).to_string()}]});
    let mut app = app();
    app.update(Snapshot::from_document(&document, &control_snapshot));
    app.handle(control('p'));
    app.palette_index = app
        .palette()
        .iter()
        .position(|(_, action)| matches!(action, Some(Action::AcceptTeamStage { .. })))
        .unwrap();
    assert!(app.handle(key(KeyCode::Enter)).is_empty());
    let displayed = screen(&mut app, 160, 40);
    assert!(displayed.contains("Original source observation"));
    assert!(displayed.contains("Stage: planning"));
    assert!(displayed.contains("old-result-scope"));
    assert!(displayed.contains("old-review-hash"));
    let mut changed = app.snapshot.clone();
    changed.team_stage_results.clear();
    changed.team_stage_result_details.clear();
    changed.diagnostic = "New unrelated status".into();
    app.update(changed);
    assert!(screen(&mut app, 160, 40).contains("Original source observation"));
    app.handle(key(KeyCode::PageDown));
    assert!(app.scroll > 0);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        vec![Effect::Action(Action::AcceptTeamStage {
            stage: TaskStage::Planning,
            task_ids: vec!["context".into()],
            scope_hash: "old-result-scope".into()
        })]
    );
}

#[test]
fn executor_result_restoration_confirmation_keeps_exact_completed_output_and_scope() {
    let mut app = app();
    app.snapshot
        .task_result_restores
        .insert("task-context".into(), "saved-result-scope".into());
    app.snapshot.task_result_restore_details.insert(
        "saved-result-scope".into(),
        "Completed original observation\nIndependent verification is required".into(),
    );
    assert!(matches!(
        app.next_approval().unwrap().0,
        Action::RestoreTaskResult { .. }
    ));
    assert!(app.handle(key(KeyCode::Char('a'))).is_empty());
    assert!(screen(&mut app, 120, 30).contains("Completed original observation"));
    let mut changed = app.snapshot.clone();
    changed.task_result_restores.clear();
    changed.task_result_restore_details.clear();
    app.update(changed);
    assert!(screen(&mut app, 120, 30).contains("Completed original observation"));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        vec![Effect::Action(Action::RestoreTaskResult {
            task_id: "task-context".into(),
            scope_hash: "saved-result-scope".into()
        })]
    );
}

#[test]
fn observed_effect_confirmation_retains_displayed_intent_and_scope_across_refresh() {
    let mut app = app();
    app.snapshot
        .task_effect_reconciliations
        .insert("task-code".into(), "observed-after-scope".into());
    app.snapshot.task_effect_reconciliation_details.insert(
        "observed-after-scope".into(),
        "Original approved file intent\nObserved matching after-state; no repeat write".into(),
    );
    app.handle(control('p'));
    app.palette_index = app
        .palette()
        .iter()
        .position(|(_, action)| matches!(action, Some(Action::ReconcileTaskEffects { .. })))
        .unwrap();
    assert!(app.handle(key(KeyCode::Enter)).is_empty());
    let mut changed = app.snapshot.clone();
    changed.task_effect_reconciliations.clear();
    changed.task_effect_reconciliation_details.clear();
    app.update(changed);
    assert!(screen(&mut app, 120, 30).contains("Original approved file intent"));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        vec![Effect::Action(Action::ReconcileTaskEffects {
            task_id: "task-code".into(),
            scope_hash: "observed-after-scope".into()
        })]
    );
}

#[test]
fn wave_effect_confirmation_pins_entire_selected_cohort_and_displayed_scope() {
    use architect::runtime::team::adjudicate::PendingWave;
    let mut app = app();
    app.snapshot.wave_effect_reconciliations.insert(
        "8".into(),
        PendingWave {
            wave_attempt: 8,
            task_ids: vec!["writer-a".into(), "writer-b".into()],
            scope_hash: "cohort-scope".into(),
        },
    );
    app.snapshot.wave_effect_reconciliation_details.insert(
        "cohort-scope".into(),
        "Observed intent A and B; no repeated write".into(),
    );
    app.handle(control('p'));
    app.palette_index = app
        .palette()
        .iter()
        .position(|(_, action)| matches!(action, Some(Action::ReconcileWaveEffects { .. })))
        .unwrap();
    assert!(app.handle(key(KeyCode::Enter)).is_empty());
    let mut changed = app.snapshot.clone();
    changed.wave_effect_reconciliations.clear();
    changed.wave_effect_reconciliation_details.clear();
    app.update(changed);
    assert!(screen(&mut app, 120, 30).contains("Observed intent A and B"));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        vec![Effect::Action(Action::ReconcileWaveEffects {
            task_ids: vec!["writer-a".into(), "writer-b".into()],
            scope_hash: "cohort-scope".into()
        })]
    );
}

#[test]
fn first_chat_send_starts_intake_once_and_retains_the_prompt_while_pending() {
    let mut app = App::default();
    app.handle(key(KeyCode::Char('i')));
    app.handle(Event::Paste("Zbuduj raport\nz pytaniami o zakres".into()));
    assert_eq!(
        app.handle(control('s')),
        vec![Effect::BeginConversation(
            "Zbuduj raport\nz pytaniami o zakres".into()
        )]
    );
    assert!(app.sending);
    assert!(app.handle(control('s')).is_empty());
    assert_eq!(app.editor.text, "Zbuduj raport\nz pytaniami o zakres");
    assert!(screen(&mut app, 120, 25).contains("Send already pending"));
}

#[test]
fn interview_reply_requests_an_orchestrator_turn_and_empty_send_explains_itself() {
    let mut app = app();
    app.snapshot.intake = true;
    app.handle(key(KeyCode::Char('i')));
    assert!(app.handle(control('s')).is_empty());
    assert!(app.notice.contains("Message is empty"));
    app.handle(Event::Paste("Tylko odczyt".into()));
    let action = Action::ChatMessage {
        text: "Tylko odczyt".into(),
    };
    assert_eq!(
        app.handle(control('s')),
        vec![Effect::Action(action.clone())]
    );
    app.action_result(&action, Err("daemon unavailable".into()));
    assert!(!app.sending);
    assert_eq!(app.editor.text, "Tylko odczyt");
    assert!(app.notice.contains("daemon unavailable"));
}

#[test]
fn optional_spec_import_has_separate_draft_and_preserves_chat() {
    let mut app = App::default();
    app.editor.set("original task".into());
    app.handle(key(KeyCode::Char('n')));
    assert_eq!(app.view, View::CreateRun);
    app.handle(Event::Paste("examples/fake-spec.json".into()));
    assert_eq!(
        app.handle(control('s')),
        vec![Effect::CreateRun("examples/fake-spec.json".into())]
    );
    assert_eq!(app.editor.text, "original task");
    assert!(screen(&mut app, 120, 25).contains("SPEC.json path"));
}

#[test]
fn interview_approval_card_pins_requirements_and_routing_while_new_snapshots_arrive() {
    let mut app = app();
    app.snapshot.intake = true;
    app.snapshot.reviewed_intake_proposal = Some(("interview-round".into(), "old-contract".into()));
    app.snapshot.intake_proposal_details =
        "Original requirement; chosen model; exact check argv".into();
    let chat = screen(&mut app, 120, 30);
    assert!(chat.contains("Reviewed task contract"));
    app.handle(key(KeyCode::Char('a')));
    assert!(matches!(app.view, View::Confirm { .. }));
    let card = screen(&mut app, 120, 30);
    assert!(card.contains("chosen model"));
    let mut newer = app.snapshot.clone();
    newer.reviewed_intake_proposal = None;
    newer.intake_proposal_details = "Changed contract".into();
    app.update(newer);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        vec![Effect::Action(Action::AcceptIntake {
            round_id: "interview-round".into(),
            scope_hash: "old-contract".into(),
        })]
    );
}

#[test]
fn discovery_overlay_reports_the_runtime_probe_without_inventing_native_sessions() {
    let document = json!({"id":"discovery","intake":true,"status":"running","stage":"discovery",
        "orchestration_rounds":[{"id":"round","status":"running","discovery_status":"running"}]});
    let mut app = App::default();
    app.update(Snapshot::from_document(&document, &json!({})));
    let text = screen(&mut app, 120, 30);
    assert!(text.contains("CLI discovery"));
    assert!(text.contains("Discover CLI capabilities"));
    assert!(app.snapshot.agents.is_empty());
    let mut paused = document;
    paused["status"] = json!("paused");
    app.update(Snapshot::from_document(&paused, &json!({})));
    let text = screen(&mut app, 120, 30);
    assert!(text.contains("Current step | paused"));
    assert!(!text.contains("Discover CLI capabilities"));
}

#[test]
fn native_resume_card_freezes_exact_session_checkpoint_and_scope_across_refresh() {
    let mut app = app();
    let scope = "a".repeat(64);
    let native = "10000000-0000-4000-8000-000000000001";
    let package = architect::runtime::team::resume::Pending {
        task_id: "T-resume".into(),
        operation: architect::team::TaskOperation::Verify,
        session_id: "managed-resume".into(),
        previous_turn_id: "interrupted-turn".into(),
        native_session_id: native.into(),
        scope_hash: scope.clone(),
    };

    app.snapshot
        .task_session_resumes
        .insert(package.task_id.clone(), package);
    app.snapshot.task_session_resume_details.insert(
        scope.clone(),
        format!("Exact native session: {native}\nCheckpoint: interrupted-turn"),
    );
    assert!(matches!(
        app.next_approval().unwrap().0,
        Action::ResumeTaskSession { .. }
    ));
    assert!(app.handle(key(KeyCode::Char('a'))).is_empty());
    let View::Confirm {
        action, details, ..
    } = &app.view
    else {
        panic!("missing continuation confirmation")
    };
    assert_eq!(
        action,
        &Action::ResumeTaskSession {
            task_id: "T-resume".into(),
            scope_hash: scope.clone()
        }
    );
    assert!(details.contains(native) && details.contains("interrupted-turn"));
    let mut refreshed = app.snapshot.clone();
    refreshed.task_session_resumes.clear();
    refreshed.task_session_resume_details.clear();
    app.update(refreshed);
    let rendered = screen(&mut app, 140, 35);
    assert!(rendered.contains(native));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        vec![Effect::Action(Action::ResumeTaskSession {
            task_id: "T-resume".into(),
            scope_hash: scope
        })]
    );
}

#[test]
fn replacement_handoff_card_keeps_the_reviewed_payload_and_scope_when_current_package_disappears() {
    use architect::runtime::team::replacement::{HANDOFF_VERSION, Handoff, Pending};
    let mut app = app();
    let scope = "b".repeat(64);
    let handoff = Handoff {
        version: HANDOFF_VERSION.into(),
        task_id: "T-new-session".into(),
        task_attempt: 1,
        spec_revision: 1,
        configuration_hash: "c".repeat(64),
        source_hashes: std::collections::BTreeMap::new(),
        contract: serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap(),
        task_contract: json!({"description":"Preserve all original constraints"}),
        dependencies: vec![],
        knowledge: json!({"unresolved":"retain this finding"}),
        prior_sessions: vec![],
        prior_turns: vec![],
        prior_turn_refs: vec![],
        artifacts: vec![],
        task_checks: vec![],
        task_effects: vec![],
        message_history: json!({}),
    };
    let hash = handoff.hash().unwrap();
    let p = Pending {
        task_id: handoff.task_id.clone(),
        operation: architect::team::TaskOperation::Execute,
        previous_session: None,
        previous_turn: None,
        reason: "Native identity unobserved".into(),
        handoff,
        handoff_hash: hash.clone(),
        scope_hash: scope.clone(),
    };
    p.validate().unwrap();
    let mut forged = p.clone();
    forged.handoff.task_contract = json!({"description":"different scope"});
    assert!(forged.validate().is_err());
    app.snapshot
        .task_session_replacements
        .insert(p.task_id.clone(), p);
    app.snapshot.task_session_replacement_details.insert(scope.clone(),format!("DISTINCT NEW SESSION\nHandoff: {hash}\nPreserve all original constraints\nUnresolved: retain this finding"));
    assert!(matches!(
        app.next_approval().unwrap().0,
        Action::ReplaceTaskSession { .. }
    ));
    assert!(app.handle(key(KeyCode::Char('a'))).is_empty());
    let View::Confirm {
        action, details, ..
    } = &app.view
    else {
        panic!("missing replacement approval card")
    };
    assert_eq!(
        action,
        &Action::ReplaceTaskSession {
            task_id: "T-new-session".into(),
            scope_hash: scope.clone()
        }
    );
    assert!(details.contains(&hash) && details.contains("retain this finding"));
    let mut changed = app.snapshot.clone();
    changed.task_session_replacements.clear();
    changed.task_session_replacement_details.clear();
    app.update(changed);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        vec![Effect::Action(Action::ReplaceTaskSession {
            task_id: "T-new-session".into(),
            scope_hash: scope
        })]
    );
}

#[test]
fn management_recovery_card_freezes_retained_answer_and_scoped_kind_across_refresh() {
    use architect::runtime::orchestration_recovery::{Kind, Pending};
    let mut app = app();
    let scope = "f".repeat(64);
    let package = Pending {
        review_part: None,
        round_id: "round-interview".into(),
        kind: Kind::Replacement,
        role: architect::config::RoleKind::Verifier,
        previous_session: Some("failed-review".into()),
        previous_turn: Some("old-turn".into()),
        native_session_id: Some("10000000-0000-4000-8000-000000000001".into()),
        retained_executor: Some("original-orchestrator".into()),
        retained_output: Some("Preserve the already completed answer".into()),
        retained_review: None,
        handoff: json!({"constraints":"Keep the original interview obligations"}),
        handoff_hash: "a".repeat(64),
        scope_hash: scope.clone(),
    };
    for (kind, fragment) in [
        (Kind::Replacement, "distinct session"),
        (Kind::Restore, "without repeating"),
    ] {
        let mut guide = App::default();
        let mut part = package.clone();
        part.review_part = Some("review-part-2".into());
        part.kind = kind;
        guide
            .snapshot
            .orchestration_recoveries
            .insert("part".into(), part);
        guide.snapshot.orchestration_recovery_preparing = true;
        assert!(guide.next_approval().is_none());
        guide.snapshot.orchestration_recovery_preparing = false;
        let (action, label) = guide
            .next_approval()
            .expect("a retained review part needs guided recovery");
        assert!(label.contains(fragment));
        assert!(matches!(action,Action::RecoverOrchestration{kind:actual,..} if actual==kind));
    }
    app.snapshot.orchestration_recovery_details.insert(scope.clone(),"DISTINCT NEW SESSION for verifier only; preserve the already completed answer and original interview obligations".into());
    app.snapshot
        .orchestration_recoveries
        .insert("recovery".into(), package);
    app.handle(control('p'));
    app.palette_index = app
        .palette()
        .iter()
        .position(|(label, _)| label.starts_with("Recover management"))
        .unwrap();
    app.handle(key(KeyCode::Enter));
    let View::Confirm { details, .. } = &app.view else {
        panic!("missing management recovery card")
    };
    assert!(details.contains("completed answer"));
    let mut changed = app.snapshot.clone();
    changed.orchestration_recoveries.clear();
    changed.orchestration_recovery_details.clear();
    app.update(changed);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        vec![Effect::Action(Action::RecoverOrchestration {
            round_id: "round-interview".into(),
            kind: Kind::Replacement,
            scope_hash: scope
        })]
    );
}

#[test]
fn captured_check_recovery_card_preserves_actual_output_and_scope_after_refresh() {
    use architect::runtime::team::{
        check_recovery::{Outcome, Pending},
        checks::Invocation,
    };
    let mut app = app();
    let scope = "c".repeat(64);
    let intent:Invocation=serde_json::from_value(json!({"id":"check-old","task_id":"task-old","task_attempt":1,"artifact_id":"artifact-old","input_snapshot":null,"artifact_hash":"a".repeat(64),
        "plan":{"id":"actual-check","argv":["tool","check"],"phase":"before_verification","expected_exit_code":0,"criterion_ids":["R1.C1"],"gate_ids":[],"rationale":"Actual planned assertion"},
        "spec_revision":1,"configuration_hash":"b".repeat(64),"source_hashes":{},"authorization_scope":"d".repeat(64),"before_hash":"e".repeat(64),"after_hash":null,"state":"unknown","success":false,"output_hash":null,"evidence_id":null})).unwrap();
    let output = "Actual captured exit_code=1; assertion failed".to_string();
    let p = Pending {
        invocation_id: intent.id.clone(),
        task_id: intent.task_id.clone(),
        outcome: Outcome {
            id: "captured-outcome".into(),
            invocation_id: intent.id.clone(),
            intent_hash: "f".repeat(64),
            original_intent: intent,
            output: output.clone(),
            output_hash: architect::hash(&output),
            process_success: false,
            observed_after: Some(std::collections::BTreeMap::new()),
            baseline_barrier: None,
        },
        scope_hash: scope.clone(),
    };
    app.snapshot
        .task_check_restores
        .insert(p.invocation_id.clone(), p);
    app.snapshot
        .task_check_restore_details
        .insert(scope.clone(), output);
    app.handle(control('p'));
    app.palette_index = app
        .palette()
        .iter()
        .position(|(label, _)| label.starts_with("Restore captured check result"))
        .unwrap();
    app.handle(key(KeyCode::Enter));
    let View::Confirm { details, .. } = &app.view else {
        panic!("missing captured check approval card")
    };
    assert!(details.contains("assertion failed"));
    let mut changed = app.snapshot.clone();
    changed.task_check_restores.clear();
    changed.task_check_restore_details.clear();
    app.update(changed);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        vec![Effect::Action(Action::RestoreTaskCheckResult {
            invocation_id: "check-old".into(),
            scope_hash: scope
        })]
    );
}

#[test]
fn task_carousel_checklist_centers_the_followed_task_and_counts_progress() {
    let tasks: Vec<_> = (0..30)
        .map(|i| {
            json!({"id":format!("T{i:02}"),"owner_id":if i % 2 == 0 {"executor"} else {"verifier"},
            "description":format!("Task number {i}"),"state":if i == 20 {"running"} else if i < 20 {"verified"} else {"queued"},"stage":"execution"})
        })
        .collect();
    let document = json!({"id":"R1","status":"running","stage":"execution","team":{
        "agents":[{"id":"executor","role":"implementer"},{"id":"verifier","role":"reviewer"}],"tasks":tasks}});
    let mut app = app();
    app.update(Snapshot::from_document(&document, &json!({})));
    assert_eq!(app.snapshot.tasks.len(), 30);
    assert_eq!(app.snapshot.tasks[3].id, "T03");
    assert_eq!(app.snapshot.tasks[3].owner, "verifier");
    // Unfocused, the list follows the observed active task.
    assert_eq!(app.followed_task(), 20);
    let text = screen(&mut app, 160, 40);
    // The centered task shows its details; distant tasks are outside the window.
    assert!(text.contains("T20 "));
    assert!(text.contains("21/30"));
    assert!(text.contains("[●] Task number 20"));
    assert!(text.contains("[✓] Task number 19"));
    assert!(text.contains("[ ] Task number 21"));
    // Two previous tasks, the followed one and two next ones.
    assert!(text.contains("[✓] Task number 18"));
    assert!(text.contains("[ ] Task number 22"));
    assert!(!text.contains("Task number 17"));
    assert!(!text.contains("Task number 23"));
    assert!(!text.contains("Task number 2 "));
    assert!(text.contains("✓ 20 done"));
    assert!(text.contains("● 1 in progress"));
    assert!(text.contains("○ 9 to do"));
    // Tab cycles chat -> team -> tasks; focus starts at the followed task.
    app.handle(key(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Team);
    app.handle(key(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Tasks);
    assert_eq!(app.task_index, 20);
    app.handle(key(KeyCode::Down));
    assert_eq!(app.task_index, 21);
    // The graph selection follows the selected task's owner.
    assert_eq!(app.selected_agent_id.as_deref(), Some("verifier"));
    app.handle(key(KeyCode::Home));
    let text = screen(&mut app, 160, 40);
    assert!(text.contains("T00 "));
    assert!(text.contains("[✓] Task number 1"));
    assert!(!text.contains("T20 "));
    assert!(!text.contains("Task number 19"));
    app.handle(key(KeyCode::End));
    let text = screen(&mut app, 160, 40);
    assert!(text.contains("T29 "));
    assert!(text.contains("[ ] Task number 28"));
    app.handle(key(KeyCode::Enter));
    assert_eq!(app.view, View::Agent { tab: 0 });
    assert_eq!(app.selected_agent_id.as_deref(), Some("verifier"));
    app.handle(key(KeyCode::Esc));
    app.handle(Event::Key(KeyEvent::new(
        KeyCode::BackTab,
        KeyModifiers::SHIFT,
    )));
    assert_eq!(app.focus, Focus::Team);
    for (w, h) in [(1, 1), (12, 5), (40, 10), (90, 12), (200, 60)] {
        screen(&mut app, w, h);
    }
}

#[test]
fn empty_task_list_and_tiny_task_panels_render() {
    let mut app = app();
    app.focus = Focus::Tasks;
    assert!(screen(&mut app, 70, 20).contains("No tasks yet"));
    for key_code in [KeyCode::Up, KeyCode::Down, KeyCode::End, KeyCode::Enter] {
        app.handle(key(key_code));
    }
    assert_eq!(app.task_index, 0);
    for (w, h) in [(12, 5), (30, 8), (70, 9)] {
        screen(&mut app, w, h);
    }
}

#[test]
fn interview_chat_guides_the_user_to_answer_and_clears_waiting_after_send() {
    let run = json!({"id":"interview","intake":true,"status":"awaiting_start",
        "orchestration_rounds":[{"id":"question-1","status":"awaiting_user","request_sequence":1,
        "output":json!({"message":"1. Jaki format raportu?\n2. Jakie ograniczenia?","intake_proposal_json":null,"team_plan_json":null}).to_string()}]});
    let mut app = App::default();
    app.update(Snapshot::from_document(&run, &json!({})));
    assert_eq!(
        app.snapshot.awaiting_user_reply.as_deref(),
        Some("question-1")
    );
    let rendered = screen(&mut app, 140, 40);
    assert!(rendered.contains("Czeka na Ciebie"), "{rendered}");
    assert!(
        rendered.contains("Orkiestrator czeka na Twoją"),
        "{rendered}"
    );
    assert!(rendered.contains("Jaki format raportu?"), "{rendered}");
    assert!(app.editing);
    assert_eq!(app.focus, Focus::Chat);
    app.handle(key(KeyCode::Esc));
    assert!(!app.editing);
    assert!(app.handle(key(KeyCode::Enter)).is_empty());
    assert!(app.editing);
    assert_eq!(app.message_target, "orchestrator");
    app.editor.set("Po polsku, Markdown.".into());
    let action = Action::ChatMessage {
        text: app.editor.text.clone(),
    };
    assert_eq!(
        app.handle(control('s')),
        vec![Effect::Action(action.clone())]
    );
    app.action_result(&action, Err("transport failed".into()));
    assert!(app.snapshot.awaiting_user_reply.is_some());
    assert_eq!(app.editor.text, "Po polsku, Markdown.");
    app.action_result(
        &action,
        Ok("Message accepted. Orchestrator turn started.".into()),
    );
    assert!(app.snapshot.awaiting_user_reply.is_none());
    assert!(app.editor.text.is_empty());
    let mut next = run.clone();
    next["status"] = json!("running");
    app.update(Snapshot::from_document(&next, &json!({})));
    assert!(app.snapshot.awaiting_user_reply.is_none());
}

#[test]
fn reviewed_team_approval_waits_for_authenticated_preparation_and_remains_explicit() {
    let run = json!({"id":"reviewed-team","status":"awaiting_start","stage":"discovery","orchestration_rounds":[{"id":"round","status":"reviewed"}]});
    let mut app = App::default();
    let mut control = json!({"reviewed_team_round":"round","team_acceptance_ready":false,"orchestration_recovery_state":"preparing","scope_hashes":{"accept_team":"reviewed-scope"}});
    app.update(Snapshot::from_document(&run, &control));
    assert!(app.snapshot.orchestration_recovery_preparing);
    assert!(app.snapshot.reviewed_team_round.is_none());
    assert!(app.next_approval().is_none());
    assert!(screen(&mut app, 140, 40).contains("Sprawdzanie zapisanych"));
    control["team_acceptance_ready"] = json!(true);
    control["orchestration_recovery_state"] = json!("ready");
    app.update(Snapshot::from_document(&run, &control));
    let (action, _) = app.next_approval().unwrap();
    assert_eq!(
        action,
        Action::AcceptTeam {
            round_id: "round".into(),
            scope_hash: "reviewed-scope".into()
        }
    );
    assert!(app.handle(key(KeyCode::Char('a'))).is_empty());
    assert!(matches!(app.view, View::Confirm { .. }));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        vec![Effect::Action(action)]
    );
}

#[test]
fn recovery_preparation_is_visible_without_claiming_a_native_agent_is_running() {
    let run = json!({"id":"retained-review","status":"blocked","stage":"discovery","orchestration_rounds":[{"id":"round","status":"failed"}]});
    let mut app = App::default();
    app.update(Snapshot::from_document(
        &run,
        &json!({"orchestration_recovery_state":"preparing","orchestration_recovery_count":0}),
    ));
    assert!(app.snapshot.orchestration_recovery_preparing);
    assert!(app.next_approval().is_none());
    assert!(app.snapshot.agents.iter().all(|a| a.status != "running"));
    let rendered = screen(&mut app, 140, 40);
    assert!(rendered.contains("Sprawdzanie zapisanych"), "{rendered}");
    app.update(Snapshot::from_document(
        &run,
        &json!({"orchestration_recovery_state":"unavailable","orchestration_recovery_count":0}),
    ));
    assert!(app.snapshot.orchestration_recovery_unavailable);
    assert!(!app.snapshot.orchestration_recovery_preparing);
    assert!(screen(&mut app, 140, 40).contains("Nie można udostępnić"));
    app.update(Snapshot::from_document(
        &run,
        &json!({"orchestration_recovery_state":"ready","orchestration_recovery_count":0}),
    ));
    assert!(!app.snapshot.orchestration_recovery_preparing);
}

#[test]
fn chat_approval_shortcut_opens_current_verified_stage_results() {
    let mut app = app();
    let document =
        json!({"id":"R1","status":"awaiting_result","stage":"discovery","team":{"tasks":[]}});
    let control = json!({"team_stage_results":{"discovery":{"stage":"discovery","task_ids":["context"],"results":[],"scope_hash":"stage-scope"}}});
    app.update(Snapshot::from_document(&document, &control));
    let rendered = screen(&mut app, 160, 40);
    assert!(
        rendered.contains("results await your acceptance"),
        "{rendered}"
    );
    app.handle(key(KeyCode::Char('a')));
    assert!(matches!(
        app.view,
        View::Confirm {
            action: Action::AcceptTeamStage { .. },
            ..
        }
    ));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        vec![Effect::Action(Action::AcceptTeamStage {
            stage: architect::team::TaskStage::Discovery,
            task_ids: vec!["context".into()],
            scope_hash: "stage-scope".into()
        })]
    );
}

#[test]
fn diff_review_confirmation_freezes_candidate_and_profile_across_refresh() {
    let mut app = app();
    app.view = View::Run { tab: 4 };
    app.snapshot
        .scope_hashes
        .insert("review_diff".into(), "original-diff-scope".into());
    app.snapshot.run_details.insert(
        "Diff".into(),
        "Original candidate main.rs +1 -1; reviewer agy model-X".into(),
    );
    assert!(app.handle(key(KeyCode::Char('r'))).is_empty());
    let frozen = app.view.clone();
    assert!(
        matches!(&frozen, View::Confirm {action:Action::ReviewDiff {scope_hash}, details,..}
        if scope_hash=="original-diff-scope" && details.contains("Original candidate") && details.contains("agy model-X"))
    );
    let mut replacement = app.snapshot.clone();
    replacement
        .scope_hashes
        .insert("review_diff".into(), "new-diff-scope".into());
    replacement
        .run_details
        .insert("Diff".into(), "New candidate".into());
    app.update(replacement);
    assert_eq!(app.view, frozen);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        vec![Effect::Action(Action::ReviewDiff {
            scope_hash: "original-diff-scope".into()
        })]
    );
}

#[test]
fn task_recovery_preparation_is_visible_and_prevents_premature_generic_reconciliation() {
    let run = json!({"id":"recovering","status":"blocked","stage":"discovery","reconciliation_required":true});
    let mut app = App::default();
    app.update(Snapshot::from_document(&run,&json!({"task_recovery_state":"preparing","scope_hashes":{"reconcile":"scope"},"recovery":{"observed_input_hash":"inputs","reconciliation_available":true}})));
    assert!(app.next_approval().is_none());
    assert!(app.snapshot.agents.iter().all(|a| a.status != "running"));
    let display = screen(&mut app, 140, 40);
    assert!(display.contains("Checking interrupted tasks"), "{display}");
    app.update(Snapshot::from_document(&run,&json!({"task_recovery_state":"unavailable","scope_hashes":{"reconcile":"scope"},"recovery":{"observed_input_hash":"inputs","reconciliation_available":true}})));
    assert!(app.next_approval().is_none());
    assert!(screen(&mut app, 140, 40).contains("Task recovery changed"));
}

#[test]
fn guided_reconciliation_requires_a_current_observation_and_pins_the_displayed_versions() {
    let run = json!({"id":"interrupted","status":"blocked","stage":"discovery","reconciliation_required":true,"pending_edits":[]});
    let mut control = json!({"scope_hashes":{"reconcile":"scope-before"},"recovery":{"observed_input_hash":"inputs-before","reconciliation_available":true,"inputs":{"source":{"kind":"readable","hash":"source-version"}}}});
    let mut app = App::default();
    app.update(Snapshot::from_document(&run, &control));
    let (action, _) = app.next_approval().unwrap();
    assert_eq!(
        action,
        Action::Reconcile {
            scope_hash: "scope-before".into(),
            input_hash: "inputs-before".into()
        }
    );
    assert!(app.handle(key(KeyCode::Char('a'))).is_empty());
    let View::Confirm { details, .. } = &app.view else {
        panic!("missing recovery card")
    };
    assert!(details.contains("source-version") && details.contains("does not repeat an agent"));
    let frozen = app.view.clone();
    control["scope_hashes"]["reconcile"] = json!("scope-after");
    control["recovery"]["observed_input_hash"] = json!("inputs-after");
    app.update(Snapshot::from_document(&run, &control));
    assert_eq!(app.view, frozen);
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        vec![Effect::Action(action)]
    );
    for available in [false, true] {
        control["recovery"]["reconciliation_available"] = json!(available);
        if available {
            control["recovery"]["observed_input_hash"] = serde_json::Value::Null;
        }
        app.update(Snapshot::from_document(&run, &control));
        assert!(app.next_approval().is_none());
    }
}

#[test]
fn guided_recovery_preserves_a_whole_effect_cohort_before_generic_reconciliation() {
    let mut app = app();
    app.snapshot.status = "blocked".into();
    app.snapshot.reconciliation_required = true;
    app.snapshot.reconciliation_available = true;
    app.snapshot.reconciliation_input_hash = Some("input-observation".into());
    app.snapshot
        .scope_hashes
        .insert("reconcile".into(), "generic-scope".into());
    app.snapshot
        .task_effect_reconciliations
        .insert("one".into(), "individual-scope".into());
    app.snapshot.wave_effect_reconciliations.insert(
        "cohort".into(),
        architect::runtime::team::adjudicate::PendingWave {
            wave_attempt: 1,
            task_ids: vec!["one".into(), "two".into()],
            scope_hash: "cohort-scope".into(),
        },
    );
    assert!(
        matches!(app.next_approval().unwrap().0, Action::ReconcileWaveEffects{task_ids,scope_hash} if task_ids==vec!["one","two"] && scope_hash=="cohort-scope")
    );
    for status in ["running", "paused", "cancelled"] {
        app.snapshot.status = status.into();
        assert!(app.next_approval().is_none());
    }
}

#[test]
fn queued_interview_answer_is_guided_without_requiring_duplicate_submission() {
    let run = json!({"id":"waiting","intake":true,"status":"awaiting_start","reconciliation_required":false,
        "communication_ledger":{"messages":{"answer":{"id":"answer","recipient_agent_id":"orchestrator","sender":{"kind":"user"},"state":"queued_next_turn"}}}});
    let control = json!({"scope_hashes":{"orchestrate":"queued-scope"}});
    let mut app = App::default();
    app.update(Snapshot::from_document(&run, &control));
    assert!(app.snapshot.queued_orchestrator_turn);
    assert!(matches!(
        app.next_approval().unwrap().0,
        Action::Orchestrate { .. }
    ));
    app.handle(key(KeyCode::Char('a')));
    assert_eq!(
        app.handle(key(KeyCode::Enter)),
        vec![Effect::Action(Action::Orchestrate {
            scope_hash: "queued-scope".into()
        })]
    );
    let mut blocked = run;
    blocked["reconciliation_required"] = json!(true);
    app.update(Snapshot::from_document(&blocked, &control));
    assert!(!app.snapshot.queued_orchestrator_turn);
}

#[test]
fn agent_inspector_wraps_every_tab_and_retains_reading_positions() {
    use architect::tui::model::AGENT_TABS;
    let mut app = app();
    for (tab, name) in AGENT_TABS.iter().enumerate().take(7) {
        app.snapshot.agents[0].details.insert((*name).into(), json!([
            {"id":"record-1", "output":format!("{} END_OF_LONG_TEXT\nsecond paragraph", "界abc ".repeat(40))},
            {"id":"record-2", "output":"last-record"}
        ]).to_string());
        if tab == 4 {
            app.snapshot.messages.push(Message {
                id: "message-1".into(),
                target: "parent".into(),
                text: format!("{} END_OF_LONG_TEXT\nsecond paragraph", "界abc ".repeat(40)),
                ..Message::default()
            });
        }
        app.view = View::Agent { tab };
        let mut seen = String::new();
        for _ in 0..40 {
            seen.push_str(&screen(&mut app, 40, 12));
            app.handle(key(KeyCode::PageDown));
        }
        assert!(
            seen.contains("END_OF_LONG_TEXT"),
            "missing long line tail in {name}"
        );
        assert!(
            seen.contains("second paragraph"),
            "missing multiline output in {name}"
        );
        let state = app.inspector_mut(tab);
        assert_eq!(state.offset, state.max_offset());
        app.handle(key(KeyCode::Home));
        screen(&mut app, 40, 12);
        app.handle(key(KeyCode::Down));
        let offset = app.inspector_mut(tab).offset;
        app.handle(key(KeyCode::Right));
        screen(&mut app, 40, 12);
        app.handle(key(KeyCode::Left));
        screen(&mut app, 40, 12);
        assert_eq!(app.inspector_mut(tab).offset, offset);
    }
}

#[test]
fn agent_inspector_search_keeps_complete_records_and_uses_its_own_filter() {
    let mut app = app();
    app.view = View::Agent { tab: 2 };
    app.snapshot.agents[0].details.insert("Artifacts".into(), json!([
        {"id":"artifact-A", "content":"ordinary output"},
        {"id":"artifact-B", "content":"NEEDLE\ncontinued", "hash":"full-hash", "checks":[{"check_id":"C1", "status":"blocked"}]}
    ]).to_string());
    screen(&mut app, 80, 24);
    app.handle(key(KeyCode::Char('/')));
    for c in "needle".chars() {
        app.handle(key(KeyCode::Char(c)));
    }
    app.handle(key(KeyCode::Enter));
    let drawn = screen(&mut app, 80, 24);
    assert!(
        drawn.contains("artifact-B") && drawn.contains("full-hash") && drawn.contains("blocked")
    );
    assert!(!drawn.contains("artifact-A"));
    assert!(app.team_filter.is_empty() && app.chat_filter.is_empty());
    app.handle(key(KeyCode::Char('v')));
    assert!(screen(&mut app, 80, 24).contains("Raw"));
    app.handle(key(KeyCode::Esc));
    assert_eq!(app.view, View::Agent { tab: 2 });
    assert!(app.inspector_mut(2).query.is_empty());
    app.handle(key(KeyCode::Char('/')));
    for c in "no-such-output".chars() {
        app.handle(key(KeyCode::Char(c)));
    }
    assert!(screen(&mut app, 80, 24).contains("No matching records"));
    app.handle(key(KeyCode::Esc));
    assert!(!app.searching);
    assert!(app.inspector_mut(2).query.is_empty());
}

#[test]
fn agent_inspector_clamps_on_resize_refresh_and_keeps_active_narrow_tab() {
    let mut app = app();
    app.view = View::Agent { tab: 6 };
    app.snapshot.agents[0].details.insert(
        "Checks".into(),
        (0..100)
            .map(|i| format!("row-{i}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    assert!(screen(&mut app, 32, 10).contains("7/8 Checks"));
    app.handle(key(KeyCode::End));
    assert!(screen(&mut app, 32, 10).contains("row-99"));
    screen(&mut app, 140, 50);
    let state = app.inspector_mut(6);
    assert_eq!(state.offset, state.max_offset());
    let mut snapshot = app.snapshot.clone();
    snapshot.agents[0]
        .details
        .insert("Checks".into(), "short result".into());
    app.update(snapshot);
    assert!(screen(&mut app, 32, 10).contains("short result"));
    assert_eq!(app.inspector_mut(6).offset, 0);
    for (w, h) in [(12, 5), (16, 7), (24, 10)] {
        screen(&mut app, w, h);
    }
}

#[test]
fn agent_inspector_comments_show_scoped_text_reactions_and_version() {
    use architect::comments::{Comment, Element, ElementKind, ElementRef};
    let mut app = app();
    let target = ElementRef {
        kind: ElementKind::Task,
        id: "task-parent".into(),
        version_hash: "v2".into(),
    };
    app.snapshot.comment_targets.push(Element {
        reference: target.clone(),
        label: "Task".into(),
        agent_ids: ["parent".into()].into(),
        context: json!({}),
    });
    for (id, target_id, text) in [
        ("C1", "task-parent", "scoped comment"),
        ("C2", "other-task", "unrelated comment"),
    ] {
        app.snapshot.element_comments.push(Comment { id:id.into(), run_id:"R1".into(), spec_revision:1,
            target:ElementRef { id:target_id.into(), version_hash:"v1".into(), ..target.clone() },
            context:json!({}), text:text.into(), author:"user".into(), created_at:0, resolved:false, deleted:false,
            reactions:vec![serde_json::from_value(json!({"round_id":"round-1", "response":{"comment_id":id, "kind":"answer", "text":"reviewed reaction", "task_id":null}})).unwrap()] });
    }
    app.view = View::Agent { tab: 7 };
    let doc = architect::tui::inspector::document(&app, 7, false)
        .into_iter()
        .map(|l| l.text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        doc.contains("scoped comment")
            && doc.contains("reviewed reaction")
            && doc.contains("stale or historical")
    );
    assert!(!doc.contains("unrelated comment"));
    app.snapshot.element_comments[0].deleted = true;
    assert!(screen(&mut app, 80, 24).contains("No recorded comments"));
}

#[test]
fn agent_inspector_decodes_nested_output_and_preserves_raw_receipt_data() {
    let mut app = app();
    let output =
        json!({"artifact":"first line\nsecond line", "checks":[{"status":"blocked"}]}).to_string();
    let source = json!({"turns":[{"id":"turn-1", "output":output}]});
    app.snapshot.agents[0]
        .details
        .insert("Session".into(), source.to_string());
    let readable = architect::tui::inspector::document(&app, 1, false)
        .into_iter()
        .map(|l| l.text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(readable.contains("first line") && readable.contains("second line"));
    assert!(!readable.contains("\\n"));
    let raw = architect::tui::inspector::document(&app, 1, true)
        .into_iter()
        .map(|l| l.text)
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&raw).unwrap(),
        source
    );
    app.snapshot.agents[0].details.insert("Messages".into(), json!([
        {"id":"M1", "content_ref":{"document":"body-M1","hash":"full-receipt-hash"}, "delivery_receipt":{"turn_id":"turn-1"}}
    ]).to_string());
    app.snapshot.messages.push(Message {
        id: "M1".into(),
        sender: "user".into(),
        target: "parent".into(),
        text: "actual body".into(),
        state: "queued_next_turn".into(),
        ..Message::default()
    });
    let raw = architect::tui::inspector::document(&app, 4, true)
        .into_iter()
        .map(|l| l.text)
        .collect::<Vec<_>>()
        .join("\n");
    let messages: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(messages[0]["content_ref"]["hash"], "full-receipt-hash");
    assert_eq!(messages[0]["delivery_receipt"]["turn_id"], "turn-1");
    assert_eq!(messages[0]["text"], "actual body");
    assert_eq!(messages[0]["state"], "queued_next_turn");
}

#[test]
fn workspace_approval_cards_freeze_selected_changes_and_destination_across_refresh() {
    for activate in [false, true] {
        let mut app = app();
        app.snapshot.status = "awaiting_start".into();
        app.snapshot.workspace_required = true;
        if activate {
            app.snapshot.workspace_activation = Some("activation-old".into());
            app.snapshot.workspace_activation_details =
                "Captured candidate /private/project, commit old-version".into();
        } else {
            app.snapshot.workspace_preparation = Some(([".".into()].into(), "prepare-old".into()));
            app.snapshot.workspace_preparation_details = "Original source; selected working copy main.rs; leave unrelated.txt outside candidate".into();
        }
        let expected = app.next_approval().unwrap().0;
        app.handle(key(KeyCode::Char('a')));
        let frozen = app.view.clone();
        assert!(
            matches!(&frozen,View::Confirm {details,..} if if activate {details.contains("old-version")} else {details.contains("main.rs") && details.contains("unrelated.txt")})
        );
        let mut changed = app.snapshot.clone();
        changed.workspace_preparation = Some((["new.rs".into()].into(), "prepare-new".into()));
        changed.workspace_activation = Some("activation-new".into());
        changed.workspace_preparation_details = "Different local changes".into();
        changed.workspace_activation_details = "Different candidate".into();
        app.update(changed);
        assert_eq!(app.view, frozen);
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            vec![Effect::Action(expected)]
        );
    }
}

#[test]
fn workspace_overlay_identifies_actual_runtime_work_without_a_native_agent() {
    let mut app = app();
    app.snapshot.status = "running".into();
    app.snapshot.workspace_phase = "Observe workspace inputs and local changes".into();
    app.snapshot.agents.clear();
    let rendered = screen(&mut app, 160, 40);
    assert!(rendered.contains("Current step | Workspace"), "{rendered}");
    assert!(rendered.contains("Observe workspace inputs"), "{rendered}");
}

#[test]
fn completed_task_delivery_and_recovery_cards_pin_the_original_checkout_and_versions() {
    for recovery in [false, true] {
        let mut app = app();
        app.snapshot.status = "completed".into();
        app.snapshot.delivery_details =
            "Original checkout /user/project; reviewed candidate v1; retain backup for main.rs"
                .into();
        if recovery {
            app.snapshot.delivery_recovery = Some(architect::runtime::delivery::Recovery {
                scope_hash: "observed-v1".into(),
                observation_hash: "actual-v1".into(),
                targets: Default::default(),
                backups: Default::default(),
                restorable: vec!["main.rs".into()],
                desired_state_observed: false,
            });
        } else {
            app.snapshot.delivery_scope = Some("delivery-v1".into());
        }
        let expected = app.next_approval().unwrap().0;
        app.handle(key(KeyCode::Char('a')));
        let frozen = app.view.clone();
        assert!(
            matches!(&frozen,View::Confirm {details,..} if details.contains("/user/project") && details.contains("v1"))
        );
        let mut changed = app.snapshot.clone();
        changed.delivery_scope = Some("delivery-v2".into());
        changed.delivery_recovery = None;
        changed.delivery_details = "Different result".into();
        app.update(changed);
        assert_eq!(app.view, frozen);
        assert_eq!(
            app.handle(key(KeyCode::Enter)),
            vec![Effect::Action(expected)]
        );
    }
}

#[test]
fn interview_source_observation_overlay_is_runtime_work_without_a_native_agent() {
    let document = json!({"id":"source-observation","intake":true,"status":"running","stage":"discovery","orchestration_rounds":[{"id":"round","status":"running","discovery_status":"observing_sources"}]});
    let mut app = App::default();
    app.update(Snapshot::from_document(&document, &json!({})));
    let text = screen(&mut app, 140, 30);
    assert!(text.contains("Observe project sources for the interview"));
    assert!(!text.contains("Discover CLI capabilities"));
    assert!(app.snapshot.agents.is_empty());
}

#[test]
fn context_overlay_counts_unique_native_observed_bytes_and_hides_activity_when_paused() {
    use architect::{
        context::{Read, transport::Transport},
        domain::{Run, RunSpec},
        providers::TurnRequest,
    };
    let root = tempfile::tempdir().unwrap();
    let mut spec: RunSpec =
        serde_json::from_str(include_str!("../examples/fake-spec.json")).unwrap();
    spec.workspace = root.path().to_string_lossy().into();
    let run = Run::new(architect::id(), spec).unwrap();
    let prompt = "Complete original pinned source to read before answering";
    let mut transport = Transport::new(&TurnRequest {
        turn_id: "context-turn",
        run: &run,
        prompt,
        schema: &json!({"type":"object"}),
        resume: None,
    })
    .unwrap();
    let reader = transport.reader().unwrap();
    let source_hash = transport.bundle.scope.prompt_hash.clone();
    for (sequence, doc, size) in [
        (1, reader.index_hash(), 16384),
        (2, source_hash.as_str(), 8),
        (3, source_hash.as_str(), 8),
    ] {
        let page = reader
            .read(
                &Read {
                    bundle_hash: reader.bundle_hash().into(),
                    document_hash: doc.into(),
                    offset: 0,
                    max_bytes: size,
                },
                sequence,
            )
            .unwrap();
        transport.accept(page.receipt).unwrap();
        transport.observe(sequence).unwrap();
    }
    let (read, total) = transport.progress().unwrap();
    assert_eq!(read, reader.index_bytes() + 8);
    let mut document = json!({"id":run.id,"status":"running","stage":"discovery","session_ledger":{"sessions":{"context-agent":{"id":"context-agent","state":"sent","current_turn_id":"context-turn","scope":{"agent_id":"reader"}}},"turns":{"context-turn":{"context":transport}}}});
    let mut app = App::default();
    app.update(Snapshot::from_document(&document, &json!({})));
    assert_eq!(
        app.snapshot.context_progress,
        vec![("reader".into(), read, total)]
    );
    let text = screen(&mut app, 160, 32);
    assert!(text.contains("Current step | Context delivery"));
    assert!(text.contains(&format!("read context {read} / {total} bytes")));
    document["status"] = json!("paused");
    app.update(Snapshot::from_document(&document, &json!({})));
    let text = screen(&mut app, 160, 32);
    assert!(!text.contains("Context delivery"));
    assert!(!text.contains("read context"));
}
