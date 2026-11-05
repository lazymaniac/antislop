use super::*;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

fn screen(term: &Terminal<TestBackend>) -> Vec<String> {
    let buffer = term.backend().buffer();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect()
        })
        .collect()
}

fn view_of(doc: &str) -> View {
    let mut view = View::default();
    view.plan_doc = doc.into();
    view
}

#[test]
fn tiny_redraw_discards_old_document_hit_boxes() {
    let view = view_of("a plan line");
    let theme = Theme::new(true, false);
    let mut plan = PlanView::new("p1".into());
    let mut wide = Terminal::new(TestBackend::new(60, 20)).unwrap();
    wide.draw(|frame| plan.draw(frame, frame.area(), &view, &theme))
        .unwrap();
    assert!(plan.doc_area.width > 0);
    let mut tiny = Terminal::new(TestBackend::new(12, 5)).unwrap();
    tiny.draw(|frame| plan.draw(frame, frame.area(), &view, &theme))
        .unwrap();
    assert_eq!(plan.doc_area, Rect::default());
    assert!(plan.row_docs.is_empty());
}

#[test]
fn blank_plan_documents_synthesize_the_default_plan_document() {
    let mut view = view_of(" \n\t ");
    view.tasks.clear();
    let lines = PlanView::new("p1".into()).build_lines(&view);
    assert_eq!(lines[0].text, "# Execution Plan");
    assert!(lines[0].is_heading);
    assert!(
        lines
            .iter()
            .any(|line| line.text.contains("No tasks proposed yet"))
    );
}

#[test]
fn long_lines_wrap_and_every_word_stays_readable() {
    let words: Vec<String> = (0..30).map(|n| format!("word{n}")).collect();
    let view = view_of(&format!("# Title\n{}\nlast", words.join(" ")));
    let mut plan = PlanView::new("p".into());
    let mut term = Terminal::new(TestBackend::new(40, 16)).unwrap();
    term.draw(|f| plan.draw(f, f.area(), &view, &Theme::new(true, false)))
        .unwrap();
    let text = screen(&term).join("\n");
    for word in &words {
        assert!(text.contains(word.as_str()), "{word} missing in\n{text}");
    }
    // Continuation rows carry a bare separator instead of a number.
    assert!(text.contains("     │ "));
}

#[test]
fn wrapped_graphemes_are_never_split() {
    let view = view_of(&"👩‍💻e\u{301}".repeat(40));
    let mut plan = PlanView::new("p".into());
    let mut term = Terminal::new(TestBackend::new(30, 14)).unwrap();
    term.draw(|f| plan.draw(f, f.area(), &view, &Theme::new(true, false)))
        .unwrap();
    let text = screen(&term).join("");
    assert!(!text.contains('\u{fffd}'));
    assert!(text.contains("👩‍💻"));
}

#[test]
fn the_cursor_line_stays_in_view_and_clicks_pick_the_wrapped_line() {
    let long = "alpha beta gamma delta epsilon ".repeat(8);
    let doc: Vec<String> = (0..12)
        .map(|n| {
            if n == 5 {
                long.clone()
            } else {
                format!("line {n}")
            }
        })
        .collect();
    let view = view_of(&doc.join("\n"));
    let mut plan = PlanView::new("p".into());
    let theme = Theme::new(true, false);
    let mut term = Terminal::new(TestBackend::new(40, 14)).unwrap();
    plan.cursor = 11;
    term.draw(|f| plan.draw(f, f.area(), &view, &theme))
        .unwrap();
    assert!(screen(&term).join("\n").contains("line 11"));

    plan.cursor = 0;
    term.draw(|f| plan.draw(f, f.area(), &view, &theme))
        .unwrap();
    // Find a continuation row of the long line and click it.
    let continuation = plan
        .row_docs
        .iter()
        .enumerate()
        .skip(plan.scroll)
        .filter(|(_, doc)| **doc == Some(5))
        .map(|(row, _)| row)
        .nth(1)
        .expect("the long line wraps");
    let row = plan.doc_area.y + (continuation - plan.scroll) as u16;
    plan.mouse_click(plan.doc_area.x + 3, row, &view);
    assert_eq!(plan.cursor, 5);
}

