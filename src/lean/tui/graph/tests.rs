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
fn expanded_pty_content_grows_with_the_viewport() {
    let viewport = (76, 12);
    let size = Size::fit(Some(viewport), Some("worker:t1".into()));
    let content_area = |width: u16, height: u16| usize::from(width - 4) * usize::from(height - 4);

    // Before the viewport-fit change the 56-column ceiling produced a
    // 52x6 PTY content area at this same viewport.
    assert_eq!(content_area(56, 10), 312);
    assert_eq!((size.expanded_w, size.expanded_h), (70, 10));
    assert_eq!(content_area(size.expanded_w, size.expanded_h), 396);
    assert!(content_area(size.expanded_w, size.expanded_h) > content_area(56, 10));
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
        assert!(node.width < WIDTH && node.height < HEIGHT, "{node:?}");
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
                node.y
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
    assert!(
        compact_canvas
            .lines()
            .join("\n")
            .contains("Combining e\u{301}")
    );
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
                    "terminal={width}x{height} edge {from} -> {to} crosses {}",
                    node.key
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
}

#[test]
fn agent_card_shows_recorded_branch() {
    let mut view = View::default();
    view.agents.insert(
        ORCHESTRATOR.into(),
        super::super::view::Agent {
            branch: Some("feature/tasks".into()),
            ..Default::default()
        },
    );
    let graph = Graph::new(&view);
    let node = graph.node(ORCHESTRATOR).unwrap();
    let mut canvas = Canvas::new_window(node.x, node.y, WIDTH, HEIGHT);
    graph.render(&view, &mut canvas, Relations::all(), ORCHESTRATOR);
    assert!(canvas.lines().join("\n").contains("branch: feature/tasks"));
}

#[test]
fn requirements_rounds_share_one_card_that_fits() {
    let mut view = View::default();
    for key in REQUIREMENTS_ROUNDS {
        view.agents.insert(key.into(), Default::default());
    }
    let graph = Graph::new_with_viewport(
        &view,
        &Scope::default(),
        &Default::default(),
        GroupingMode::Dependency,
        None,
    );
    let cards: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| node.visible && requirements_round(&node.key).is_some())
        .collect();
    assert_eq!(cards.len(), 1);
    for line in card_lines(
        &view,
        &cards[0].key,
        usize::from(WIDTH - 4),
        MAX_SUMMARY_LINES,
    ) {
        assert!(line.width() <= usize::from(WIDTH - 4), "{line}");
    }
}

