use super::health::{card_lines, describe, effect_lines};
use super::nodes::draw_node;
use super::*;
const FULL: Size = Size {
    w: WIDTH,
    h: HEIGHT,
};

#[test]
fn windowed_canvas_storage_is_exactly_the_viewport_area() {
    let canvas = Canvas::new_window(40, 20, 17, 9);
    assert_eq!(canvas.cells.len(), 17 * 9);
    assert_eq!(canvas.width, 17);
    assert_eq!(canvas.height, 9);
}

#[test]
fn small_viewports_shrink_cards_to_two_by_two_and_wide_ones_center_them() {
    let tasks = (1..=6).map(|n| task(&format!("t{n}"), &[])).collect();
    let view = view(tasks);
    let layout = |vp| {
        Graph::new_with_viewport(
            &view,
            None,
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
    graph.render(&view, &mut canvas, Filter::All, ORCHESTRATOR);
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
        None,
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
            None,
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
        None,
        &collapsed,
        super::GroupingMode::Dependency,
        Some((80, 24)),
    );
    let idle = cache.get(
        &view,
        1,
        None,
        &collapsed,
        super::GroupingMode::Dependency,
        Some((80, 24)),
    );
    assert!(std::rc::Rc::ptr_eq(&first, &idle));
    view.apply(
        &serde_json::json!({"seq": 1, "kind": "plan_proposed", "payload": {
            "plan": "p1", "tasks": [{"id": "t1", "title": "first", "depends_on": []}]}}),
    );
    let updated = cache.get(
        &view,
        1,
        None,
        &collapsed,
        super::GroupingMode::Dependency,
        Some((80, 24)),
    );
    assert!(!std::rc::Rc::ptr_eq(&first, &updated));
    let resized = cache.get(
        &view,
        1,
        None,
        &collapsed,
        super::GroupingMode::Dependency,
        Some((40, 12)),
    );
    assert!(!std::rc::Rc::ptr_eq(&updated, &resized));
    collapsed.insert("t1".into());
    let folded = cache.get(
        &view,
        1,
        None,
        &collapsed,
        super::GroupingMode::Status,
        Some((40, 12)),
    );
    assert!(!std::rc::Rc::ptr_eq(&resized, &folded));
    let searched = cache.get(
        &view,
        1,
        Some("first"),
        &collapsed,
        super::GroupingMode::Status,
        Some((40, 12)),
    );
    assert!(!std::rc::Rc::ptr_eq(&folded, &searched));
    let switched = cache.get(
        &view,
        2,
        Some("first"),
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
        Some("first"),
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
fn role_aliases_share_one_card_even_when_collapsed_or_filtered() {
    let mut row = task("t1", &[]);
    row.status = "verifying".into();
    row.round = 2;
    row.summary = Some("Execute storage migration.".into());
    row.verifier_summary = Some("Validate roundtrips.".into());
    let mut view = view(vec![row, task("t2", &["t1"])]);
    view.links.insert(
        ("verifier:t1".into(), "worker:t2".into()),
        super::super::view::Link {
            count: 1,
            last: std::time::Instant::now(),
        },
    );
    view.links.insert(
        ("verifier:t1".into(), "worker:t1".into()),
        super::super::view::Link {
            count: 1,
            last: std::time::Instant::now(),
        },
    );
    view.links.insert(
        ("worker:t1".into(), "verifier:t2".into()),
        super::super::view::Link {
            count: 2,
            last: std::time::Instant::now() - RECENT * 2,
        },
    );
    let collapsed = std::collections::HashSet::from(["t1".to_owned()]);
    for grouping in [GroupingMode::Dependency, GroupingMode::Status] {
        for viewport in [None, Some((78, 20))] {
            let graph = Graph::new_with_viewport(&view, None, &collapsed, grouping, viewport);
            assert_eq!(graph.nodes.len(), 5);
            assert!(
                graph
                    .nodes
                    .iter()
                    .all(|node| !node.key.starts_with("verifier:"))
            );
            let node = graph.node("verifier:t1").unwrap();
            assert_eq!(node, graph.node("worker:t1").unwrap());
            assert!(node.visible && node.collapsed);
            let routes = graph.edge_routes(Filter::Mail);
            assert_eq!(routes.len(), 1);
            assert_eq!((routes[0].0, routes[0].1), ("worker:t1", "worker:t2"));
            let mail = graph
                .edges
                .iter()
                .find(|edge| matches!(edge.kind, Kind::Mail { .. }))
                .unwrap();
            assert_eq!(mail.label.as_deref(), Some("M✉3"));
            assert_eq!(mail.kind, Kind::Mail { recent: true });
            let mut canvas = Canvas::new_window(node.x, node.y, WIDTH, HEIGHT);
            graph.render(&view, &mut canvas, Filter::All, "verifier:t1");
            let text = canvas.lines().join("\n");
            assert!(text.contains("verifier r2") && text.contains("checking r2"));
            assert!(!text.contains("Execute") && !text.contains("Validate"));
            for query in ["verifier:t1", "roundtrips", "checking r2"] {
                let filtered =
                    Graph::new_with_viewport(&view, Some(query), &collapsed, grouping, viewport);
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
    view.tasks[0].status = "verifying".into();
    for role in ["worker", "verifier"] {
        view.agents.clear();
        view.agents.insert(
            format!("{role}:t1"),
            super::super::view::Agent {
                activity: "✗ process exited".into(),
                ..Default::default()
            },
        );
        assert_eq!(health(&view, "worker:t1"), Health::Problem);
        assert_eq!(health(&view, "verifier:t1"), Health::Problem);
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
            graph.render(&view, &mut canvas, Filter::All, "worker:t1");
            let rows = canvas.lines();
            if node.key == "worker:t1" {
                assert!(rows[0].contains("Repair storage"), "{rows:?}");
                assert!(rows[1].contains("- implementation"), "{rows:?}");
            } else {
                assert!(rows[1].contains("- orchestration"), "{rows:?}");
            }
        }
    }
}

#[test]
fn compact_cards_with_effects_can_have_no_effect_rows() {
    let mut row = task("t1", &[]);
    row.effect = Some("Updated storage.".into());
    let view = view(vec![row]);
    let mut node = Node::new("worker:t1".into(), 0, 0);
    node.height = MIN_HEIGHT;
    assert_eq!(node.body_lines().1, 0);
    let mut canvas = Canvas::new_window(0, 0, node.width, node.height);
    draw_node(
        &view,
        &mut canvas,
        &node,
        false,
        &Theme::new(true, false),
        0,
    );
    assert!(canvas.lines().iter().all(|line| !line.contains("effect:")));
}

#[test]
fn summaries_are_bounded_and_truncated_at_grapheme_boundaries() {
    let mut row = task("t1", &[]);
    row.summary = Some("👩‍💻e\u{301}".repeat(100));
    row.verifier_summary = Some("Check the summary.".into());
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
    assert_eq!(graph.node("worker:t1").unwrap().height, HEIGHT);
    let worker = graph.node("worker:t1").unwrap();
    let verifier = graph.node("verifier:t1").unwrap();
    assert_eq!(verifier.key, worker.key);
    for (_, _, path) in graph.edge_routes(Filter::All) {
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
        row.focus = "A long verification specification.".repeat(40);
        row.summary = summary.map(str::to_owned);
        row.verifier_summary = summary.map(str::to_owned);
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
                "verifier:t1",
                usize::from(WIDTH - 4),
                MAX_SUMMARY_LINES
            ),
            ["Verify Storage."]
        );
        assert!(
            Graph::new(&view)
                .nodes
                .iter()
                .all(|node| node.height == HEIGHT)
        );
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
            "verifier:t1",
            usize::from(WIDTH - 4),
            MAX_SUMMARY_LINES
        ),
        ["Verify t1."]
    );
}

#[test]
fn summary_only_search_finds_tasks_and_admin_descriptions_stay_fixed() {
    let mut row = task("t1", &[]);
    row.summary = Some("Repair data storage.".into());
    row.verifier_summary = Some("Validate roundtrips.".into());
    let mut view = view(vec![row]);
    for query in ["data storage", "roundtrips"] {
        let graph = Graph::new_with_options(
            &view,
            Some(query),
            &Default::default(),
            GroupingMode::Dependency,
        );
        assert!(graph.node("worker:t1").unwrap().visible);
        assert!(graph.node("verifier:t1").unwrap().visible);
    }
    for (key, description) in [
        (ORCHESTRATOR, "Coordinates the plan."),
        (PLAN_VERIFIER, "Checks the plan."),
        (AUDITOR, "Audits task results."),
        (
            super::super::super::COMMENT_ORCHESTRATOR,
            "Addresses comments.",
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
            card_lines(&view, key, usize::from(WIDTH - 4), MAX_SUMMARY_LINES),
            [description]
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
    assert_eq!(health(&view, "verifier:t1"), Health::Problem);
    assert_eq!(health(&view, "worker:t2"), Health::Waiting);
    assert_eq!(health(&view, "worker:t3"), Health::Working);
    assert_eq!(health(&view, "verifier:t3"), Health::Working);
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
    graph.render_with(&view, &mut canvas, Filter::All, "worker:t1", &theme, 0);
    let text = canvas.lines().join("\n");
    for frame in ['╭', '╮', '╰', '╯', '╔', '╗', '╚', '╝', '║'] {
        assert!(!text.contains(frame), "{frame} in\n{text}");
    }
    assert!(text.contains(theme.glyphs().problem));
}

#[test]
fn working_cards_animate_with_the_tick_and_stay_ascii_safe() {
    let mut row = task("t1", &[]);
    row.status = "working".into();
    let view = view(vec![row]);
    let graph = Graph::new(&view);
    let frame = |theme: &Theme, tick| {
        let mut canvas = Canvas::new(graph.width, graph.height);
        graph.render_with(&view, &mut canvas, Filter::All, "", theme, tick);
        canvas.lines().join("\n")
    };
    let unicode = Theme::new(true, false);
    assert_ne!(frame(&unicode, 0), frame(&unicode, 1));
    assert!(frame(&unicode, 3).contains(unicode.spinner(3)));
    let ascii = Theme::new(false, true);
    assert!(frame(&ascii, 2).contains(ascii.spinner(2)));
}

#[test]
fn tasks_sit_in_columns_by_dependency_depth_with_one_card_per_task() {
    let view = view(vec![task("t1", &[]), task("t2", &["t1"]), task("t3", &[])]);
    let graph = Graph::new(&view);
    let at = |key: &str| {
        let node = graph.node(key).unwrap();
        (node.x, node.y)
    };
    assert_eq!(at("worker:t1"), (column_x(FULL, 0), row_y(FULL, 1)));
    assert_eq!(at("verifier:t1"), at("worker:t1"));
    assert_eq!(at("worker:t3"), (column_x(FULL, 0), row_y(FULL, 2)));
    assert_eq!(at("worker:t2"), (column_x(FULL, 1), row_y(FULL, 1)));
    assert_eq!(at(ORCHESTRATOR), (column_x(FULL, 1), 0));
}

#[test]
fn cards_are_fixed_and_no_event_moves_a_single_node() {
    use super::super::view::Agent;
    use serde_json::json;
    let mut worker = task("t1", &[]);
    worker.title = "Store".into();
    worker.summary = Some("Fix the storage path so writes survive a restart.".into());
    let mut view = view(vec![worker, task("t2", &["t1"])]);
    let geometry = |view: &View| -> Vec<(String, u16, u16, u16, bool)> {
        Graph::new(view)
            .nodes
            .iter()
            .map(|node| (node.key.clone(), node.x, node.y, node.height, node.visible))
            .collect()
    };
    let before = geometry(&view);
    assert!(before.iter().all(|(_, _, _, height, _)| *height == HEIGHT));

    // Activity: the round starts and its agent runs — no geometry moves,
    // and the worker's doing text lands on its own line, not behind
    // the state.
    view.tasks[0].status = "working".into();
    view.tasks[0].round = 1;
    view.agents.insert(
        "worker:t1".into(),
        Agent {
            running: true,
            provider: "codex".into(),
            activity: "cargo check --workspace".into(),
            ..Default::default()
        },
    );
    assert_eq!(geometry(&view), before, "activity moves no card");
    let graph = Graph::new(&view);
    let node = graph.node("worker:t1").unwrap();
    let mut canvas = Canvas::new_window(node.x, node.y, WIDTH, node.height);
    graph.render(&view, &mut canvas, Filter::All, "worker:t1");
    let rows = canvas.lines();
    assert!(rows[2].contains("working r1"), "{}", rows[2]);
    assert!(!rows[2].contains("cargo check"), "state alone: {}", rows[2]);
    assert!(
        rows[1].contains("codex - "),
        "the provider has its own row: {}",
        rows[1]
    );
    assert!(
        !rows[usize::from(ACTIVITY_ROW)].contains("codex"),
        "the activity line is independent of the provider: {}",
        rows[usize::from(ACTIVITY_ROW)]
    );
    assert!(
        rows[usize::from(ACTIVITY_ROW)].contains("doing: cargo check"),
        "{}",
        rows[usize::from(ACTIVITY_ROW)]
    );

    // Effect and verdict: the round's card data changes no geometry and
    // takes its own slot without pushing the summary away.
    view.tasks[0].status = "approved".into();
    view.tasks[0].round = 1;
    view.tasks[0].effect = Some("3 files changed — src/a.rs, src/b.rs, src/c.rs".into());
    view.tasks[0].obligations = Some(json!({"total": 2, "ok": 2, "missing": 0}));
    view.tasks[0].integration = Some(json!({"outcome": "committed", "detail": "as abc123"}));
    assert_eq!(geometry(&view), before, "a verdict moves no card");
    let graph = Graph::new(&view);
    let node = graph.node("worker:t1").unwrap();
    let mut canvas = Canvas::new_window(node.x, node.y, WIDTH, node.height);
    graph.render(&view, &mut canvas, Filter::All, "worker:t1");
    let rows = canvas.lines();
    assert!(
        rows[..BODY_ROW as usize + MAX_SUMMARY_LINES]
            .iter()
            .any(|row| row.contains("Fix the storage")),
        "the summary keeps its slot: {rows:?}"
    );
    let effect_slot: String = rows[EFFECT_ROW as usize..rows.len() - 1].join(" ");
    assert!(effect_slot.contains("effect: 3 files"), "{rows:?}");
    assert!(
        rows.last().unwrap().contains("committed"),
        "meta at the bottom"
    );
    assert_eq!(rows.len(), usize::from(HEIGHT));

    // A retry clears the round's card data and still moves no card.
    view.tasks[0].status = "queued".into();
    view.tasks[0].effect = None;
    view.tasks[0].obligations = None;
    view.tasks[0].integration = None;
    assert_eq!(geometry(&view), before, "a retry moves no card");
    // Neither role ever shows a summary line displaced by the other's
    // data: every visible card keeps its fixed slot count.
    for node in Graph::new(&view).nodes.iter().filter(|node| node.visible) {
        assert_eq!(node.height, HEIGHT, "{}", node.key);
        assert!(node.y.saturating_add(HEIGHT) >= HEIGHT);
    }
}

#[test]
fn every_card_block_stays_in_its_own_slot() {
    use super::super::view::Agent;
    use serde_json::json;
    // Slots follow one arithmetic: the height ends where the last
    // block ends, so no block can overrun the meta row.
    assert_eq!(BRANCH_ROW, ACTIVITY_ROW + 1);
    assert_eq!(BODY_ROW, BRANCH_ROW + 1);
    assert_eq!(EFFECT_ROW, BODY_ROW + MAX_SUMMARY_LINES as u16);
    assert_eq!(HEIGHT, EFFECT_ROW + MAX_EFFECT_LINES as u16 + 1);
    let mut row = task("t1", &["t0"]);
    row.summary =
        Some("Repair the storage engine so a restart never loses a queued write, at all.".into());
    let mut view = view(vec![task("t0", &[]), row]);
    let render = |view: &View| -> Vec<String> {
        let graph = Graph::new(view);
        let node = graph.node("worker:t1").unwrap();
        let mut canvas = Canvas::new_window(node.x, node.y, WIDTH, node.height);
        graph.render(view, &mut canvas, Filter::All, "worker:t1");
        canvas.lines()
    };
    // Working: the doing text sits on the activity row, alone, and no
    // effect words appear anywhere while the round reported nothing.
    view.tasks[1].status = "working".into();
    view.tasks[1].round = 1;
    view.agents.insert(
        "worker:t1".into(),
        Agent {
            running: true,
            activity: "cargo  check".into(),
            ..Default::default()
        },
    );
    let rows = render(&view);
    assert_eq!(rows.len(), usize::from(HEIGHT));
    assert!(rows[2].contains("working r1"), "{rows:?}");
    assert!(!rows[2].contains("cargo"), "state alone: {}", rows[2]);
    assert!(
        rows[usize::from(ACTIVITY_ROW)].contains("doing: cargo check"),
        "{rows:?}"
    );
    for (row, text) in rows.iter().enumerate() {
        if row != usize::from(ACTIVITY_ROW) {
            assert!(!text.contains("doing"), "{rows:?}");
        }
        assert!(!text.contains("effect"), "{rows:?}");
    }
    // A verdict: the card's shape of the shared effect reading takes
    // its own slot — marker in front, text normalized once — and
    // displaces no summary row and no meta row.
    view.tasks[1].status = "approved".into();
    view.tasks[1].round = 2;
    view.tasks[1].effect = Some("two  files cut…".into());
    view.tasks[1].effect_cut = Some(true);
    view.tasks[1].integration = Some(json!({"outcome": "committed", "detail": "as abc12"}));
    let rows = render(&view);
    assert_eq!(
        effect_lines(&view, "worker:t1", usize::from(WIDTH - 4), MAX_EFFECT_LINES).join(""),
        "effect: [fragment] two files cut…",
        "one normalized line, marker in front"
    );
    assert!(
        rows[usize::from(EFFECT_ROW)].contains("effect: [fragment]"),
        "{rows:?}"
    );
    for text in &rows[BODY_ROW as usize..usize::from(EFFECT_ROW)] {
        assert!(!text.contains("effect"), "summary keeps its slot: {rows:?}");
    }
    assert!(rows[BODY_ROW as usize].contains("Repair"), "{rows:?}");
    let bottom = &rows[usize::from(HEIGHT) - 1];
    assert!(bottom.contains("committed"), "meta at the bottom: {rows:?}");
    assert!(!bottom.contains("effect"), "{rows:?}");
}

#[test]
fn arrows_move_to_the_nearest_node_in_that_direction() {
    let view = view(vec![task("t1", &[]), task("t2", &["t1"]), task("t3", &[])]);
    let graph = Graph::new(&view);
    let go = |from: &str, dx, dy| graph.neighbour(from, dx, dy);
    assert_eq!(go("worker:t1", 1, 0).as_deref(), Some("worker:t2"));
    assert_eq!(go("worker:t1", 0, 1).as_deref(), Some("worker:t3"));
    assert_eq!(go("verifier:t1", 0, 1).as_deref(), Some("worker:t3"));
    assert_eq!(go("worker:t2", -1, 0).as_deref(), Some("worker:t1"));
    assert_eq!(go(ORCHESTRATOR, 0, 1).as_deref(), Some("worker:t2"));
    assert_eq!(go(PLAN_VERIFIER, -1, 0), None);
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
    graph.render(&view, &mut canvas, Filter::All, "worker:t2");
    let lines = canvas.lines();
    let worker = graph.node("worker:t1").unwrap();
    let next = graph.node("worker:t2").unwrap();
    let row = &lines[usize::from(worker.mid_y())];
    let chars: Vec<char> = row.chars().collect();
    // t1 → t2 straight across the gap, ending in an arrow at t2's border.
    assert_eq!(chars[usize::from(next.x - 1)], '▶', "{row}");
    assert_eq!(chars[usize::from(worker.right())], '─', "{row}");
    assert_eq!(chars[usize::from(next.x)], '█', "the selected card's bar");
    assert!(lines[usize::from(worker.y)].contains("Task t1"));
    // A message between them is counted beside the source.
    assert!(lines[usize::from(worker.mid_y() - 1)].contains("✉2"));
    assert!(lines[usize::from(worker.y + 2)].contains("proposed"));
}

#[test]
fn all_includes_every_supervision_edge() {
    let view = view(vec![task("t1", &[]), task("t2", &["t1"])]);
    let graph = Graph::new(&view);
    let all = graph.edge_routes(Filter::All);
    let supervision = graph.edge_routes(Filter::Supervises);
    assert_eq!(supervision.len(), 2);
    for edge in &supervision {
        assert!(all.contains(edge), "All must include {edge:?}");
    }
    assert!(
        graph
            .edge_routes(Filter::Depends)
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