fn proposed(doc: &str) -> View {
    let mut view = view_of(doc);
    view.tasks.push(TaskRow {
        id: "t1".into(),
        status: "proposed".into(),
        ..TaskRow::default()
    });
    view
}

fn press(plan: &mut PlanView, view: &View, code: KeyCode) -> PlanAction {
    plan.key(KeyEvent::new(code, KeyModifiers::NONE), view)
}

#[test]
fn enter_comments_and_accept_keys_request_acceptance() {
    let view = proposed("# Plan\nbody");
    let mut plan = PlanView::new("p".into());
    assert_eq!(
        press(&mut plan, &view, KeyCode::Enter),
        PlanAction::CommentWholePlan
    );
    for code in [KeyCode::Char('a'), KeyCode::Char('1')] {
        assert_eq!(press(&mut plan, &view, code), PlanAction::Accept);
    }
}

fn task_in(id: &str, section: &str, status: &str) -> TaskRow {
    TaskRow {
        id: id.into(),
        title: format!("Title {id}"),
        section: Some(section.into()),
        status: status.into(),
        ..TaskRow::default()
    }
}

fn states(view: &View) -> Vec<(String, SectionState)> {
    PlanView::new("p".into())
        .build_lines(view)
        .into_iter()
        .map(|line| (line.text, line.state))
        .collect()
}

#[test]
fn a_section_is_done_only_when_all_its_tasks_are_and_subheadings_inherit() {
    let mut view = view_of("## Storage\ntext `t2` mentioned\n### Details\n## Other\nbody");
    view.tasks = vec![
        task_in("t1", "Storage", "approved"),
        task_in("t2", "Storage", "working"),
        task_in("t3", "Other", "failed"),
    ];
    let got = states(&view);
    // One of two tasks is still working: the section is not completed,
    // and a mention of a task id in prose does not change the section.
    assert_eq!(got[0].1, SectionState::Working);
    assert_eq!(got[1].1, SectionState::Working);
    assert_eq!(got[2].1, SectionState::Working);
    assert_eq!(got[3].1, SectionState::Problem);
    assert_eq!(got[4].1, SectionState::Problem);

    view.tasks[1].status = "approved".into();
    assert_eq!(states(&view)[0].1, SectionState::Done);
    assert_eq!(states(&view)[2].1, SectionState::Done);
}

#[test]
fn headings_match_sections_exactly_not_by_substring() {
    let mut view = view_of("## Storage layer rewrite\n## Storage");
    view.tasks = vec![task_in("t1", "Storage", "approved")];
    let got = states(&view);
    assert_eq!(got[0].1, SectionState::Open);
    assert_eq!(got[1].1, SectionState::Done);
}

fn comment(id: u64, status: &str, line: usize) -> Comment {
    Comment {
        id,
        plan_id: Some("p".into()),
        target: format!("plan:p:line:{line}"),
        text: format!("remark {id}"),
        status: status.into(),
    }
}

