use super::health::{card_lines, describe};
use super::nodes::draw_node;
use super::*;

#[test]
fn windowed_canvas_storage_is_exactly_the_viewport_area() {
    let canvas = Canvas::new_window(40, 20, 17, 9);
    assert_eq!(canvas.cells.len(), 17 * 9);
    assert_eq!(canvas.width, 17);
    assert_eq!(canvas.height, 9);
}

#[test]
fn expanded_card_areas_clip_with_a_card_scrolled_past_the_top() {
    let viewport = Rect::new(10, 5, 80, 30);
    let [card, title, content] = super::expanded_card_areas(viewport, (20, -1), (40, 20));
    assert_eq!(card, Rect::new(20, 5, 40, 14));
    // The title row sits at top = -1, above the viewport.
    assert!(title.is_empty());
    // Content spans rows 1..18 of the card; only rows 5..18 are visible.
    assert_eq!(content, Rect::new(22, 5, 36, 13));
    assert_eq!(
        content.bottom(),
        card.bottom() - 1,
        "content ends at the frame"
    );
    let [card, ..] = super::expanded_card_areas(viewport, (20, -30), (40, 20));
    assert!(card.is_empty(), "a card scrolled out of view has no area");
}

#[test]
fn expanded_card_content_grows_with_the_viewport() {
    let viewport = (76, 12);
    let size = Size::fit(Some(viewport), Some("worker:t1".into()));
    let content_area = |width: u16, height: u16| {
        super::expanded_card_areas(
            Rect::new(0, 0, viewport.0, viewport.1),
            (0, 0),
            (width, height),
        )[2]
    };

    // Before the viewport-fit change the 56-column ceiling produced a
    // 52x7 PTY content area at this same viewport.
    assert_eq!(
        usize::from(content_area(56, 10).width) * usize::from(content_area(56, 10).height),
        364
    );
    assert_eq!((size.expanded_w, size.expanded_h), (74, 10));
    let expanded = content_area(size.expanded_w, size.expanded_h);
    let baseline = content_area(56, 10);
    assert_eq!(
        usize::from(expanded.width) * usize::from(expanded.height),
        490
    );
    assert!(
        usize::from(expanded.width) * usize::from(expanded.height)
            > usize::from(baseline.width) * usize::from(baseline.height)
    );
}

#[test]
fn expanded_standalone_agent_keeps_its_canvas_margin_without_a_task_frame() {
    let mut view = view(vec![task("t1", &[])]);
    view.agents
        .insert(ORCHESTRATOR.into(), super::super::view::Agent::default());
    let viewport = (80, 24);
    let graph = Graph::new_with_expanded(
        &view,
        &Scope::default(),
        &Default::default(),
        GroupingMode::Dependency,
        Some(viewport),
        Some(ORCHESTRATOR.into()),
        0,
    );
    let node = graph.node(ORCHESTRATOR).unwrap();
    assert!(node.visible);
    assert!(node.group.is_none());
    assert_eq!(node.x, 1);
    assert_eq!((node.width, node.height), (viewport.0 - 2, viewport.1 - 2));
}

#[test]
fn small_viewports_shrink_cards_to_two_by_two_and_wide_ones_center_them() {
    let tasks = (1..=6).map(|n| task(&format!("t{n}"), &[])).collect();
    let view = view(tasks);
    let layout = |vp| {
        Graph::new_with_viewport(
            &view,
            &Scope::default(),
            &Default::default(),
            GroupingMode::Dependency,
            Some(vp),
        )
    };
    let small = layout((70, 24));
    for node in small.nodes.iter().filter(|node| node.visible) {
        assert!(node.width <= 40 && node.height < HEIGHT, "{node:?}");
    }
    assert!(small.width <= 70, "{}", small.width);
    assert!(2 * small.nodes[0].height + GAP_Y <= 24);
    let wide = layout((300, 80));
    let left = wide.nodes.iter().filter(|n| n.visible).map(|n| n.x).min();
    assert!(left.unwrap() > MARGIN);
    assert!(wide.width <= 300);
}

#[test]
fn expanded_card_reflows_the_graph_without_overlapping_its_neighbours() {
    let view = view(vec![task("t1", &[]), task("t2", &["t1"])]);
    let graph = Graph::new_with_expanded(
        &view,
        &Scope::default(),
        &Default::default(),
        GroupingMode::Dependency,
        Some((120, 48)),
        Some("worker:t1".into()),
        0,
    );
    let expanded = graph.node("worker:t1").unwrap();
    assert!(expanded.width > WIDTH);
    assert!(expanded.height > HEIGHT);
    // The expanded card, its reviewer and their frame stay within the
    // viewport: opening a card never needs a horizontal scroll.
    for (width, key) in [(120, "worker:t1"), (90, "reviewer:t2"), (120, ORCHESTRATOR)] {
        let graph = Graph::new_with_expanded(
            &view,
            &Scope::default(),
            &Default::default(),
            GroupingMode::Dependency,
            Some((width, 48)),
            Some(key.into()),
            0,
        );
        assert!(graph.width <= width, "{key} at {width}: {}", graph.width);
        assert!(graph.node(key).unwrap().width > WIDTH);
    }
    assert!(
        graph
            .nodes
            .iter()
            .filter(|node| node.visible && node.key != expanded.key)
            .all(|node| expanded.right() <= node.x
                || node.right() <= expanded.x
                || expanded.y.saturating_add(expanded.height) <= node.y
                || node.y.saturating_add(node.height) <= expanded.y)
    );
}