#[test]
fn graph_includes_planning_agents_in_every_layout() {
    let mut view = View::default();
    for key in ["planner", "options-planner", "requirements-planner"] {
        view.agents.insert(key.into(), Default::default());
    }
    for viewport in [None, Some((60, 30)), Some((180, 40))] {
        let graph = Graph::new_with_viewport(
            &view,
            &Scope::default(),
            &Default::default(),
            GroupingMode::Dependency,
            viewport,
        );
        for key in ["planner", "options-planner", "requirements-planner"] {
            let node = graph.node(key).unwrap();
            assert!(node.visible);
            assert!(
                graph
                    .edges
                    .iter()
                    .any(|edge| { edge.from == ORCHESTRATOR && edge.to == key })
            );
            assert!(
                graph
                    .nodes
                    .iter()
                    .filter(|other| other.visible && other.key != key)
                    .all(|other| other.x != node.x || other.y != node.y)
            );
        }
    }
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
fn a_tasks_agents_have_cards_of_their_own_in_one_frame_even_when_collapsed_or_filtered() {
    let mut row = task("t1", &[]);
    row.status = "reviewing".into();
    row.round = 2;
    row.summary = Some("Execute storage migration.".into());
    row.reviewer_summary = Some("Validate roundtrips.".into());
    let mut view = view(vec![row, task("t2", &["t1"])]);
    view.links.insert(
        ("reviewer:t1".into(), "worker:t2".into()),
        super::super::view::Link {
            count: 1,
            last: std::time::Instant::now(),
        },
    );
    view.links.insert(
        ("reviewer:t1".into(), "worker:t1".into()),
        super::super::view::Link {
            count: 1,
            last: std::time::Instant::now(),
        },
    );
    view.links.insert(
        ("worker:t1".into(), "reviewer:t2".into()),
        super::super::view::Link {
            count: 2,
            last: std::time::Instant::now() - RECENT * 2,
        },
    );
    let collapsed = std::collections::HashSet::from(["t1".to_owned()]);
    for grouping in [GroupingMode::Dependency, GroupingMode::Status] {
        for viewport in [None, Some((78, 20))] {
            let graph =
                Graph::new_with_viewport(&view, &Scope::default(), &collapsed, grouping, viewport);
            assert_eq!(graph.nodes.len(), 7);
            let worker = graph.node("worker:t1").unwrap();
            let node = graph.node("reviewer:t1").unwrap();
            assert_eq!(node.key, "reviewer:t1");
            assert_eq!(node.group, worker.group);
            assert!(node.visible && node.collapsed && worker.collapsed);
            // Messages run between the cards of the agents that sent them.
            let mail: Vec<_> = graph
                .edges
                .iter()
                .filter(|edge| matches!(edge.kind, Kind::Mail { .. }))
                .map(|edge| {
                    (
                        edge.from.as_str(),
                        edge.to.as_str(),
                        edge.label.as_deref().unwrap_or_default(),
                        edge.kind,
                    )
                })
                .collect();
            assert!(mail.contains(&(
                "reviewer:t1",
                "worker:t2",
                "M✉1",
                Kind::Mail { recent: true }
            )));
            assert!(mail.contains(&(
                "worker:t1",
                "reviewer:t2",
                "M✉2",
                Kind::Mail { recent: false }
            )));
            assert_eq!(graph.edge_routes(Relations::only(2)).len(), mail.len());
            let mut canvas = Canvas::new_window(node.x, node.y, WIDTH, HEIGHT);
            graph.render(&view, &mut canvas, Relations::all(), "reviewer:t1");
            let text = canvas.lines().join("\n");
            assert!(
                text.contains("reviewer") && text.contains("round 2"),
                "{text}"
            );
            assert!(!text.contains("Execute") && !text.contains("Validate"));
            for query in ["reviewer:t1", "roundtrips", "checking r2"] {
                let filtered =
                    Graph::new_with_viewport(&view, &scoped(query), &collapsed, grouping, viewport);
                assert!(filtered.node("worker:t1").unwrap().visible);
                assert!(!filtered.node("worker:t2").unwrap().visible);
            }
        }
    }
    view.tasks[0].status = "failed".into();
    view.agents.insert(
        "worker:t1".into(),
        super::super::view::Agent {
            running: true,
            ..Default::default()
        },
    );
    assert_eq!(health(&view, "worker:t1"), Health::Problem);
    view.tasks[0].status = "reviewing".into();
    for role in ["worker", "reviewer"] {
        view.agents.clear();
        view.agents.insert(
            format!("{role}:t1"),
            super::super::view::Agent {
                activity: "✗ process exited".into(),
                ..Default::default()
            },
        );
        assert_eq!(health(&view, &format!("{role}:t1")), Health::Problem);
    }
}

#[test]
fn names_and_work_kinds_remain_visible_on_collapsed_cards() {
    let mut task = task("t1", &[]);
    task.title = "Repair storage".into();
    task.kind = "implementation".into();
    let view = view(vec![task]);
    for collapsed in [
        Default::default(),
        std::collections::HashSet::from(["t1".into()]),
    ] {
        let graph = Graph::new_with_options(&view, None, &collapsed, GroupingMode::Dependency);
        for node in graph.nodes.iter().filter(|node| node.visible) {
            let mut canvas = Canvas::new_window(node.x, node.y, WIDTH, node.height);
            graph.render(&view, &mut canvas, Relations::all(), "worker:t1");
            let rows = canvas.lines();
            match node.key.as_str() {
                "worker:t1" => {
                    assert!(rows[0].contains("worker"), "{rows:?}");
                    assert!(rows[0].contains("worker - impl"), "{rows:?}");
                }
                "reviewer:t1" => {
                    assert!(rows[0].contains("reviewer"), "{rows:?}");
                    assert!(!rows[0].contains("reviewer -"), "{rows:?}");
                }
                _ => assert!(!rows.join(" ").contains("orchestration"), "{rows:?}"),
            }
        }
        // The task's name is the title of its frame.
        let frame = &graph.frames[0];
        let mut canvas = Canvas::new_window(frame.x, frame.y, frame.width, 1);
        graph.render(&view, &mut canvas, Relations::all(), "worker:t1");
        assert!(canvas.lines()[0].contains("Repair storage"));
    }
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
    assert_ne!(reviewer.key, worker.key);
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
        assert!(Graph::new(&view).nodes.iter().all(|node| node.height <= 4));
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
        (
            "requirements-extraction-analyst",
            &["Records requirements from", "your request and documents."],
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
    assert!(describe(&view, ORCHESTRATOR).contains("failed: boom"));

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
fn a_task_is_a_framed_unit_and_units_stand_in_rows_by_dependency_depth() {
    let mut options = task("t2-options", &["t1"]);
    options.kind = "options".into();
    let view = view(vec![
        task("t1", &[]),
        task("t2", &["t1", "t2-options"]),
        task("t3", &[]),
        options,
    ]);
    let graph = Graph::new(&view);
    let node = |key: &str| graph.node(key).unwrap();
    let frame = |id: &str| graph.frames.iter().find(|frame| frame.task == id).unwrap();
    // Worker and reviewer stand side by side in their task's frame.
    let (worker, reviewer) = (node("worker:t1"), node("reviewer:t1"));
    assert_eq!(worker.y, reviewer.y);
    assert_eq!(worker.right() + GAP_X, reviewer.x);
    assert_eq!(worker.group.as_deref(), Some("task:t1"));
    let t1 = frame("t1");
    assert!(t1.x < worker.x && reviewer.right() < t1.x + t1.width);
    // Independent tasks share a row; a dependent one stands below them,
    // its options card first in its frame.
    assert_eq!(frame("t3").y, t1.y);
    let t2 = frame("t2");
    assert!(t2.y >= t1.y + t1.height + GAP_Y);
    assert_eq!(node("worker:t2-options").group.as_deref(), Some("task:t2"));
    assert_eq!(
        node("worker:t2-options").right() + GAP_X,
        node("worker:t2").x
    );
    assert!(graph.frames.iter().all(|frame| frame.task != "t2-options"));
    assert!(
        graph
            .node("reviewer:t2-options")
            .is_some_and(|card| card.key == "worker:t2-options")
    );
    // One dependency runs from frame to frame, the options task's own copy
    // of it folded into its unit.
    let between: Vec<_> = graph
        .edge_routes(Relations::only(0))
        .into_iter()
        .filter(|(from, _, _)| from.starts_with("task:"))
        .collect();
    assert_eq!(between.len(), 1);
    let (from, to, path) = &between[0];
    assert_eq!((*from, *to), ("task:t1", "task:t2"));
    assert_eq!(path.first().unwrap().1, t1.y + t1.height - 1);
    assert_eq!(path.last().unwrap().1, t2.y);
    assert!(admin_above(&graph, t1.y));
}

fn admin_above(graph: &Graph, top: u16) -> bool {
    graph
        .nodes
        .iter()
        .filter(|node| node.visible && is_admin(&node.key) && !is_final_review(&node.key))
        .all(|node| node.y + node.height < top)
}

#[test]
fn cards_fit_content_and_frames_and_routes_follow_height_changes() {
    use super::super::view::Agent;
    use serde_json::json;
    let mut row = task("t1", &[]);
    row.summary = Some("Short summary.".into());
    let mut view = view(vec![row, task("t2", &["t1"])]);
    let render = |view: &View| {
        let graph = open_cards(view);
        let node = graph.node("worker:t1").unwrap();
        let mut canvas = Canvas::new_window(node.x, node.y, node.width, node.height);
        graph.render(view, &mut canvas, Relations::all(), "worker:t1");
        (graph, canvas.lines())
    };
    let (short, rows) = render(&view);
    assert_eq!(rows.len(), 4, "title, status, summary, metadata: {rows:?}");
    assert!(
        rows.iter()
            .all(|row| !row.trim_matches(['▌', '█', ' ']).is_empty()),
        "{rows:?}"
    );
    view.tasks[0].status = "working".into();
    view.agents.insert(
        "worker:t1".into(),
        Agent {
            running: true,
            activity: "cargo check".into(),
            branch: Some("task/repair".into()),
            ..Default::default()
        },
    );
    let (_, working) = render(&view);
    assert_eq!(working.len(), rows.len() + 2);
    assert!(working[2].contains("doing: cargo check"));
    assert!(working[3].contains("branch: task/repair"));
    view.tasks[0].status = "approved".into();
    view.tasks[0].summary = Some("Long café 👩‍💻 summary. ".repeat(40));
    view.tasks[0].effect = Some("Updated storage".into());
    view.tasks[0].integration = Some(json!({"outcome":"committed","detail":"as abc12"}));
    let (long, rows) = render(&view);
    assert!(long.node("worker:t1").unwrap().height > short.node("worker:t1").unwrap().height);
    assert!(rows.join(" ").contains('…') && rows.join(" ").contains("effect: Updated storage"));
    assert!(rows.last().unwrap().contains("committed"));
    for graph in [short, long] {
        assert!(graph.nodes.iter().all(|node| node.height <= HEIGHT));
        for frame in &graph.frames {
            let tallest = graph
                .nodes
                .iter()
                .filter(|node| node.group.as_deref() == Some(&frame.key))
                .map(|node| node.height)
                .max()
                .unwrap();
            assert_eq!(frame.height, tallest + 2);
        }
        for (_, _, path) in graph.edge_routes(Relations::all()) {
            assert!(
                graph
                    .nodes
                    .iter()
                    .filter(|node| node.visible)
                    .all(|node| !Graph::route_intersects_card(&path, node))
            );
        }
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
    assert_eq!(go("worker:t1", 1, 0).as_deref(), Some("reviewer:t1"));
    assert_eq!(go("reviewer:t1", 1, 0).as_deref(), Some("worker:t3"));
    assert_eq!(go("worker:t3", -1, 0).as_deref(), Some("reviewer:t1"));
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
fn edges_join_boxes_without_crossing_them() {
    let mut view = view(vec![task("t1", &[]), task("t2", &["t1"])]);
    view.links.insert(
        ("worker:t1".into(), "worker:t2".into()),
        super::super::view::Link {
            count: 2,
            last: std::time::Instant::now(),
        },
    );
    let graph = Graph::new(&view);
    let mut canvas = Canvas::new(graph.width, graph.height);
    graph.render(&view, &mut canvas, Relations::all(), "worker:t2");
    let lines = canvas.lines();
    let chars = |y: u16| lines[usize::from(y)].chars().collect::<Vec<char>>();
    let worker = graph.node("worker:t1").unwrap();
    let reviewer = graph.node("reviewer:t1").unwrap();
    let next = graph.node("worker:t2").unwrap();
    // Worker → reviewer straight across the gap in their frame.
    let row = chars(worker.y + 1);
    assert_eq!(row[usize::from(reviewer.x - 1)], '▶', "{row:?}");
    assert_eq!(row[usize::from(worker.right())], '─', "{row:?}");
    assert_eq!(
        chars(next.y)[usize::from(next.x)],
        '█',
        "the selected card's bar"
    );
    // t1's frame → t2's frame, ending in an arrow on t2's top border.
    let (top, bottom) = (&graph.frames[1], &graph.frames[0]);
    assert_eq!((bottom.task.as_str(), top.task.as_str()), ("t1", "t2"));
    assert!(
        chars(top.y).contains(&'▼'),
        "{:?}",
        lines[usize::from(top.y)]
    );
    assert!(chars(bottom.y + bottom.height - 1).contains(&'┬'));
    assert!(lines[usize::from(bottom.y)].contains("Task t1"));
    // A message between them is counted beside the source.
    assert!(lines.join("\n").contains("✉2"));
    assert!(lines[usize::from(worker.y + 1)].contains("proposed"));
}

#[test]
fn all_includes_every_supervision_edge() {
    let mut view = view(vec![task("t1", &[]), task("t2", &["t1"])]);
    view.agents
        .insert(super::super::super::PLANNER.into(), Default::default());
    let graph = Graph::new(&view);
    let all = graph.edge_routes(Relations::all());
    // The orchestrator supervises the session's agents; tasks stand in
    // frames of their own.
    let supervision = graph.edge_routes(Relations::only(3));
    assert_eq!(supervision.len(), 1);
    for edge in &supervision {
        assert!(all.contains(edge), "All must include {edge:?}");
    }
    assert!(
        graph
            .edge_routes(Relations::only(0))
            .iter()
            .all(|edge| !supervision.contains(edge))
    );
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
fn open_cards(view: &View) -> Graph {
    let toggled = view
        .tasks
        .iter()
        .filter(|task| {
            matches!(
                task.status.as_str(),
                "approved" | "skipped" | "cancelled" | "revised"
            )
        })
        .map(|task| task.id.clone())
        .collect();
    Graph::new_with_options(view, None, &toggled, GroupingMode::Dependency)
}

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
    assert!(done.collapsed && done.height == 2);
    assert!(!next.collapsed && next.height > 2);
    let (above, below) = (&graph.frames[0], &graph.frames[1]);
    assert_eq!(
        above.height,
        done.height + 2,
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
fn options_card_finishes_with_one_result_line_and_no_cli() {
    use super::super::view::Agent;
    use serde_json::json;
    let mut row = task("t1", &[]);
    row.kind = "options".into();
    row.status = "approved".into();
    row.round = 2;
    row.obligations = Some(json!({"total":21,"ok":21,"missing":0}));
    let mut view = view(vec![row]);
    for role in ["worker", "reviewer"] {
        view.agents.insert(
            format!("{role}:t1"),
            Agent {
                provider: "codex".into(),
                ..Default::default()
            },
        );
    }
    for collapsed in [
        Default::default(),
        std::collections::HashSet::from(["t1".into()]),
    ] {
        let graph = Graph::new_with_options(&view, None, &collapsed, GroupingMode::Dependency);
        let node = graph.node("worker:t1").unwrap();
        let mut canvas = Canvas::new_window(node.x, node.y, node.width, node.height);
        graph.render(&view, &mut canvas, Relations::all(), "worker:t1");
        let rows = canvas.lines();
        assert!(
            rows[0].contains("options") && !rows[0].contains("options -"),
            "{rows:?}"
        );
        assert!(
            rows[1].contains("approved r2") && rows[1].contains("21/21"),
            "{rows:?}"
        );
        assert_eq!(
            rows.iter().filter(|line| line.contains("approved")).count(),
            1,
            "{rows:?}"
        );
        assert!(
            !rows.join(" ").contains("codex") && !rows.join(" ").contains("V:"),
            "{rows:?}"
        );
    }
}

#[test]
fn reviewer_marker_waits_for_worker_before_animating_and_finishing() {
    let mut row = task("t1", &[]);
    row.status = "working".into();
    let mut view = view(vec![row]);
    let graph = Graph::new(&view);
    let node = graph.node("reviewer:t1").unwrap();
    let theme = Theme::new(true, false);
    for (status, expected) in [
        ("working", 0),
        ("reviewing", 1),
        ("approved", node.height - 1),
    ] {
        view.tasks[0].status = status.into();
        let mut canvas = Canvas::new_window(node.x, node.y, node.width, node.height);
        draw_node(&view, &mut canvas, node, false, None, &theme, 1);
        let markers: Vec<_> = (0..node.height)
            .filter(|row| {
                canvas.cell(node.x, node.y + row).unwrap().style
                    == theme.card().patch(theme.progress_marker())
            })
            .collect();
        assert_eq!(markers, [expected], "{status}");
    }
}

#[test]
fn workflow_places_preparation_before_tasks_and_acceptance_after_them() {
    let mut view = view(vec![task("t1", &[])]);
    let preparation = [
        ORCHESTRATOR,
        REQUIREMENTS_ROUNDS[0],
        crate::lean::PLANNER,
        PLAN_REVIEWER,
    ];
    let acceptance = [
        "requirements-audit-analyst",
        "requirements-audit-reviewer",
        AUDITOR,
    ];
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
                    assert!((pair[0].y, pair[0].x) < (pair[1].y, pair[1].x));
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
fn requirements_card_names_the_activity_without_a_fixed_round_count() {
    let mut view = View::default();
    let key = REQUIREMENTS_ROUNDS[0];
    for running in [false, true] {
        view.agents.insert(
            key.into(),
            super::super::view::Agent {
                running,
                activity: "turn finished".into(),
                ..Default::default()
            },
        );
        let graph = Graph::new(&view);
        let node = graph.node(key).unwrap();
        let mut canvas = Canvas::new_window(node.x, node.y, node.width, node.height);
        draw_node(
            &view,
            &mut canvas,
            node,
            false,
            None,
            &Theme::new(true, false),
            0,
        );
        let status = &canvas.lines()[1];
        assert!(status.contains("extracting"), "{status}");
        assert!(
            !status.contains("/4") && !status.contains("round"),
            "{status}"
        );
    }
}