#[test]
fn review_sends_the_drafts_only_when_there_are_some() {
    let mut view = proposed("# Plan\nbody\nmore");
    view.plan = Some("p".into());
    let mut plan = PlanView::new("p".into());
    assert_eq!(
        press(&mut plan, &view, KeyCode::Char('R')),
        PlanAction::None
    );
    plan.button = BottomButton::Review;
    assert_eq!(press(&mut plan, &view, KeyCode::Enter), PlanAction::None);

    view.comments.insert(1, comment(1, "draft", 2));
    view.comments.insert(2, comment(2, "open", 3));
    assert_eq!(view.draft_plan_comments().len(), 1);
    assert_eq!(
        press(&mut plan, &view, KeyCode::Char('R')),
        PlanAction::SubmitReview
    );
    assert_eq!(
        press(&mut plan, &view, KeyCode::Enter),
        PlanAction::SubmitReview
    );

    // The bar offers it, the banner explains it, and a click sends it.
    let theme = Theme::new(true, false);
    let mut term = Terminal::new(TestBackend::new(100, 20)).unwrap();
    term.draw(|f| plan.draw(f, f.area(), &view, &theme))
        .unwrap();
    let text = screen(&term).join("\n");
    assert!(text.contains("Review: send (R)"), "{text}");
    assert!(text.contains("1 draft comment(s)"), "{text}");
    assert!(text.contains("1 unresolved comment(s)"), "{text}");
    assert!(
        text.contains("[draft]"),
        "the comment row says it is a draft"
    );
    let rect = plan.button_rects[3];
    assert_eq!(
        plan.mouse_click(rect.x + 2, rect.y + 1, &view),
        PlanAction::SubmitReview
    );
    // D takes back the draft on the cursor's line.
    plan.cursor = 1;
    assert_eq!(
        press(&mut plan, &view, KeyCode::Char('D')),
        PlanAction::ToggleResolveComment {
            id: 1,
            next_status: "deleted".into()
        }
    );
    // D also deletes a whole-plan draft comment when the cursor line has no line comment.
    view.comments.insert(
        3,
        Comment {
            id: 3,
            plan_id: Some("p".into()),
            target: "plan:p".into(),
            text: "whole plan remark".into(),
            status: "draft".into(),
        },
    );
    plan.cursor = 0;
    assert_eq!(
        press(&mut plan, &view, KeyCode::Char('D')),
        PlanAction::ToggleResolveComment {
            id: 3,
            next_status: "deleted".into()
        }
    );
}

fn graph_view() -> View {
    let mut view = view_of(
        "# Plan\nOpis 👩‍💻 e\u{301}\n\n### 1. First [t1]\n- **Task**: `t1`\n- **Goal**: First goal\n\n### 2. Second [t2]\n- **Task**: `t2`\n\n## Notes\nKeep this note",
    );
    view.tasks = vec![
        task_in("t1", "First", "approved"),
        task_in("t2", "Second", "proposed"),
    ];
    view.tasks[1].depends_on = vec!["t1".into()];
    view
}

#[test]
fn split_plan_keeps_prose_and_comment_anchors_and_draws_dependency_edges() {
    let mut view = graph_view();
    view.comments.insert(1, comment(1, "draft", 12));
    let mut plan = PlanView::new("p".into());
    let mut term = Terminal::new(TestBackend::new(140, 30)).unwrap();
    term.draw(|f| plan.draw(f, f.area(), &view, &Theme::new(true, false)))
        .unwrap();
    assert!(plan.doc_area.right() < plan.graph_area.x);
    let text = screen(&term).join("\n");
    assert!(text.contains("Description (v)"));
    assert!(text.contains("Opis 👩‍💻") && text.contains("e\u{301}"));
    assert!(text.contains("Title t1"));
    assert!(text.contains("Title t2"));
    assert!(text.contains('▼'), "the dependency has an arrow: {text}");
    assert!(!text.contains("**Task**"));
    assert!(!text.contains("### 1."));
    let prose = plan.description_lines(&view);
    let note_index = prose
        .iter()
        .position(|line| line.text == "Keep this note")
        .unwrap();
    assert_eq!(prose[note_index].number, 12);
    plan.cursor = note_index;
    assert!(matches!(
        press(&mut plan, &view, KeyCode::Char('c')),
        PlanAction::CommentLine { line: 12, .. }
    ));
    assert_eq!(
        press(&mut plan, &view, KeyCode::Char('D')),
        PlanAction::ToggleResolveComment {
            id: 1,
            next_status: "deleted".into()
        }
    );
    press(&mut plan, &view, KeyCode::Char('v'));
    press(&mut plan, &view, KeyCode::Right);
    assert_eq!(plan.graph_selected, 1);
    assert!(matches!(
        press(&mut plan, &view, KeyCode::Char('c')),
        PlanAction::CommentLine { line: 8, .. }
    ));
    press(&mut plan, &view, KeyCode::Left);
    assert_eq!(plan.graph_selected, 0);
    assert_eq!(
        plan.cursor, note_index,
        "graph navigation preserves the prose cursor"
    );
    // Task comments appear at the top of Selected task even when goal is very long.
    view.tasks[0].goal = "very long goal text ".repeat(30);
    view.comments.insert(2, comment(2, "draft", 4));
    term.draw(|f| plan.draw(f, f.area(), &view, &Theme::new(true, false)))
        .unwrap();
    let text = screen(&term).join("\n");
    assert!(
        text.contains("#2 [draft]: remark 2"),
        "task comment is visible in Selected task:\n{text}"
    );
    // Clicking in Selected task below graph_area focuses the task graph pane.
    plan.graph_focused = false;
    plan.mouse_click(plan.graph_area.x + 2, plan.graph_area.bottom() + 1, &view);
    assert!(plan.graph_focused);
}