#[test]
fn expanded_unit_has_its_own_row_and_rendered_dependencies_miss_card_contents() {
    let mut tasks = vec![task("t1", &[]), task("t2", &[])];
    tasks[0].title = "Café 👩‍💻 root".into();
    tasks[0].summary = Some("Combining e\u{301} and a joined glyph 👩‍💻".into());
    tasks.extend([
        task("t3", &["t1"]),
        task("t4", &["t2"]),
        task("t5", &["t3", "t4"]),
    ]);
    let view = view(tasks);
    let graph = Graph::new_with_expanded(
        &view,
        &Scope::default(),
        &Default::default(),
        GroupingMode::Dependency,
        None,
        Some("worker:t1".into()),
        0,
    );
    let expanded = graph.node("worker:t1").unwrap();
    let sibling = graph.node("worker:t2").unwrap();
    assert_ne!(expanded.y, sibling.y, "expanded unit gets a dedicated row");
    let cards: Vec<_> = graph.nodes.iter().filter(|node| node.visible).collect();
    for (index, left) in cards.iter().enumerate() {
        for right in &cards[index + 1..] {
            assert!(
                left.x.saturating_add(left.width) <= right.x
                    || right.x.saturating_add(right.width) <= left.x
                    || left.y.saturating_add(left.height) <= right.y
                    || right.y.saturating_add(right.height) <= left.y,
                "visible cards overlap: {left:?} and {right:?}"
            );
        }
    }
    for (from, to, path) in graph.edge_routes(Relations::all()) {
        for node in &cards {
            assert!(
                !Graph::route_intersects_card(&path, node),
                "edge {from} -> {to} crosses card {} at ({},{})",
                node.key,
                node.x,
                node.y,
            );
        }
    }
    let compact = Graph::new_with_viewport(
        &view,
        &Scope::default(),
        &Default::default(),
        GroupingMode::Dependency,
        None,
    );
    let mut compact_canvas = Canvas::new(compact.width, compact.height);
    compact.render_with(
        &view,
        &mut compact_canvas,
        Relations::all(),
        "worker:t1",
        &Theme::new(false, false),
        0,
    );
    assert!(compact_canvas.lines().join("\n").contains("Café"));
    for (width, height, theme) in [
        (180, 70, Theme::new(false, false)),
        (80, 24, Theme::new(false, true)),
        (11, 4, Theme::new(false, true)),
    ] {
        let viewport = if (width, height) == (180, 70) {
            None
        } else {
            Some((width, height))
        };
        let graph = Graph::new_with_expanded(
            &view,
            &Scope::default(),
            &Default::default(),
            GroupingMode::Dependency,
            viewport,
            Some("worker:t1".into()),
            0,
        );
        let expanded = graph.node("worker:t1").unwrap();
        let sibling = graph.node("worker:t2").unwrap();
        println!(
            "rendered terminal={width}x{height} card=t1@{},{} {}x{} sibling=t2@{},{} {}x{} graph={}x{}",
            expanded.x,
            expanded.y,
            expanded.width,
            expanded.height,
            sibling.x,
            sibling.y,
            sibling.width,
            sibling.height,
            graph.width,
            graph.height
        );
        let visible: Vec<_> = graph.nodes.iter().filter(|node| node.visible).collect();
        for (index, left) in visible.iter().enumerate() {
            for right in &visible[index + 1..] {
                assert!(
                    left.x.saturating_add(left.width) <= right.x
                        || right.x.saturating_add(right.width) <= left.x
                        || left.y.saturating_add(left.height) <= right.y
                        || right.y.saturating_add(right.height) <= left.y,
                    "terminal={width}x{height} cards overlap: {left:?} and {right:?}"
                );
            }
        }
        for (from, to, path) in graph.edge_routes(Relations::all()) {
            for node in &visible {
                assert!(
                    !Graph::route_intersects_card(&path, node),
                    "terminal={width}x{height} edge {from} -> {to} crosses {} at ({},{}), path={path:?}",
                    node.key,
                    node.x,
                    node.y
                );
            }
        }
        let mut canvas = Canvas::new_window(0, 0, width, height);
        graph.render_with(&view, &mut canvas, Relations::all(), "worker:t1", &theme, 0);
        let rendered = canvas.lines().join("\n");
        if width >= 12 && height >= 5 {
            assert!(
                rendered.contains("Café"),
                "terminal={width}x{height}: {rendered}"
            );
            assert!(
                rendered.contains("1 Details"),
                "terminal={width}x{height}: {rendered}"
            );
        }
        assert_eq!(canvas.cells.len(), usize::from(width) * usize::from(height));
    }
}

#[test]
fn expanded_details_tab_shows_agent_and_task_metadata_in_card() {
    let mut task = task("t1", &[]);
    task.title = "Implement graph cards".into();
    task.round = 2;
    let mut view = view(vec![task]);
    view.agents.insert(
        "worker:t1".into(),
        super::super::view::Agent {
            provider: "codex".into(),
            model: Some("gpt-test".into()),
            last_ms: Some(1_200),
            ..Default::default()
        },
    );
    let graph = Graph::new_with_expanded(
        &view,
        &Scope::default(),
        &Default::default(),
        GroupingMode::Dependency,
        Some((120, 48)),
        Some("worker:t1".into()),
        0,
    );
    let mut canvas = Canvas::new(graph.width, graph.height);
    graph.render(&view, &mut canvas, Relations::all(), "worker:t1");
    let text = canvas.lines().join("\n");
    for expected in [
        "1 Details",
        "CLI: codex",
        "Model: gpt-test",
        "Elapsed:",
        "Task: Implement graph cards",
        "Round: 2",
    ] {
        assert!(text.contains(expected), "missing {expected} in {text}");
    }
    assert_eq!(text.matches("Round: 2").count(), 1, "{text}");
    assert!(
        !text.contains("round 2"),
        "status duplicates Details round: {text}"
    );
    // The title row and the tab strip stand out from the card body, and the
    // open tab stands out from the strip.
    let theme = Theme::new(true, false);
    let node = graph.node("worker:t1").unwrap();
    let mut canvas = Canvas::new(graph.width, graph.height);
    graph.render_with(&view, &mut canvas, Relations::all(), "worker:t1", &theme, 0);
    let mut bg = |x: u16, y: u16| canvas.cell(x, y).unwrap().style.bg;
    let header = theme.card_header().bg;
    assert_eq!(bg(node.x + node.width / 2, node.y), header);
    assert_eq!(bg(node.right() - 2, node.y), header);
    let strip = theme.tab_strip().bg;
    assert_eq!(bg(node.right() - 2, node.y + 1), strip);
    assert_eq!(bg(node.x + 3, node.y + 1), theme.tab_active().bg);
    let body = bg(node.x + 3, node.y + 3);
    assert!(header != body && strip != body && header != strip);
}

#[test]
fn cached_layout_reuses_idle_frames_and_invalidates_changed_inputs() {
    let mut view = super::View::default();
    let mut cache = super::LayoutCache::default();
    let mut collapsed = std::collections::HashSet::new();
    let first = cache.get(
        &view,
        1,
        &Scope::default(),
        &collapsed,
        super::GroupingMode::Dependency,
        Some((80, 24)),
    );
    let idle = cache.get(
        &view,
        1,
        &Scope::default(),
        &collapsed,
        super::GroupingMode::Dependency,
        Some((80, 24)),
    );
    assert!(std::rc::Rc::ptr_eq(&first, &idle));
    view.apply(
        &serde_json::json!({"seq": 1, "kind": "plan_proposed", "payload": {
            "plan": "p1", "tasks": [{"change_kind":"none","id": "t1", "title": "first", "depends_on": []}]}}),
    );
    let updated = cache.get(
        &view,
        1,
        &Scope::default(),
        &collapsed,
        super::GroupingMode::Dependency,
        Some((80, 24)),
    );
    assert!(!std::rc::Rc::ptr_eq(&first, &updated));
    let resized = cache.get(
        &view,
        1,
        &Scope::default(),
        &collapsed,
        super::GroupingMode::Dependency,
        Some((40, 12)),
    );
    assert!(!std::rc::Rc::ptr_eq(&updated, &resized));
    collapsed.insert("t1".into());
    let folded = cache.get(
        &view,
        1,
        &Scope::default(),
        &collapsed,
        super::GroupingMode::Status,
        Some((40, 12)),
    );
    assert!(!std::rc::Rc::ptr_eq(&resized, &folded));
    let searched = cache.get(
        &view,
        1,
        &scoped("first"),
        &collapsed,
        super::GroupingMode::Status,
        Some((40, 12)),
    );
    assert!(!std::rc::Rc::ptr_eq(&folded, &searched));
    let switched = cache.get(
        &view,
        2,
        &scoped("first"),
        &collapsed,
        super::GroupingMode::Status,
        Some((40, 12)),
    );
    assert!(!std::rc::Rc::ptr_eq(&searched, &switched));
    // Expiring mail highlights must invalidate an otherwise idle graph.
    cache.entry.as_mut().unwrap().expires = Some(std::time::Instant::now());
    let expired = cache.get(
        &view,
        2,
        &scoped("first"),
        &collapsed,
        super::GroupingMode::Status,
        Some((40, 12)),
    );
    assert!(!std::rc::Rc::ptr_eq(&switched, &expired));
}

#[test]
fn clipped_paths_match_full_rendering_at_joints_and_viewport_edges() {
    let routes = [
        vec![(0, 18), (70, 18)],
        vec![(25, 0), (25, 50)],
        vec![(31, 14), (20, 14), (20, 22), (33, 22)],
        vec![(36, 25), (22, 16)],
        vec![(29, 21), (29, 16), (22, 16)],
    ];
    let style = ratatui::style::Style::default().fg(crate::lean::tui::theme::ayu::BLUE);
    let mut full = super::Canvas::new_window(0, 0, 80, 60);
    let mut clipped = super::Canvas::new_window(20, 15, 12, 7);
    for route in &routes {
        full.path(route, style);
        clipped.path(route, style);
    }
    for y in 15..22 {
        for x in 20..32 {
            let expected = full.cell(x, y).unwrap();
            let actual = clipped.cell(x, y).unwrap();
            assert_eq!(super::Canvas::glyph(expected), super::Canvas::glyph(actual));
            assert_eq!(expected.style, actual.style);
        }
    }
    let mut far = super::Canvas::new_window(20, 15, 12, 7);
    far.path(&[(0, 18), (u16::MAX, 18)], style);
    far.path(&[(25, 0), (25, u16::MAX)], style);
    assert_eq!(far.cells.len(), 12 * 7);
    assert_eq!(super::Canvas::glyph(far.cell(25, 18).unwrap()), "┼");
}

use crate::lean::tui::view::TaskRow;

fn task(id: &str, depends: &[&str]) -> TaskRow {
    TaskRow {
        id: id.into(),
        title: format!("Task {id}"),
        depends_on: depends.iter().map(|id| id.to_string()).collect(),
        status: "proposed".into(),
        ..TaskRow::default()
    }
}

fn view(tasks: Vec<TaskRow>) -> View {
    let mut view = View::default();
    view.tasks = tasks;
    view
}

#[test]
fn compact_cards_with_effects_can_have_no_effect_rows() {
    let mut row = task("t1", &[]);
    row.effect = Some("Updated storage.".into());
    let view = view(vec![row]);
    let mut node = Node::new("worker:t1".into(), 0, 0);
    node.height = 4;
    let mut canvas = Canvas::new_window(0, 0, node.width, node.height);
    draw_node(
        &view,
        &mut canvas,
        &node,
        false,
        None,
        &Theme::new(true, false),
        0,
    );
    assert!(canvas.lines().iter().all(|line| !line.contains("effect:")));
}

#[test]
fn summaries_are_bounded_and_truncated_at_grapheme_boundaries() {
    let mut row = task("t1", &[]);
    row.summary = Some("👩‍💻e\u{301}".repeat(100));
    row.reviewer_summary = Some("Check the summary.".into());
    let view = view(vec![row]);
    let lines = card_lines(
        &view,
        "worker:t1",
        usize::from(WIDTH - 4),
        MAX_SUMMARY_LINES,
    );
    assert_eq!(lines.len(), MAX_SUMMARY_LINES);
    assert!(lines.last().unwrap().ends_with('…'));
    for line in &lines {
        assert!(line.width() <= usize::from(WIDTH - 4));
        assert!(
            line.graphemes(true)
                .all(|g| ["👩‍💻", "e\u{301}", "…"].contains(&g))
        );
    }
    let graph = Graph::new(&view);
    assert_eq!(
        graph.node("worker:t1").unwrap().height,
        3 + MAX_SUMMARY_LINES as u16
    );
    let worker = graph.node("worker:t1").unwrap();
    let reviewer = graph.node("reviewer:t1").unwrap();
    assert_eq!(reviewer.key, worker.key);
    assert_eq!(reviewer.group, worker.group);
    for (_, _, path) in graph.edge_routes(Relations::all()) {
        assert!(
            graph
                .nodes
                .iter()
                .filter(|n| n.visible)
                .all(|node| !Graph::route_intersects_card(&path, node))
        );
    }
}