#[test]
fn graph_pan_survives_redraw_and_selecting_a_task_reveals_it() {
    let mut view = graph_view();
    for index in 3..20 {
        let mut task = task_in(&format!("t{index}"), "", "proposed");
        task.depends_on = vec![format!("t{}", index - 1)];
        view.tasks.push(task);
    }
    let mut plan = PlanView::new("p".into());
    let mut term = Terminal::new(TestBackend::new(100, 26)).unwrap();
    let draw = |plan: &mut PlanView, term: &mut Terminal<TestBackend>| {
        term.draw(|f| plan.draw(f, f.area(), &view, &Theme::new(true, false)))
            .unwrap();
    };
    draw(&mut plan, &mut term);
    press(&mut plan, &view, KeyCode::Char('v'));
    plan.key(KeyEvent::new(KeyCode::Down, KeyModifiers::SHIFT), &view);
    draw(&mut plan, &mut term);
    assert_eq!(plan.graph_offset.1, 3);
    assert_eq!(plan.graph_offset.0, 0);
    draw(&mut plan, &mut term);
    assert_eq!(plan.graph_offset.1, 3);
    assert_eq!(plan.graph_offset.0, 0);
    press(&mut plan, &view, KeyCode::End);
    draw(&mut plan, &mut term);
    assert!(screen(&term).join("\n").contains("Title t19"));
    assert!(plan.graph_offset.1 > 3);
    assert_eq!(plan.graph_offset.0, 0);
    let (rect, index) = plan.task_rects.last().copied().unwrap();
    plan.mouse_click(rect.x, rect.y, &view);
    assert_eq!(plan.graph_selected, index);
    assert!(plan.graph_focused);
    plan.mouse_click(plan.doc_area.x, plan.doc_area.y, &view);
    assert!(!plan.graph_focused);
}

#[test]
fn split_plan_handles_small_terminals_and_terminal_fallbacks() {
    let view = graph_view();
    let mut plan = PlanView::new("p".into());
    for (width, height) in [(140, 30), (40, 24), (20, 9), (12, 5), (2, 2), (1, 1)] {
        let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
        term.draw(|f| plan.draw(f, f.area(), &view, &Theme::new(false, true)))
            .unwrap();
        for cell in &term.backend().buffer().content {
            assert!(cell.symbol().is_ascii());
            assert_eq!(cell.fg, ratatui::style::Color::Reset);
            assert_eq!(cell.bg, ratatui::style::Color::Reset);
        }
        if width == 40 {
            assert!(plan.doc_area.bottom() <= plan.graph_area.y);
        }
        if width <= 12 {
            assert!(plan.task_rects.is_empty());
            assert_eq!(plan.graph_area, Rect::default());
            assert_eq!(plan.button_rects, [Rect::default(); 4]);
        }
    }
}