#[test]
fn missing_or_cleaned_empty_summaries_use_short_legacy_fallbacks() {
    for summary in [None, Some(""), Some(" \n\t "), Some("\x1b[31m\x1b[0m")] {
        let mut row = task("t1", &[]);
        row.title = "Storage".into();
        row.goal = "A long and unrelated goal.".repeat(40);
        row.focus = "A long review specification.".repeat(40);
        row.summary = summary.map(str::to_owned);
        row.reviewer_summary = summary.map(str::to_owned);
        let view = view(vec![row]);
        assert_eq!(
            card_lines(
                &view,
                "worker:t1",
                usize::from(WIDTH - 4),
                MAX_SUMMARY_LINES
            ),
            ["Storage"]
        );
        assert_eq!(
            card_lines(
                &view,
                "reviewer:t1",
                usize::from(WIDTH - 4),
                MAX_SUMMARY_LINES
            ),
            ["Review Storage."]
        );
        assert!(Graph::new(&view).nodes.iter().all(|node| node.height <= 8));
    }
    let mut row = task("t1", &[]);
    row.title.clear();
    row.kind = "analysis".into();
    assert_eq!(
        card_lines(
            &view(vec![row.clone()]),
            "worker:t1",
            usize::from(WIDTH - 4),
            MAX_SUMMARY_LINES
        ),
        ["analysis"]
    );
    row.kind.clear();
    assert_eq!(
        card_lines(
            &view(vec![row]),
            "reviewer:t1",
            usize::from(WIDTH - 4),
            MAX_SUMMARY_LINES
        ),
        ["Review t1."]
    );
}

#[test]
fn summary_only_search_finds_tasks_and_admin_descriptions_stay_fixed() {
    let mut row = task("t1", &[]);
    row.summary = Some("Repair data storage.".into());
    row.reviewer_summary = Some("Validate roundtrips.".into());
    let mut view = view(vec![row]);
    for query in ["data storage", "roundtrips"] {
        let graph = Graph::new_with_options(
            &view,
            Some(query),
            &Default::default(),
            GroupingMode::Dependency,
        );
        assert!(graph.node("worker:t1").unwrap().visible);
        assert!(graph.node("reviewer:t1").unwrap().visible);
    }
    for (key, description) in [
        (
            ORCHESTRATOR,
            &["Agrees requirements with", "you and steers the team."][..],
        ),
        (
            PLAN_REVIEWER,
            &["Independently checks the", "plan before you see it."],
        ),
        (
            AUDITOR,
            &["Checks the finished work", "delivers your request."],
        ),
        (
            super::super::super::COMMENT_ORCHESTRATOR,
            &["Answers your comments and", "acts on them."],
        ),
        (
            super::super::super::PLANNER,
            &["Turns agreed requirements", "into a task plan."],
        ),
    ] {
        view.agents.insert(
            key.into(),
            super::super::view::Agent {
                running: true,
                activity: "exec a command".into(),
                ..Default::default()
            },
        );
        assert_eq!(
            card_lines(&view, key, usize::from(WIDTH - 4), ADMIN_SUMMARY_LINES)
                .iter()
                .map(|line| line.trim_end())
                .collect::<Vec<_>>(),
            description
        );
    }
}

#[test]
fn health_marks_problems_and_cards_have_no_frame() {
    let mut failed = task("t1", &[]);
    failed.status = "failed".into();
    let mut waiting = task("t2", &[]);
    waiting.status = "waiting".into();
    let mut working = task("t3", &[]);
    working.status = "working".into();
    let mut done = task("t4", &[]);
    done.status = "approved".into();
    let mut view = view(vec![failed, waiting, working, done]);
    assert_eq!(health(&view, "worker:t1"), Health::Problem);
    assert_eq!(health(&view, "reviewer:t1"), Health::Problem);
    assert_eq!(health(&view, "worker:t2"), Health::Waiting);
    assert_eq!(health(&view, "worker:t3"), Health::Working);
    // The reviewer waits until the worker hands it a round.
    assert_eq!(health(&view, "reviewer:t3"), Health::Idle);
    view.tasks[2].status = "reviewing".into();
    assert_eq!(health(&view, "worker:t3"), Health::Done);
    assert_eq!(health(&view, "reviewer:t3"), Health::Working);
    view.tasks[2].status = "working".into();
    assert_eq!(health(&view, "worker:t4"), Health::Done);
    // An orchestrator turn that failed is a problem until it runs again.
    view.agents.insert(
        ORCHESTRATOR.into(),
        super::super::view::Agent {
            activity: "✗ failed: boom".into(),
            ..Default::default()
        },
    );
    assert_eq!(health(&view, ORCHESTRATOR), Health::Problem);
    assert!(describe(&view, ORCHESTRATOR, &Theme::new(true, false)).contains("failed: boom"));

    let graph = Graph::new(&view);
    let mut canvas = Canvas::new(graph.width, graph.height);
    let theme = Theme::new(true, false);
    graph.render_with(&view, &mut canvas, Relations::all(), "worker:t1", &theme, 0);
    let text = canvas.lines().join("\n");
    for frame in ['╭', '╮', '╰', '╯', '╔', '╗', '╚', '╝', '║'] {
        assert!(!text.contains(frame), "{frame} in\n{text}");
    }
    assert!(text.contains(theme.glyphs().problem));
}

#[test]
fn card_progress_waits_at_top_bounces_while_working_and_finishes_at_bottom() {
    let mut row = task("t1", &[]);
    row.status = "working".into();
    row.round = 2;
    let mut view = view(vec![row]);
    let graph = Graph::new(&view);
    let node = graph.node("worker:t1").unwrap();
    for theme in [Theme::new(true, false), Theme::new(false, true)] {
        for selected in [false, true] {
            let fill = if selected {
                theme.raised()
            } else {
                theme.card()
            };
            let frame = |view: &View, tick| {
                let mut canvas = Canvas::new_window(node.x, node.y, node.width, node.height);
                draw_node(view, &mut canvas, node, selected, None, &theme, tick);
                canvas
            };
            let marker_rows = |canvas: &mut Canvas| {
                (0..node.height)
                    .filter(|row| {
                        canvas.cell(node.x, node.y + row).unwrap().style
                            == fill.patch(theme.progress_marker())
                    })
                    .collect::<Vec<_>>()
            };
            let last = node.height - 1;
            for (tick, expected) in [
                (0, 0),
                (1, 1),
                (last as u64, last),
                (last as u64 + 1, last - 1),
                (last as u64 * 2, 0),
            ] {
                let mut canvas = frame(&view, tick);
                assert_eq!(marker_rows(&mut canvas), vec![expected]);
                let cell = canvas.cell(node.x, node.y + expected).unwrap();
                assert_eq!(cell.style, fill.patch(theme.progress_marker()));
                assert_eq!(Canvas::glyph(cell), if theme.ascii { "#" } else { "█" });
                let text = canvas.lines().join("\n");
                assert!(text.contains("round 2"), "{text}");
                assert!(!text.contains("working"), "{text}");
                for spinner in theme.glyphs().spinner {
                    assert!(!canvas.lines()[1].contains(spinner));
                }
            }
            for status in ["proposed", "queued", "waiting"] {
                view.tasks[0].status = status.into();
                assert_eq!(frame(&view, 0).lines(), frame(&view, 7).lines());
                let mut queued = frame(&view, 0);
                assert_eq!(
                    Canvas::glyph(queued.cell(node.x, node.y).unwrap()),
                    if theme.ascii { "#" } else { "█" }
                );
                assert_eq!(marker_rows(&mut queued), vec![0], "{status}");
            }
            for status in [
                "reviewing",
                "approved",
                "failed",
                "skipped",
                "cancelled",
                "revised",
            ] {
                view.tasks[0].status = status.into();
                assert_eq!(frame(&view, 0).lines(), frame(&view, 7).lines());
                assert_eq!(marker_rows(&mut frame(&view, 0)), vec![last], "{status}");
            }
            view.tasks[0].status = "working".into();
        }
    }
}

#[test]
fn session_agent_marker_moves_from_top_to_bottom_after_its_turn() {
    let mut view = View::default();
    view.agents.insert(
        ORCHESTRATOR.into(),
        super::super::view::Agent {
            running: true,
            turns: 2,
            ..Default::default()
        },
    );
    let graph = Graph::new(&view);
    let node = graph.node(ORCHESTRATOR).unwrap();
    let theme = Theme::new(true, false);
    let frame = |view: &View| {
        let mut canvas = Canvas::new_window(node.x, node.y, node.width, node.height);
        draw_node(view, &mut canvas, node, false, None, &theme, 0);
        canvas
    };
    assert!(frame(&view).lines()[1].contains("working"));
    let agent = view.agents.get_mut(ORCHESTRATOR).unwrap();
    agent.running = false;
    agent.turns = 3;
    agent.activity = "✓ turn finished".into();
    for activity in ["✓ turn finished", ""] {
        view.agents.get_mut(ORCHESTRATOR).unwrap().activity = activity.into();
        let mut canvas = frame(&view);
        for row in 0..node.height {
            let style = canvas.cell(node.x, node.y + row).unwrap().style;
            assert_eq!(
                style == theme.card().patch(theme.progress_marker()),
                row == node.height - 1
            );
        }
    }
    view.agents.insert(ORCHESTRATOR.into(), Default::default());
    let mut canvas = frame(&view);
    for row in 0..node.height {
        let style = canvas.cell(node.x, node.y + row).unwrap().style;
        assert_eq!(
            style == theme.card().patch(theme.progress_marker()),
            row == 0
        );
    }
}

#[test]
fn compact_card_layout_is_identical_when_measuring_and_painting() {
    use serde_json::json;
    for width in [MIN_WIDTH, WIDTH] {
        for limit in [MIN_HEIGHT, HEIGHT] {
            for kind in ["implementation", "options"] {
                for status in ["working", "reviewing", "approved"] {
                    let mut row = task("t1", &[]);
                    row.kind = kind.into();
                    row.status = status.into();
                    row.summary = Some("café 👩‍👩‍👧‍👦 summary ".repeat(40));
                    row.effect = Some("Changes ".repeat(40));
                    row.obligations = Some(json!({"total":2,"ok":2}));
                    let view = view(vec![row]);
                    for key in ["worker:t1", "reviewer:t1"] {
                        for folded in [false, true] {
                            let measured = card_content::height(&view, key, width, limit, folded);
                            let painted = card_content::height(&view, key, width, measured, folded);
                            assert_eq!(measured, painted);
                            assert!(measured <= limit);
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn arrows_move_to_the_nearest_node_in_that_direction() {
    let view = view(vec![task("t1", &[]), task("t2", &["t1"]), task("t3", &[])]);
    let graph = Graph::new(&view);
    let go = |from: &str, dx, dy| graph.neighbour(from, dx, dy);
    let task_of =
        |key: Option<String>| key.and_then(|key| key.split_once(':').map(|(_, id)| id.to_owned()));
    // Across a frame, then on to the next one in the row.
    assert_eq!(go("worker:t1", 1, 0).as_deref(), Some("worker:t3"));
    assert_eq!(go("reviewer:t1", 1, 0).as_deref(), Some("worker:t3"));
    assert_eq!(go("worker:t3", -1, 0).as_deref(), Some("worker:t1"));
    assert_eq!(go("worker:t1", -1, 0), None);
    // Down to the dependent row, and back up.
    assert_eq!(task_of(go("reviewer:t1", 0, 1)).as_deref(), Some("t2"));
    assert!(matches!(
        task_of(go("worker:t2", 0, -1)).as_deref(),
        Some("t1" | "t3")
    ));
    assert!(go(ORCHESTRATOR, 0, 1).is_some_and(|key| key.contains(':')));
}

#[test]
fn a_dependency_arrow_enters_beside_the_title_in_every_frame_of_a_growing_card() {
    let mut child = task("t2", &["t1"]);
    child.title = "Add farewell".into();
    child.status = "approved".into();
    child.round = 1;
    let view = view(vec![task("t1", &[]), child]);
    // A narrow graph panel starts the expanded card barely wider than its
    // title, leaving two border columns before the corner.
    for (width, step) in [64, 70, 120]
        .into_iter()
        .flat_map(|width| (1..=super::model::FULL_STEP).map(move |step| (width, step)))
    {
        let graph = Graph::new_animated(
            &view,
            &Scope::default(),
            &Default::default(),
            GroupingMode::Dependency,
            Some((width, 30)),
            (Some("worker:t2".into()), 0),
            step,
        );
        let mut canvas = Canvas::new(graph.width, graph.height);
        graph.render(&view, &mut canvas, Relations::all(), "worker:t2");
        let frame = graph
            .frames
            .iter()
            .find(|frame| frame.task == "t2")
            .unwrap();
        let incoming: Vec<_> = graph
            .edge_routes(Relations::only(0))
            .into_iter()
            .filter(|(_, to, _)| *to == "worker:t2")
            .collect();
        assert_eq!(incoming.len(), 1);
        let &(x, y) = incoming[0].2.last().unwrap();
        assert_eq!(y + 1, frame.y);
        let row = canvas.lines()[usize::from(y)].chars().collect::<Vec<_>>();
        assert_eq!(row[usize::from(x)], '▼', "{width} step {step}");
    }
}

#[test]
fn crossings_get_box_drawing_joints() {
    let mut canvas = Canvas::new(5, 5);
    canvas.path(&[(0, 2), (4, 2)], Style::default());
    canvas.path(&[(2, 0), (2, 4)], Style::default());
    canvas.path(&[(0, 0), (1, 0), (1, 1)], Style::default());
    assert_eq!(canvas.lines(), ["─┐│", " ││", "──┼──", "  │", "  │"]);
}

fn scoped(query: &str) -> Scope {
    Scope {
        query: Some(query.to_owned()),
        ..Scope::default()
    }
}

/// The graph with every finished card unfolded, for tests of card slots.

#[test]
fn finished_cards_fold_to_their_identity_and_their_row_shrinks() {
    let mut view = view(vec![task("t1", &[]), task("t2", &["t1"])]);
    view.tasks[0].status = "approved".into();
    let graph = Graph::new_with_viewport(
        &view,
        &Scope::default(),
        &Default::default(),
        GroupingMode::Dependency,
        Some((40, 200)),
    );
    let (done, next) = (
        graph.node("worker:t1").unwrap(),
        graph.node("worker:t2").unwrap(),
    );
    assert!(done.collapsed && done.height == 4);
    assert!(!next.collapsed && next.height > 2);
    let (above, below) = (&graph.frames[0], &graph.frames[1]);
    assert_eq!(
        above.height, done.height,
        "the folded row takes only its height"
    );
    assert_eq!(below.y, above.y + above.height + GAP_Y);
    // z unfolds a finished card and folds a running one.
    let toggled = ["t1".to_owned(), "t2".to_owned()].into_iter().collect();
    let graph = Graph::new_with_viewport(
        &view,
        &Scope::default(),
        &toggled,
        GroupingMode::Dependency,
        Some((40, 200)),
    );
    assert!(!graph.node("worker:t1").unwrap().collapsed);
    assert!(graph.node("worker:t2").unwrap().collapsed);
}

#[test]
fn workflow_places_preparation_before_tasks_and_acceptance_after_them() {
    let mut view = view(vec![task("t1", &[])]);
    let preparation = [ORCHESTRATOR, crate::lean::PLANNER, PLAN_REVIEWER];
    let acceptance = [AUDITOR];
    for key in preparation.into_iter().chain(acceptance) {
        view.agents.insert(key.into(), Default::default());
    }
    for viewport in [None, Some((60, 30)), Some((180, 40))] {
        for query in [None, Some("audit")] {
            let graph = Graph::new_with_viewport(
                &view,
                &Scope {
                    query: query.map(str::to_owned),
                    ..Default::default()
                },
                &Default::default(),
                GroupingMode::Dependency,
                viewport,
            );
            let visible = |key| graph.node(key).filter(|node| node.visible);
            for sequence in [preparation.as_slice(), acceptance.as_slice()] {
                let nodes: Vec<_> = sequence.iter().filter_map(|key| visible(key)).collect();
                for pair in nodes.windows(2) {
                    assert!((pair[0].y, pair[0].x) <= (pair[1].y, pair[1].x));
                }
            }
            for frame in graph.frames.iter().filter(|frame| frame.visible) {
                for key in preparation {
                    if let Some(node) = visible(key) {
                        assert!(node.y + node.height <= frame.y);
                    }
                }
                for key in acceptance {
                    if let Some(node) = visible(key) {
                        assert!(node.y >= frame.y + frame.height);
                    }
                }
            }
            let shown: Vec<_> = graph.nodes.iter().filter(|node| node.visible).collect();
            for (index, a) in shown.iter().enumerate() {
                for b in &shown[index + 1..] {
                    assert!(
                        a.right() <= b.x
                            || b.right() <= a.x
                            || a.y + a.height <= b.y
                            || b.y + b.height <= a.y
                    );
                }
            }
        }
    }
}

#[test]
fn an_expanded_card_spans_a_narrow_viewport_past_the_lanes_that_end_above_it() {
    let mut rows = vec![
        task("t1", &[]),
        task("t2", &["t1"]),
        task("t3", &["t1"]),
        task("t4", &["t2", "t3"]),
    ];
    rows[3].title = "Handle auditor reports before workflow".into();
    rows[3].reviewer_summary = Some("Review audit".into());
    let view = view(rows);
    let graph = Graph::new_with_expanded(
        &view,
        &Scope::default(),
        &Default::default(),
        GroupingMode::Dependency,
        Some((70, 40)),
        Some("reviewer:t4".into()),
        0,
    );
    // t1 → t3 skips t2's row in a lane left of the rows; it ends above t4,
    // so t4's card keeps the viewport's width, `MARGIN` either side.
    assert!(graph.edge_routes(Relations::all()).len() > 1);
    let frame = graph
        .frames
        .iter()
        .find(|frame| frame.task == "t4")
        .unwrap();
    assert_eq!((frame.x, frame.x + frame.width), (1, 69));
    assert_eq!(graph.width, 70, "no horizontal scroll");
}

#[test]
fn a_route_stopping_under_its_target_points_up_into_it() {
    let mut graph = Graph::new(&View::default());
    let card = |key: &str, y: u16| super::Node {
        key: key.into(),
        x: 6,
        y,
        width: 20,
        height: 4,
        visible: true,
        ..Default::default()
    };
    graph.nodes = vec![card("orchestrator", 0), card("auditor", 26)];
    // The auditor's detour: up the left margin, along under the card, and a
    // repeated last point, as the layout lays it.
    graph.edges = vec![super::Edge {
        from: "auditor".into(),
        to: "orchestrator".into(),
        kind: super::Kind::Reviews,
        label: None,
        path: Some(vec![(16, 30), (2, 30), (2, 4), (12, 4), (12, 4)]),
    }];
    let mut canvas = Canvas::new(30, 32);
    graph.render(&View::default(), &mut canvas, Relations::all(), "");
    let row = canvas.lines()[4].chars().collect::<Vec<char>>();
    assert_eq!(row[12], '▲', "{}", canvas.lines()[4]);
}

#[test]
fn a_detour_turns_out_from_under_its_card_with_its_label_in_sight() {
    let mut view = view(vec![task("t1", &[]), task("t2", &["t1"])]);
    for key in [crate::lean::PLANNER, "planner", PLAN_REVIEWER, AUDITOR] {
        view.agents.insert(key.into(), Default::default());
    }
    let graph = Graph::new_with_viewport(
        &view,
        &Scope::default(),
        &Default::default(),
        GroupingMode::Dependency,
        Some((70, 60)),
    );
    let auditor = graph.node(AUDITOR).unwrap();
    let edge = graph
        .edges
        .iter()
        .find(|edge| edge.from == AUDITOR && edge.to == ORCHESTRATOR)
        .unwrap();
    let path = edge.path.as_ref().unwrap();
    let mut canvas = Canvas::new(graph.width, graph.height + 2);
    graph.render(&view, &mut canvas, Relations::all(), "");
    let lines = canvas.lines();
    // The route leaves the card downwards: a corner under the card, and the
    // label beside it rather than under the card.
    let under = lines[usize::from(auditor.y + auditor.height)..].join("\n");
    assert_eq!(path[0].1, auditor.y + auditor.height, "{path:?}");
    assert!(under.contains('R'), "{}", lines.join("\n"));
}

#[test]
fn planning_specialist_card_uses_its_role_title() {
    let mut view = View::default();
    let key = "planning-analysis:181b2d5b-8ff1-4526-8b5f-649f09e2672c";
    view.agents.insert(key.into(), Default::default());
    let graph = Graph::new_with_viewport(
        &view,
        &Scope::default(),
        &Default::default(),
        GroupingMode::Dependency,
        Some((100, 40)),
    );
    assert!(graph.node(key).is_some());
    let mut canvas = Canvas::new(graph.width, graph.height);
    graph.render(&view, &mut canvas, Relations::all(), key);
    let shown = canvas.lines().join("\n");
    assert!(shown.contains("Feature Impact Analysis"), "{shown}");
    assert!(!shown.contains("planning-analysis"), "{shown}");
}

#[test]
fn rows_stay_centred_and_lanes_on_either_side_stay_within_the_viewport() {
    // t1 → t2 → t3 → t4 is a chain; t5 starts a second one, and t6 waits
    // for both chains' ends, t7 for a root and the chain's middle.
    let view = view(vec![
        task("t1", &[]),
        task("t2", &["t1"]),
        task("t3", &["t2"]),
        task("t4", &["t3"]),
        task("t5", &[]),
        task("t6", &["t4", "t5"]),
        task("t7", &["t5", "t2"]),
        task("t8", &["t1", "t5"]),
    ]);
    for width in [70u16, 100, 160, 240] {
        let graph = Graph::new_with_viewport(
            &view,
            &Scope::default(),
            &Default::default(),
            GroupingMode::Dependency,
            Some((width, 40)),
        );
        let mut canvas = Canvas::new(graph.width, graph.height);
        graph.render(&view, &mut canvas, Relations::all(), "");
        let lines = canvas.lines();
        let shown = lines.join("\n");
        assert_eq!(graph.width, width, "no horizontal scroll:\n{shown}");
        // Each row of frames is centred: even margins, give or take one.
        let mut rows = std::collections::BTreeMap::<u16, (u16, u16)>::new();
        for frame in graph.frames.iter().filter(|frame| frame.visible) {
            let row = rows.entry(frame.y).or_insert((u16::MAX, 0));
            *row = (row.0.min(frame.x), row.1.max(frame.x + frame.width));
        }
        for (left, right) in rows.values() {
            assert!(left.abs_diff(width - right) <= 1, "{width}:\n{shown}");
        }
        // Every link stays on screen, outside the cards, and the links into
        // one task end in a single arrow on its frame.
        let cards: Vec<_> = graph.nodes.iter().filter(|node| node.visible).collect();
        let routes = graph.edge_routes(Relations::only(0));
        for (from, to, path) in &routes {
            assert!(path.iter().all(|(x, _)| *x < width), "{from}→{to} {path:?}");
            for card in &cards {
                assert!(!Graph::route_intersects_card(path, card), "{from}→{to}");
            }
        }
        for frame in graph.frames.iter().filter(|frame| frame.visible) {
            let into = format!("task:{}", frame.task);
            let ends: std::collections::HashSet<_> = routes
                .iter()
                .filter(|(_, to, _)| *to == into)
                .map(|(_, _, path)| *path.last().unwrap())
                .collect();
            assert!(ends.len() <= 1, "{into} has arrows {ends:?}:\n{shown}");
            let arrows = lines[usize::from(frame.y)]
                .chars()
                .skip(usize::from(frame.x))
                .take(usize::from(frame.width))
                .filter(|ch| *ch == '▼')
                .count();
            assert_eq!(arrows, ends.len(), "{into}:\n{shown}");
        }
        // The links skipping rows run in lanes on both sides of a single
        // column of tasks; beside two columns, on the side their ends stand.
        let lanes: std::collections::HashSet<_> = routes
            .iter()
            .filter(|(_, _, path)| path.len() > 4)
            .map(|(_, _, path)| path[2].0 < width / 2)
            .collect();
        let single = graph
            .frames
            .iter()
            .all(|frame| frame.x == graph.frames[0].x);
        assert_eq!(lanes.len(), if single { 2 } else { 1 }, "{width}:\n{shown}");
    }
}

#[test]
fn grouped_projection_uses_explicit_unique_ownership_and_keeps_every_stage() {
    use crate::lean::tui::work_units::{WorkUnitId, WorkUnits};
    let mut dedicated = task("dedicated", &[]);
    dedicated.kind = "options".into();
    dedicated.reviewer_summary = Some("Review options".into());
    let mut shared = task("shared", &[]);
    shared.kind = "options".into();
    let mut owner = task("build", &["dedicated", "shared"]);
    owner.reviewer_summary = Some("Review implementation".into());
    let mut view = view(vec![dedicated, shared, owner, task("other", &["shared"])]);
    view.task_meta
        .entry("dedicated".into())
        .or_default()
        .options_for = Some("build".into());
    view.task_meta
        .entry("shared".into())
        .or_default()
        .options_for = Some("build".into());
    for key in [
        ORCHESTRATOR,
        crate::lean::COMMENT_ORCHESTRATOR,
        crate::lean::PLANNER,
        PLAN_REVIEWER,
        AUDITOR,
        "research-alpha",
        "research-beta",
    ] {
        view.agents.insert(key.into(), Default::default());
    }
    let work = WorkUnits::new(&view);
    let build = work.get(&WorkUnitId::Task("build".into())).unwrap();
    assert_eq!(
        build
            .stages
            .iter()
            .map(|stage| stage.id.0.as_str())
            .collect::<Vec<_>>(),
        [
            "worker:dedicated",
            "reviewer:dedicated",
            "worker:build",
            "reviewer:build"
        ]
    );
    assert_eq!(work.for_stage("reviewer:dedicated").unwrap().id, build.id);
    assert_ne!(work.for_stage("worker:shared").unwrap().id, build.id);
    assert_ne!(
        work.for_stage("research-alpha").unwrap().id,
        work.for_stage("research-beta").unwrap().id
    );
    assert_eq!(
        work.for_stage(crate::lean::COMMENT_ORCHESTRATOR)
            .unwrap()
            .id,
        WorkUnitId::Orchestration
    );
    assert_eq!(
        work.for_stage(PLAN_REVIEWER).unwrap().id,
        WorkUnitId::Planning
    );
    assert_eq!(
        work.for_stage(crate::lean::PLANNER).unwrap().id,
        WorkUnitId::Planning
    );
    assert_eq!(
        work.for_stage("worker:other").unwrap().stages.len(),
        1,
        "no invented reviewer"
    );
    let graph = Graph::new(&view);
    assert_eq!(
        graph.nodes.iter().filter(|node| node.visible).count(),
        work.units.len()
    );
    assert_eq!(graph.node("worker:dedicated"), graph.node("reviewer:build"));
    assert_eq!(graph.node(PLAN_REVIEWER), graph.node(crate::lean::PLANNER));
    view.task_meta.get_mut("dedicated").unwrap().options_for = None;
    assert_eq!(
        WorkUnits::new(&view)
            .for_stage("worker:dedicated")
            .unwrap()
            .id,
        WorkUnitId::Task("dedicated".into()),
        "dependency/name alone must not establish ownership"
    );
}

#[test]
fn grouped_stages_keep_retry_identity_and_member_problems_decisions_and_filters() {
    use crate::lean::tui::work_units::WorkUnits;
    let mut task = task("t1", &[]);
    task.status = "reviewing".into();
    task.reviewer_summary = Some("Check contracts".into());
    let mut view = view(vec![task]);
    view.agents.insert(
        "worker:t1".into(),
        super::super::view::Agent {
            activity: "✗ raw worker failure".into(),
            provider: "codex".into(),
            ..Default::default()
        },
    );
    view.agents.insert(
        "reviewer:t1".into(),
        super::super::view::Agent {
            running: true,
            provider: "claude".into(),
            ..Default::default()
        },
    );
    let before = WorkUnits::new(&view);
    let unit = before.for_stage("worker:t1").unwrap();
    assert_eq!(unit.health(&view), Health::Problem);
    assert_eq!(unit.active_stage(&view).id.0, "reviewer:t1");
    view.tasks[0].round = 5;
    let after = WorkUnits::new(&view);
    assert_eq!(unit.id, after.for_stage("reviewer:t1").unwrap().id);
    view.approvals.push(super::super::view::PendingApproval {
        request: "approval".into(),
        role: "reviewer:t1".into(),
        provider: "codex".into(),
        summary: "Approve command".into(),
        orchestrator: false,
        options: Vec::new(),
    });
    assert_eq!(unit.decisions(&view), 1);
    for scope in [
        Scope {
            query: Some("reviewer:t1".into()),
            ..Default::default()
        },
        Scope {
            providers: ["claude".to_owned()].into(),
            ..Default::default()
        },
        Scope {
            states: [Health::Problem].into(),
            ..Default::default()
        },
    ] {
        let graph = Graph::new_with_viewport(
            &view,
            &scope,
            &Default::default(),
            GroupingMode::Dependency,
            Some((80, 24)),
        );
        assert!(
            graph.node("worker:t1").unwrap().visible,
            "any matching stage exposes its work card"
        );
    }
}

#[test]
fn external_mail_routes_between_groups_and_internal_mail_keeps_exact_endpoints() {
    let mut first = task("t1", &[]);
    first.reviewer_summary = Some("Review".into());
    let mut view = view(vec![first, task("t2", &["t1"])]);
    for (from, to) in [("worker:t1", "reviewer:t1"), ("reviewer:t1", "worker:t2")] {
        view.links.insert(
            (from.into(), to.into()),
            super::super::view::Link {
                count: 2,
                last: std::time::Instant::now(),
            },
        );
    }
    let graph = Graph::new(&view);
    let mail: Vec<_> = graph
        .edges
        .iter()
        .filter(|edge| matches!(edge.kind, Kind::Mail { .. }))
        .collect();
    assert_eq!(mail.len(), 1);
    assert_eq!(
        (mail[0].from.as_str(), mail[0].to.as_str()),
        ("worker:t1", "worker:t2")
    );
    assert!(
        view.links
            .contains_key(&("worker:t1".into(), "reviewer:t1".into()))
    );
    for (_, _, path) in graph.edge_routes(Relations::all()) {
        assert!(
            graph
                .nodes
                .iter()
                .filter(|node| node.visible)
                .all(|node| !Graph::route_intersects_card(&path, node)),
            "external routes must not penetrate cards: {path:?}"
        );
    }
}

#[test]
fn grouped_cards_render_complete_graphemes_and_aggregate_state_in_all_modes() {
    let mut row = task("t1", &[]);
    row.title = "Review 👩‍💻 e\u{301} changes".into();
    row.status = "reviewing".into();
    row.reviewer_summary = Some("Review changes".into());
    let view = view(vec![row]);
    for ascii in [false, true] {
        for color in [false, true] {
            let theme = Theme::new(color, ascii);
            let graph = Graph::new_with_viewport(
                &view,
                &Scope::default(),
                &Default::default(),
                GroupingMode::Dependency,
                Some((80, 24)),
            );
            let node = graph.node("worker:t1").unwrap();
            let mut canvas = Canvas::new_window(node.x, node.y, node.width, node.height);
            graph.render_with(
                &view,
                &mut canvas,
                Relations::all(),
                "reviewer:t1",
                &theme,
                0,
            );
            let text = canvas.lines().join("\n");
            assert!(text.contains("👩‍💻") && text.contains("e\u{301}"), "{text}");
            assert!(text.contains("worker") && text.contains("reviewer"));
            assert!(text.contains("working"));
            if !color {
                assert!(
                    canvas
                        .cells
                        .iter()
                        .all(|cell| cell.style.fg.is_none() && cell.style.bg.is_none())
                );
            }
        }
    }
}
