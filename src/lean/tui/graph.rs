//! The team as a two-dimensional graph: one card per task, orthogonal edges for
//! what relates them. The layout depends only on the plan's shape, so nodes
//! stay put while events arrive.

use super::taskinfo;
use super::theme::Theme;
use super::view::{AUDITOR, ORCHESTRATOR, PLAN_VERIFIER, View, agent_name, clean, truncate, wrap};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use std::collections::HashMap;
use std::time::Duration;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub const WIDTH: u16 = 26;
const MAX_SUMMARY_LINES: usize = 4;
const MAX_EFFECT_LINES: usize = 2;
/// Fixed slots inside a card, counted from its top, each starting right
/// where the previous block can end: title, work kind, phase, verifier state, one activity
/// line, up to `MAX_SUMMARY_LINES` summary lines, up to `MAX_EFFECT_LINES`
/// effect lines and the meta line at the bottom. The height is that same
/// arithmetic's end, so frame and slots are one rule that cannot drift
/// apart: heights and positions depend only on the plan's shape, filters
/// and grouping — never on activity, an effect, a verdict or a retry.
const ACTIVITY_ROW: u16 = 4;
const BODY_ROW: u16 = ACTIVITY_ROW + 1;
const EFFECT_ROW: u16 = BODY_ROW + MAX_SUMMARY_LINES as u16;
pub const HEIGHT: u16 = EFFECT_ROW + MAX_EFFECT_LINES as u16 + 1;
const GAP_X: u16 = 6;
const GAP_Y: u16 = 2;
/// Room left of the first column for edges entering it.
const MARGIN: u16 = 3;
/// How long a message keeps its edge highlighted.
const RECENT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    All,
    Depends,
    Verifies,
    Mail,
    Supervises,
}

impl Filter {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Depends,
            Self::Depends => Self::Verifies,
            Self::Verifies => Self::Mail,
            Self::Mail => Self::Supervises,
            Self::Supervises => Self::All,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Depends => "dependencies",
            Self::Verifies => "verification",
            Self::Mail => "messages",
            Self::Supervises => "supervision",
        }
    }

    fn shows(self, kind: Kind) -> bool {
        match self {
            Self::All => true,
            Self::Depends => kind == Kind::Depends,
            Self::Verifies => kind == Kind::Verifies,
            Self::Mail => matches!(kind, Kind::Mail { .. }),
            Self::Supervises => kind == Kind::Supervises,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Depends,
    Verifies,
    Mail { recent: bool },
    Supervises,
}

impl Kind {
    fn style(self) -> Style {
        match self {
            Self::Depends => Style::default().fg(Color::Blue),
            Self::Verifies => Style::default().fg(Color::Yellow),
            Self::Mail { recent: true } => Style::default()
                .fg(Color::Magenta)
                .add_modifier(Modifier::BOLD),
            Self::Mail { recent: false } => Style::default().fg(Color::Magenta),
            Self::Supervises => Style::default().fg(Color::DarkGray),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GroupingMode {
    #[default]
    Dependency,
    Status,
}

impl GroupingMode {
    pub fn next(self) -> Self {
        match self {
            Self::Dependency => Self::Status,
            Self::Status => Self::Dependency,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Dependency => "dependencies",
            Self::Status => "state",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub key: String,
    pub x: u16,
    pub y: u16,
    pub height: u16,
    pub visible: bool,
    pub group: Option<String>,
    pub collapsed: bool,
}

impl Default for Node {
    fn default() -> Self {
        Self {
            key: String::new(),
            x: 0,
            y: 0,
            height: HEIGHT,
            visible: true,
            group: None,
            collapsed: false,
        }
    }
}

impl Node {
    pub fn new(key: String, x: u16, y: u16) -> Self {
        Self {
            key,
            x,
            y,
            height: HEIGHT,
            visible: true,
            group: None,
            collapsed: false,
        }
    }

    fn mid_y(&self) -> u16 {
        self.y.saturating_add(self.height / 2)
    }

    fn right(&self) -> u16 {
        self.x + WIDTH
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Edge {
    from: String,
    to: String,
    kind: Kind,
    label: Option<String>,
}

#[derive(Debug, Default)]
pub struct Graph {
    pub nodes: Vec<Node>,
    edges: Vec<Edge>,
    pub width: u16,
    pub height: u16,
    pub filter_query: Option<String>,
    pub collapsed: std::collections::HashSet<String>,
    pub grouping: GroupingMode,
}

#[derive(Default)]
pub struct LayoutCache {
    entry: Option<CachedGraph>,
}

struct CachedGraph {
    generation: u64,
    revision: u64,
    counts: (usize, usize, usize),
    idle: bool,
    viewport: Option<(u16, u16)>,
    expires: Option<std::time::Instant>,
    graph: std::rc::Rc<Graph>,
}

impl LayoutCache {
    pub fn get(
        &mut self,
        view: &View,
        generation: u64,
        query: Option<&str>,
        collapsed: &std::collections::HashSet<String>,
        grouping: GroupingMode,
        viewport: Option<(u16, u16)>,
    ) -> std::rc::Rc<Graph> {
        let now = std::time::Instant::now();
        let counts = (view.tasks.len(), view.agents.len(), view.links.len());
        if let Some(entry) = &self.entry
            && entry.generation == generation
            && entry.revision == view.layout_revision
            && entry.counts == counts
            && entry.idle == view.idle
            && entry.viewport == viewport
            && entry.graph.filter_query.as_deref() == query
            && &entry.graph.collapsed == collapsed
            && entry.graph.grouping == grouping
            && entry.expires.is_none_or(|expiry| now < expiry)
        {
            return std::rc::Rc::clone(&entry.graph);
        }
        let graph = std::rc::Rc::new(Graph::new_with_viewport(
            view, query, collapsed, grouping, viewport,
        ));
        let expires = view
            .links
            .values()
            .filter_map(|link| link.last.checked_add(RECENT))
            .filter(|expiry| *expiry > now)
            .min();
        self.entry = Some(CachedGraph {
            generation,
            revision: view.layout_revision,
            counts,
            idle: view.idle,
            viewport,
            expires,
            graph: std::rc::Rc::clone(&graph),
        });
        graph
    }
}

/// Canonical UI identity; role addresses in events and storage stay intact.
pub fn task_node_key(key: &str) -> String {
    match crate::lean::task_role(key) {
        Some((id, true)) => format!("worker:{id}"),
        _ => key.to_owned(),
    }
}

fn column_x(column: u16) -> u16 {
    MARGIN + column * (WIDTH + GAP_X)
}

fn row_y(row: u16) -> u16 {
    row * (HEIGHT + GAP_Y)
}

pub type EdgeRoute<'a> = (&'a str, &'a str, Vec<(u16, u16)>);

impl Graph {
    pub fn new(view: &View) -> Self {
        Self::new_with_options(
            view,
            None,
            &std::collections::HashSet::new(),
            GroupingMode::Dependency,
        )
    }

    pub fn new_with_options(
        view: &View,
        filter_query: Option<&str>,
        collapsed: &std::collections::HashSet<String>,
        grouping: GroupingMode,
    ) -> Self {
        Self::new_with_viewport(view, filter_query, collapsed, grouping, None)
    }

    pub fn new_with_viewport(
        view: &View,
        filter_query: Option<&str>,
        collapsed: &std::collections::HashSet<String>,
        grouping: GroupingMode,
        viewport: Option<(u16, u16)>,
    ) -> Self {
        let mut graph = Graph {
            nodes: Vec::new(),
            edges: Vec::new(),
            width: 0,
            height: 0,
            filter_query: filter_query.map(str::to_string),
            collapsed: collapsed.clone(),
            grouping,
        };

        let n = view.tasks.len();
        let index: HashMap<&str, usize> = view
            .tasks
            .iter()
            .enumerate()
            .map(|(index, task)| (task.id.as_str(), index))
            .collect();

        let mut columns = vec![0u16; n];

        match grouping {
            GroupingMode::Dependency => {
                let mut adj: Vec<Vec<usize>> = vec![Vec::new(); n];
                let mut in_degree: Vec<usize> = vec![0; n];
                let mut parents: Vec<Vec<usize>> = vec![Vec::new(); n];

                for (i, task) in view.tasks.iter().enumerate() {
                    for p_id in &task.depends_on {
                        if let Some(&p_idx) = index.get(p_id.as_str())
                            && p_idx != i
                        {
                            adj[p_idx].push(i);
                            parents[i].push(p_idx);
                            in_degree[i] += 1;
                        }
                    }
                }

                let mut queue = std::collections::VecDeque::new();
                for (i, &deg) in in_degree.iter().enumerate().take(n) {
                    if deg == 0 {
                        queue.push_back(i);
                    }
                }

                let mut visited = 0;
                while let Some(u) = queue.pop_front() {
                    visited += 1;
                    let u_depth = columns[u];
                    for &v in &adj[u] {
                        if u_depth + 1 > columns[v] {
                            columns[v] = u_depth + 1;
                        }
                        in_degree[v] = in_degree[v].saturating_sub(1);
                        if in_degree[v] == 0 {
                            queue.push_back(v);
                        }
                    }
                }

                // If any cycles exist, break them cleanly without recursion
                if visited < n {
                    for i in 0..n {
                        if in_degree[i] > 0 {
                            let max_p = parents[i]
                                .iter()
                                .filter(|&&p| in_degree[p] == 0)
                                .map(|&p| columns[p] + 1)
                                .max()
                                .unwrap_or(0);
                            columns[i] = max_p;
                        }
                    }
                }
            }
            GroupingMode::Status => {
                for (i, task) in view.tasks.iter().enumerate() {
                    columns[i] = match task.status.as_str() {
                        "proposed" | "queued" => 0,
                        "working" | "verifying" => 1,
                        "approved" => 2,
                        _ => 3,
                    };
                }
            }
        }

        let mut column_tasks: HashMap<u16, Vec<usize>> = HashMap::new();
        for (i, &col) in columns.iter().enumerate().take(n) {
            column_tasks.entry(col).or_default().push(i);
        }
        let mut cols: Vec<u16> = column_tasks.keys().copied().collect();
        cols.sort_unstable();

        let query_clean = filter_query.map(str::trim).filter(|s| !s.is_empty());
        let matches_query = |key: &str| -> bool {
            let Some(q) = query_clean else {
                return true;
            };
            let q_lower = q.to_lowercase();
            key.to_lowercase().contains(&q_lower)
                || agent_name(key).to_lowercase().contains(&q_lower)
                || key
                    .split_once(':')
                    .and_then(|(_, id)| view.tasks.iter().find(|t| t.id == id))
                    .is_some_and(|t| {
                        t.title.to_lowercase().contains(&q_lower)
                            || t.status.to_lowercase().contains(&q_lower)
                            || taskinfo::worker_status(t).to_lowercase().contains(&q_lower)
                            || taskinfo::verifier_status(t)
                                .to_lowercase()
                                .contains(&q_lower)
                            || t.goal.to_lowercase().contains(&q_lower)
                            || t.focus.to_lowercase().contains(&q_lower)
                            || t.summary
                                .as_ref()
                                .is_some_and(|s| s.to_lowercase().contains(&q_lower))
                            || t.verifier_summary
                                .as_ref()
                                .is_some_and(|s| s.to_lowercase().contains(&q_lower))
                            || t.effect
                                .as_ref()
                                .is_some_and(|s| s.to_lowercase().contains(&q_lower))
                    })
        };

        let pv_vis = matches_query(PLAN_VERIFIER);
        let orch_vis = matches_query(ORCHESTRATOR);
        let aud_vis = matches_query(AUDITOR);
        let admin_vis = pv_vis || orch_vis || aud_vis;

        // Determine which tasks are visible
        let mut visible_tasks: Vec<(usize, u16)> = Vec::new();
        for (i, task) in view.tasks.iter().enumerate() {
            let id = &task.id;
            let worker_vis = matches_query(&format!("worker:{id}"));
            let verifier_vis = matches_query(&format!("verifier:{id}"));
            if worker_vis || verifier_vis {
                visible_tasks.push((i, columns[i]));
            }
        }

        // Active distinct raw column depths among visible tasks
        let active_raw_cols: Vec<u16> = visible_tasks
            .iter()
            .map(|&(_, col)| col)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();

        // Group visible tasks by relative depth
        let mut rel_depth_tasks: HashMap<u16, Vec<usize>> = HashMap::new();
        for &(task_idx, raw_col) in &visible_tasks {
            let rel_depth = active_raw_cols
                .iter()
                .position(|&c| c == raw_col)
                .unwrap_or(0) as u16;
            rel_depth_tasks.entry(rel_depth).or_default().push(task_idx);
        }

        let mut sorted_rel_depths: Vec<u16> = rel_depth_tasks.keys().copied().collect();
        sorted_rel_depths.sort_unstable();

        let mut placed_tasks = std::collections::HashSet::new();

        if let Some((vp_w, _)) = viewport {
            // Flow: the first row holds the orchestrator and its verifiers, the
            // tasks it spawns follow, wrapping at the viewport width.
            // Each task card contains both execution and verification.
            let max_cols = ((vp_w.saturating_sub(MARGIN) + GAP_X) / (WIDTH + GAP_X)).max(1);
            let mut push = |key: &str, col: u16, row: u16, vis: bool| {
                graph.nodes.push(Node {
                    key: key.to_string(),
                    x: if vis { column_x(col) } else { 0 },
                    y: if vis { row_y(row) } else { 0 },
                    height: HEIGHT,
                    visible: vis,
                    group: None,
                    collapsed: false,
                });
            };

            let admin = [
                (PLAN_VERIFIER, pv_vis),
                (ORCHESTRATOR, orch_vis),
                (AUDITOR, aud_vis),
            ];
            let admin_rows = if max_cols >= 3 {
                for (slot, &(key, vis)) in admin.iter().enumerate() {
                    push(key, slot as u16, 0, vis);
                }
                u16::from(admin_vis)
            } else {
                let mut shown = 0u16;
                for &(key, vis) in &admin {
                    if vis {
                        push(key, shown % max_cols, shown / max_cols, true);
                        shown += 1;
                    } else {
                        push(key, 0, 0, false);
                    }
                }
                shown.div_ceil(max_cols)
            };

            let ordered: Vec<usize> = sorted_rel_depths
                .iter()
                .flat_map(|depth| {
                    let tasks = rel_depth_tasks.get_mut(depth).unwrap();
                    tasks.sort_unstable();
                    tasks.clone()
                })
                .collect();
            let worker_base = admin_rows;

            for (slot, &task_idx) in ordered.iter().enumerate() {
                placed_tasks.insert(task_idx);
                let id = &view.tasks[task_idx].id;
                let is_task_collapsed =
                    collapsed.contains(id) || collapsed.contains(&format!("task:{id}"));
                let (col, offset) = (slot as u16 % max_cols, slot as u16 / max_cols);
                let worker_key = format!("worker:{id}");
                let verifier_key = format!("verifier:{id}");
                let worker_vis = matches_query(&worker_key);
                let verifier_vis = matches_query(&verifier_key);

                let mut worker = Node::new(worker_key, column_x(col), row_y(worker_base + offset));
                worker.collapsed = is_task_collapsed;
                worker.group = Some(format!("task:{id}"));
                worker.visible = worker_vis || verifier_vis;
                graph.nodes.push(worker);
            }
        } else {
            let mut place_admin = |key: &str, col: u16, row: u16, vis: bool| {
                graph.nodes.push(Node {
                    key: key.to_string(),
                    x: if vis { column_x(col) } else { 0 },
                    y: if vis { row_y(row) } else { 0 },
                    height: HEIGHT,
                    visible: vis,
                    group: None,
                    collapsed: false,
                });
            };
            place_admin(PLAN_VERIFIER, 0, 0, pv_vis);
            place_admin(ORCHESTRATOR, 1, 0, orch_vis);
            place_admin(AUDITOR, 2, 0, aud_vis);

            for &rel_depth in &sorted_rel_depths {
                let tasks_at_depth = rel_depth_tasks.get_mut(&rel_depth).unwrap();
                tasks_at_depth.sort_unstable();
                let first_row = u16::from(admin_vis);
                for (current_row, &task_idx) in (first_row..).zip(tasks_at_depth.iter()) {
                    placed_tasks.insert(task_idx);
                    let task = &view.tasks[task_idx];
                    let id = &task.id;
                    let is_task_collapsed =
                        collapsed.contains(id) || collapsed.contains(&format!("task:{id}"));
                    let worker_key = format!("worker:{id}");
                    let verifier_key = format!("verifier:{id}");
                    let worker_vis = matches_query(&worker_key);
                    let verifier_vis = matches_query(&verifier_key);

                    let mut worker = Node::new(worker_key, column_x(rel_depth), row_y(current_row));
                    worker.collapsed = is_task_collapsed;
                    worker.group = Some(format!("task:{id}"));
                    worker.visible = worker_vis || verifier_vis;
                    graph.nodes.push(worker);
                }
            }
        }

        // Retain remaining hidden tasks as non-visible nodes for graph querying
        for (i, task) in view.tasks.iter().enumerate() {
            if !placed_tasks.contains(&i) {
                let id = &task.id;
                let is_task_collapsed =
                    collapsed.contains(id) || collapsed.contains(&format!("task:{id}"));
                graph.nodes.push(Node {
                    key: format!("worker:{id}"),
                    x: 0,
                    y: 0,
                    height: HEIGHT,
                    visible: false,
                    group: Some(format!("task:{id}")),
                    collapsed: is_task_collapsed,
                });
            }
        }

        // Cards are uniformly tall — every constructor gives them `HEIGHT`
        // — so stacking has one rule, `row_y`: the r-th non-empty row sits
        // where `row_y` puts row r, and a row whose cards are all filtered
        // away closes up without moving any other card.
        let mut rows = std::collections::BTreeSet::<u16>::new();
        for node in graph.nodes.iter().filter(|node| node.visible) {
            rows.insert(node.y);
        }
        let stack: HashMap<_, _> = rows
            .into_iter()
            .enumerate()
            .map(|(row, y)| (y, row_y(row as u16)))
            .collect();
        for node in graph.nodes.iter_mut().filter(|node| node.visible) {
            node.y = stack[&node.y];
        }

        let mut edge = |from: String, to: String, kind: Kind, label: Option<String>| {
            if from == to {
                return;
            }
            graph.edges.push(Edge {
                from,
                to,
                kind,
                label,
            })
        };
        edge(
            PLAN_VERIFIER.into(),
            ORCHESTRATOR.into(),
            Kind::Verifies,
            Some("V".into()),
        );
        edge(
            AUDITOR.into(),
            ORCHESTRATOR.into(),
            Kind::Verifies,
            Some("V".into()),
        );
        for task in &view.tasks {
            let worker = format!("worker:{}", task.id);
            edge(ORCHESTRATOR.into(), worker.clone(), Kind::Supervises, None);
            for parent in &task.depends_on {
                if index.contains_key(parent.as_str()) {
                    edge(
                        format!("worker:{parent}"),
                        worker.clone(),
                        Kind::Depends,
                        Some("D".into()),
                    );
                }
            }
        }
        // Both runtime roles share one visual endpoint. Sum their mail and
        // retain a recent highlight if either underlying link is recent.
        let mut links = std::collections::BTreeMap::<(String, String), (u32, bool)>::new();
        for ((from, to), link) in &view.links {
            let pair = (task_node_key(from), task_node_key(to));
            if pair.0 == pair.1 {
                continue;
            }
            let (count, recent) = links.entry(pair).or_default();
            *count = count.saturating_add(link.count);
            *recent |= link.last.elapsed() < RECENT;
        }
        for ((from, to), (count, recent)) in links {
            edge(from, to, Kind::Mail { recent }, Some(format!("M✉{count}")));
        }
        graph.width = graph
            .nodes
            .iter()
            .filter(|n| n.visible)
            .map(Node::right)
            .max()
            .unwrap_or(0)
            + GAP_X;
        graph.height = graph
            .nodes
            .iter()
            .filter(|n| n.visible)
            .map(|node| node.y.saturating_add(node.height))
            .max()
            .unwrap_or(0);
        graph
    }

    pub fn node(&self, key: &str) -> Option<&Node> {
        self.nodes.iter().find(|node| {
            node.key == key
                || crate::lean::task_role(key).is_some_and(|(id, verifier)| {
                    verifier && node.key.strip_prefix("worker:") == Some(id)
                })
        })
    }

    /// The nearest node from `key` in a direction (dx, dy), each -1, 0 or 1.
    /// Only navigates between visible nodes.
    pub fn neighbour(&self, key: &str, dx: i32, dy: i32) -> Option<String> {
        let from = self.node(key)?;
        let center = |node: &Node| {
            (
                i32::from(node.x) + i32::from(WIDTH) / 2,
                i32::from(node.y) + i32::from(node.height) / 2,
            )
        };
        let (fx, fy) = center(from);
        self.nodes
            .iter()
            .filter(|node| node.visible && node.key != from.key)
            .filter_map(|node| {
                let (x, y) = center(node);
                let (along, across) = if dx != 0 {
                    ((x - fx) * dx, (y - fy).abs())
                } else {
                    ((y - fy) * dy, (x - fx).abs())
                };
                // Rows are closer together than columns; weigh them equally.
                let (along, across) = if dx != 0 {
                    (along, across * 5)
                } else {
                    (along * 5, across)
                };
                (along > 0).then_some((along + 2 * across, node))
            })
            .min_by_key(|(cost, _)| *cost)
            .map(|(_, node)| node.key.clone())
    }

    /// The orthogonal path of an edge, through the gaps between boxes only.
    pub fn route(from: &Node, to: &Node) -> Vec<(u16, u16)> {
        let half = GAP_X / 2;
        // Same column
        if from.x == to.x {
            let is_adjacent = if from.y < to.y {
                from.y.saturating_add(from.height).saturating_add(GAP_Y) == to.y
            } else {
                to.y.saturating_add(to.height).saturating_add(GAP_Y) == from.y
            };
            if is_adjacent {
                let x = to.x + WIDTH / 2;
                if from.y > to.y {
                    return vec![(x, from.y - 1), (x, to.y.saturating_add(to.height))];
                }
                if from.y < to.y {
                    return vec![
                        (x, from.y.saturating_add(from.height)),
                        (x, to.y.saturating_sub(1)),
                    ];
                }
            } else {
                let lane = from.right() + half;
                return vec![
                    (from.right(), from.mid_y()),
                    (lane, from.mid_y()),
                    (lane, to.mid_y()),
                    (to.right(), to.mid_y()),
                ];
            }
        }
        // Neighbours on one row face each other.
        if from.y == to.y {
            if from.x + WIDTH + GAP_X == to.x {
                return vec![(from.right(), from.mid_y()), (to.x - 1, to.mid_y())];
            }
            if to.x + WIDTH + GAP_X == from.x {
                return vec![(from.x - 1, from.mid_y()), (to.right(), to.mid_y())];
            }
        }
        let out = from.right() + half;
        let into = to.x.saturating_sub(half);
        let mut path = vec![(from.right(), from.mid_y()), (out, from.mid_y())];
        if out != into {
            let lane = if to.y == 0 {
                to.height
            } else if to.y > from.y {
                to.y.saturating_sub(1)
            } else {
                to.y.saturating_add(to.height)
            };
            path.push((out, lane));
            path.push((into, lane));
        }
        path.push((into, to.mid_y()));
        path.push((to.x.saturating_sub(1), to.mid_y()));
        path
    }

    /// Returns actual routes for visible edges under the given filter.
    pub fn edge_routes(&self, filter: Filter) -> Vec<EdgeRoute<'_>> {
        let nodes: HashMap<&str, &Node> = self
            .nodes
            .iter()
            .map(|node| (node.key.as_str(), node))
            .collect();
        self.edges
            .iter()
            .filter(|edge| filter.shows(edge.kind))
            .filter_map(|edge| {
                let from = nodes.get(edge.from.as_str())?;
                let to = nodes.get(edge.to.as_str())?;
                if !from.visible || !to.visible {
                    return None;
                }
                Some((edge.from.as_str(), edge.to.as_str(), Self::route(from, to)))
            })
            .collect()
    }

    /// Checks whether any segment of a route penetrates the interior or border of a node card.
    pub fn route_intersects_card(path: &[(u16, u16)], node: &Node) -> bool {
        if node.height == 0 {
            return false;
        }
        for pair in path.windows(2) {
            let (p1, p2) = (pair[0], pair[1]);
            let x_min = p1.0.min(p2.0);
            let x_max = p1.0.max(p2.0);
            let y_min = p1.1.min(p2.1);
            let y_max = p1.1.max(p2.1);
            if x_max >= node.x
                && x_min < node.x.saturating_add(WIDTH)
                && y_max >= node.y
                && y_min < node.y.saturating_add(node.height)
            {
                return true;
            }
        }
        false
    }

    fn arrow(path: &[(u16, u16)]) -> char {
        let [.., (ax, ay), (bx, by)] = path else {
            return '▶';
        };
        match (bx.cmp(ax), by.cmp(ay)) {
            (std::cmp::Ordering::Less, _) => '◀',
            (std::cmp::Ordering::Greater, _) => '▶',
            (_, std::cmp::Ordering::Less) => '▲',
            (_, std::cmp::Ordering::Greater) => '▼',
            _ => '▶',
        }
    }

    pub fn render(&self, view: &View, canvas: &mut Canvas, filter: Filter, selected: &str) {
        self.render_with(view, canvas, filter, selected, &Theme::new(true, false), 0);
    }

    pub fn render_with(
        &self,
        view: &View,
        canvas: &mut Canvas,
        filter: Filter,
        selected: &str,
        theme: &Theme,
        tick: u64,
    ) {
        let nodes: HashMap<&str, &Node> = self
            .nodes
            .iter()
            .map(|node| (node.key.as_str(), node))
            .collect();
        for edge in self.edges.iter().filter(|edge| filter.shows(edge.kind)) {
            let (Some(from), Some(to)) =
                (nodes.get(edge.from.as_str()), nodes.get(edge.to.as_str()))
            else {
                continue;
            };
            if !from.visible || !to.visible {
                continue;
            }
            let path = Self::route(from, to);
            canvas.path(&path, edge.kind.style());
            if let Some(&(x, y)) = path.last() {
                canvas.put(x, y, Self::arrow(&path), edge.kind.style());
            }
            // Beside where the edge leaves its source, inside the gap.
            if let (Some(label), [(x, y), (next_x, _), ..]) = (&edge.label, path.as_slice()) {
                let lx = if *next_x == *x {
                    x + 1
                } else if *next_x < *x {
                    x.saturating_sub(unicode_width::UnicodeWidthStr::width(label.as_str()) as u16)
                } else {
                    x + 1
                };
                let ly = if *next_x == *x {
                    *y
                } else {
                    y.saturating_sub(1)
                };
                canvas.text(lx, ly, label, edge.kind.style());
            }
        }
        let selected = task_node_key(selected);
        for node in self.nodes.iter().filter(|n| n.visible) {
            draw_node(view, canvas, node, node.key == selected, theme, tick);
        }
    }
}

/// How an agent is doing, which decides its icon and color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    Idle,
    Working,
    Waiting,
    Problem,
    Done,
}

pub fn health(view: &View, key: &str) -> Health {
    if let Some((_, id)) = key.split_once(':') {
        let Some(task) = view.tasks.iter().find(|task| task.id == id) else {
            return Health::Idle;
        };
        if matches!(task.status.as_str(), "failed" | "needs_user") {
            return Health::Problem;
        }
        for role in ["worker", "verifier"] {
            if view
                .agents
                .get(&format!("{role}:{id}"))
                .is_some_and(|agent| {
                    agent.activity.starts_with('✗') || agent.activity.starts_with('⚠')
                })
            {
                return Health::Problem;
            }
        }
        if ["worker", "verifier"].iter().any(|role| {
            view.agents
                .get(&format!("{role}:{id}"))
                .is_some_and(|agent| agent.running)
        }) {
            return Health::Working;
        }
        return match task.status.as_str() {
            "waiting" => Health::Waiting,
            "approved" => Health::Done,
            "working" | "verifying" => Health::Working,
            _ => Health::Idle,
        };
    }
    let Some(agent) = view.agents.get(key) else {
        return Health::Idle;
    };
    if agent.running {
        Health::Working
    } else if agent.activity.starts_with('✗') || agent.activity.starts_with('⚠') {
        Health::Problem
    } else {
        Health::Idle
    }
}

/// The node's status text (without an icon), including its provider. For
/// task cards this is the state word, its round and the provider, read
/// through the shared interpretation (taskinfo); what the agent is doing
/// and why it waits have their own line and never crowd the state out.
fn describe(view: &View, key: &str) -> String {
    let agent = view.agents.get(key);
    let provider = agent.map(|agent| agent.provider.as_str());
    if let Some((_, id)) = key.split_once(':') {
        let Some(task) = view.tasks.iter().find(|task| task.id == id) else {
            return "?".into();
        };
        let status = if task.status == "verifying" {
            format!("verifier r{}", task.round)
        } else {
            format!("{} · worker", taskinfo::worker_status(task))
        };
        let active = if task.status == "verifying" {
            "verifier"
        } else {
            "worker"
        };
        let provider = view
            .agents
            .get(&format!("{active}:{id}"))
            .map(|agent| agent.provider.as_str());
        return taskinfo::with_provider(&status, provider);
    }
    let working = health(view, key) == Health::Working;
    // The live-activity rule (only while running, never the placeholder)
    // is taskinfo's, the one every view reads the same way.
    let doing = taskinfo::doing(view, key);
    let text = if working {
        "working".to_owned()
    } else {
        let activity = agent.map_or("", |agent| agent.activity.as_str());
        let activity = activity.trim_start_matches(['✗', '⚠', '✓', ' ']);
        if activity.is_empty() {
            "idle".to_owned()
        } else {
            activity.to_owned()
        }
    };
    let text = taskinfo::with_provider(&text, provider);
    match doing {
        Some(doing) => format!("{text} · {doing}"),
        None => text,
    }
}

/// The card's activity line: what this role is doing, the dependencies it
/// waits for, or the cause it stopped with — one line of its own, shared
/// with details through taskinfo, and independent of the
/// state word and the provider.
fn activity_line(view: &View, key: &str) -> Option<String> {
    let (_, id) = key.split_once(':')?;
    let task = view.tasks.iter().find(|task| task.id == id)?;
    taskinfo::task_activity(view, task)
}

/// The same bounded, grapheme-safe text drives both layout and rendering.
fn card_lines(view: &View, key: &str) -> Vec<String> {
    let normalize = |text: &str| clean(text).split_whitespace().collect::<Vec<_>>().join(" ");
    let text = match key {
        ORCHESTRATOR => "Coordinates the plan.".to_owned(),
        PLAN_VERIFIER => "Checks the plan.".to_owned(),
        AUDITOR => "Audits task results.".to_owned(),
        super::super::COMMENT_ORCHESTRATOR => "Addresses comments.".to_owned(),
        _ => {
            let Some((role, task)) = key.split_once(':').and_then(|(role, id)| {
                view.tasks
                    .iter()
                    .find(|task| task.id == id)
                    .map(|task| (role, task))
            }) else {
                return vec![normalize(&agent_name(key))];
            };
            let summary = if role == "worker" {
                &task.summary
            } else {
                &task.verifier_summary
            };
            summary
                .as_deref()
                .map(normalize)
                .filter(|text| !text.is_empty())
                .unwrap_or_else(|| {
                    let label = [&task.title, &task.kind, &task.id]
                        .into_iter()
                        .map(|text| normalize(text))
                        .find(|text| !text.is_empty())
                        .unwrap_or_else(|| "task".to_owned());
                    if role == "worker" {
                        label
                    } else {
                        format!("Verify {label}.")
                    }
                })
        }
    };
    let width = usize::from(WIDTH - 4);
    fit_lines(&text, width, MAX_SUMMARY_LINES)
}

/// The same bounded, grapheme-safe wrapping the summary uses: at most
/// `max` lines of `width` columns, the cut line forced to end in `…`.
fn fit_lines(text: &str, width: usize, max: usize) -> Vec<String> {
    let mut lines = wrap(text, width);
    if lines.len() > max {
        lines.truncate(max);
        let last = lines
            .last_mut()
            .expect("a wrapped text has at least one line");
        // Force an ellipsis even when the final visible line has spare columns.
        let mut used = 0;
        let prefix: String = last
            .trim_end()
            .graphemes(true)
            .take_while(|g| {
                used += g.width();
                used < width
            })
            .collect();
        *last = format!("{prefix}…");
    }
    lines
}

/// The worker's effect in its own fixed slot under the summary: never a
/// substitute for a summary line, never a growth of the card. The card's
/// shape of the shared reading (taskinfo) — text and fragment marker in
/// front — wrapped to the slot; `None` keeps the slot empty until the
/// round has something to report at all.
fn effect_lines(view: &View, key: &str) -> Vec<String> {
    let Some(row) = key.split_once(':').and_then(|(role, id)| {
        (role == "worker")
            .then(|| view.tasks.iter().find(|task| task.id == id))
            .flatten()
    }) else {
        return Vec::new();
    };
    let Some(effect) = taskinfo::effect_front(row) else {
        return Vec::new();
    };
    fit_lines(&effect, usize::from(WIDTH - 4), MAX_EFFECT_LINES)
}

pub fn mark(status: &str) -> (&'static str, Color) {
    match status {
        "approved" => ("✓", Color::Green),
        "needs_user" | "waiting" => ("!", Color::Yellow),
        "failed" => ("✗", Color::Red),
        "skipped" | "cancelled" | "revised" => ("–", Color::DarkGray),
        "working" | "verifying" => ("●", Color::Cyan),
        _ => ("○", Color::Gray),
    }
}

fn draw_node(
    view: &View,
    canvas: &mut Canvas,
    node: &Node,
    selected: bool,
    theme: &Theme,
    tick: u64,
) {
    if node.x >= canvas.origin_x.saturating_add(canvas.width)
        || node.x.saturating_add(WIDTH) <= canvas.origin_x
        || node.y >= canvas.origin_y.saturating_add(canvas.height)
        || node.y.saturating_add(node.height) <= canvas.origin_y
    {
        return;
    }
    let health = health(view, &node.key);
    let task = node
        .key
        .split_once(':')
        .and_then(|(_, id)| view.tasks.iter().find(|task| task.id == id));
    let accent = theme.role_accent(&node.key, task.map_or("", |task| task.kind.as_str()));
    let fill = if selected {
        theme.raised()
    } else {
        theme.card()
    };
    let (icon, tone) = match health {
        Health::Working => (theme.spinner(tick), theme.status("working")),
        Health::Problem => (theme.glyphs().problem, theme.error()),
        Health::Waiting => (theme.glyphs().waiting, theme.warning()),
        Health::Done => (theme.glyphs().done, theme.status("approved")),
        Health::Idle => (theme.glyphs().queued, theme.dim()),
    };
    let title_style = match health {
        Health::Problem => theme.error(),
        Health::Waiting => theme.warning(),
        _ => accent,
    }
    .add_modifier(Modifier::BOLD);
    let (x, y) = (node.x, node.y);
    let right = x + WIDTH - 1;
    let bottom = y.saturating_add(node.height).saturating_sub(1);
    // The card is its background; no frame is drawn around it.
    for cy in y..=bottom {
        for cx in x..=right {
            canvas.put(cx, cy, ' ', fill);
        }
        // A bar at the left edge carries the agent kind (and the selection
        // when colors are off).
        let bar = if selected { '█' } else { '▌' };
        canvas.put(x, cy, bar, fill.patch(accent));
    }
    let inner = usize::from(WIDTH - 4);
    let name = task
        .map(|task| clean(&task.title))
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| agent_name(&node.key));
    let title = if node.collapsed {
        format!("{} [+]", truncate(&name, inner.saturating_sub(7)))
    } else {
        truncate(&name, inner.saturating_sub(2))
    };
    canvas.text(x + 2, y, theme.role_icon(&node.key), fill.patch(accent));
    canvas.text(
        x + 4,
        y,
        &truncate(&title, inner.saturating_sub(2)),
        fill.patch(title_style),
    );
    let open = view
        .comments_on(&node.key)
        .iter()
        .filter(|comment| comment.status == "open")
        .count();
    if open > 0 {
        canvas.text(
            right - 3,
            y,
            &format!("✎{}", open.min(9)),
            fill.patch(theme.accent()),
        );
    }
    let kind = task.map_or("orchestration", |task| {
        if task.kind.is_empty() {
            "unknown"
        } else {
            task.kind.as_str()
        }
    });
    canvas.text(
        x + 2,
        y + 1,
        &truncate(&format!("kind: {kind}"), inner),
        fill.patch(accent),
    );
    canvas.text(x + 2, y + 2, icon, fill.patch(tone));
    canvas.text(
        x + 4,
        y + 2,
        &truncate(&describe(view, &node.key), inner.saturating_sub(2)),
        fill.patch(if health == Health::Idle {
            theme.dim()
        } else {
            tone
        }),
    );
    if let Some(task) = task {
        canvas.text(
            x + 2,
            y + 3,
            &truncate(
                &format!(
                    "V: {}{}",
                    taskinfo::verifier_status(task),
                    if task.obligations.is_some() {
                        format!(" · {}", taskinfo::coverage_short(task))
                    } else {
                        String::new()
                    }
                ),
                inner,
            ),
            fill.patch(tone),
        );
    }
    if node.collapsed {
        return;
    }
    // The activity line: what this role is doing, waits for, or stopped
    // with, on its own row — status and provider cannot crowd it out.
    if let Some(line) = activity_line(view, &node.key) {
        let style = match health {
            Health::Problem => theme.error(),
            Health::Waiting => theme.warning(),
            _ => theme.dim(),
        };
        canvas.text(
            x + 2,
            y + ACTIVITY_ROW,
            &truncate(&line, inner),
            fill.patch(style),
        );
    }
    let summary = if let Some(task) = task.filter(|task| {
        task.verifier_summary
            .as_ref()
            .is_some_and(|s| !clean(s).trim().is_empty())
    }) {
        let mut lines = fit_lines(
            &clean(
                task.summary
                    .as_deref()
                    .filter(|text| !clean(text).trim().is_empty())
                    .unwrap_or(&task.title),
            )
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
            inner,
            MAX_SUMMARY_LINES / 2,
        );
        lines.extend(fit_lines(
            &clean(task.verifier_summary.as_deref().unwrap_or_default())
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" "),
            inner,
            MAX_SUMMARY_LINES / 2,
        ));
        lines
    } else {
        card_lines(view, &node.key)
    };
    for (row, line) in summary.iter().enumerate() {
        canvas.text(
            x + 2,
            y + BODY_ROW + row as u16,
            line,
            fill.patch(theme.text()),
        );
    }
    // The worker's effect keeps its own slot under the summary, so it
    // replaces no summary line and the card never changes height for it.
    for (row, line) in effect_lines(view, &node.key).iter().enumerate() {
        canvas.text(
            x + 2,
            y + EFFECT_ROW + row as u16,
            line,
            fill.patch(theme.text()),
        );
    }
    // The meta line: what the verifier accounts for, how the result landed,
    // the concrete dependencies still open, and any messages in between.
    // Parts that do not fit drop from the right; nothing runs over an edge.
    if let Some(t) = task {
        let role = node.key.split_once(':').map_or("", |(role, _)| role);
        let meta = taskinfo::card_meta(role, view, t, inner);
        if !meta.is_empty() {
            canvas.text(
                x + 2,
                bottom,
                &truncate(&meta, inner),
                fill.patch(theme.faint()),
            );
        }
    }
}

const UP: u8 = 1;
const DOWN: u8 = 2;
const LEFT: u8 = 4;
const RIGHT: u8 = 8;

#[derive(Debug, Clone)]
struct Cell {
    lines: u8,
    ch: Option<String>,
    style: Style,
}

/// A character grid for rendering graph nodes and orthogonal edges.
/// Memory is strictly bounded to the viewport window when windowing is used.
pub struct Canvas {
    origin_x: u16,
    origin_y: u16,
    width: u16,
    height: u16,
    cells: Vec<Cell>,
}

impl Canvas {
    pub fn new(width: u16, height: u16) -> Self {
        let cap_w = width.min(2048);
        let cap_h = height.min(1024);
        Self::new_window(0, 0, cap_w, cap_h)
    }

    pub fn new_window(origin_x: u16, origin_y: u16, width: u16, height: u16) -> Self {
        Canvas {
            origin_x,
            origin_y,
            width,
            height,
            cells: vec![
                Cell {
                    lines: 0,
                    ch: None,
                    style: Style::default(),
                };
                usize::from(width) * usize::from(height)
            ],
        }
    }

    fn cell(&mut self, x: u16, y: u16) -> Option<&mut Cell> {
        if x >= self.origin_x
            && x < self.origin_x.saturating_add(self.width)
            && y >= self.origin_y
            && y < self.origin_y.saturating_add(self.height)
        {
            let lx = usize::from(x - self.origin_x);
            let ly = usize::from(y - self.origin_y);
            Some(&mut self.cells[ly * usize::from(self.width) + lx])
        } else {
            None
        }
    }

    fn get_cell(&self, x: u16, y: u16) -> Option<&Cell> {
        if x >= self.origin_x
            && x < self.origin_x.saturating_add(self.width)
            && y >= self.origin_y
            && y < self.origin_y.saturating_add(self.height)
        {
            let lx = usize::from(x - self.origin_x);
            let ly = usize::from(y - self.origin_y);
            Some(&self.cells[ly * usize::from(self.width) + lx])
        } else {
            None
        }
    }

    fn put(&mut self, x: u16, y: u16, ch: char, style: Style) {
        if let Some(cell) = self.cell(x, y) {
            cell.ch = Some(ch.to_string());
            cell.style = style;
        }
    }

    fn text(&mut self, x: u16, y: u16, text: &str, style: Style) {
        let mut cx = x;
        for grapheme in text.graphemes(true) {
            let width = UnicodeWidthStr::width(grapheme) as u16;
            // A clipped glyph must never leave half an emoji or combining
            // sequence at either viewport edge.
            if cx >= self.origin_x
                && cx.saturating_add(width) <= self.origin_x.saturating_add(self.width)
            {
                if let Some(cell) = self.cell(cx, y) {
                    cell.ch = Some(grapheme.to_owned());
                    cell.style = style;
                }
                for offset in 1..width {
                    self.put(cx.saturating_add(offset), y, ' ', style);
                }
            }
            cx = cx.saturating_add(width);
        }
    }

    /// Joins line segments so crossings and corners get box-drawing joints.
    fn path(&mut self, points: &[(u16, u16)], style: Style) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        for pair in points.windows(2) {
            let ((x, y), (tx, ty)) = (pair[0], pair[1]);
            // Routes are orthogonal. Keep x-then-y semantics for any diagonal
            // input while visiting only cells inside the viewport.
            if x != tx && y >= self.origin_y && y < self.origin_y.saturating_add(self.height) {
                let lo = x.min(tx);
                let hi = x.max(tx);
                let start = lo.max(self.origin_x);
                let end = hi.min(self.origin_x.saturating_add(self.width).saturating_sub(1));
                for cx in start..=end {
                    if let Some(cell) = self.cell(cx, y) {
                        cell.lines |=
                            if cx > lo { LEFT } else { 0 } | if cx < hi { RIGHT } else { 0 };
                        cell.style = style;
                    }
                }
            }
            if y != ty && tx >= self.origin_x && tx < self.origin_x.saturating_add(self.width) {
                let lo = y.min(ty);
                let hi = y.max(ty);
                let start = lo.max(self.origin_y);
                let end = hi.min(self.origin_y.saturating_add(self.height).saturating_sub(1));
                for cy in start..=end {
                    if let Some(cell) = self.cell(tx, cy) {
                        cell.lines |= if cy > lo { UP } else { 0 } | if cy < hi { DOWN } else { 0 };
                        cell.style = style;
                    }
                }
            }
        }
    }

    fn glyph(cell: &Cell) -> &str {
        if let Some(ch) = &cell.ch {
            return ch;
        }
        match cell.lines {
            0 => " ",
            UP | DOWN => "│",
            LEFT | RIGHT => "─",
            lines if lines == UP | DOWN => "│",
            lines if lines == LEFT | RIGHT => "─",
            lines if lines == DOWN | RIGHT => "┌",
            lines if lines == DOWN | LEFT => "┐",
            lines if lines == UP | RIGHT => "└",
            lines if lines == UP | LEFT => "┘",
            lines if lines == UP | DOWN | RIGHT => "├",
            lines if lines == UP | DOWN | LEFT => "┤",
            lines if lines == LEFT | RIGHT | DOWN => "┬",
            lines if lines == LEFT | RIGHT | UP => "┴",
            _ => "┼",
        }
    }

    /// Draws the window at (`left`, `top`) of the canvas into `area`.
    pub fn show(&self, buffer: &mut Buffer, area: Rect, left: u16, top: u16) {
        for row in 0..area.height {
            for column in 0..area.width {
                let (x, y) = (left + column, top + row);
                if let Some(cell) = self.get_cell(x, y)
                    && let Some(target) = buffer.cell_mut((area.x + column, area.y + row))
                {
                    target.set_symbol(Self::glyph(cell)).set_style(cell.style);
                }
            }
        }
    }

    pub fn lines(&self) -> Vec<String> {
        (0..self.height)
            .map(|y| {
                let line: String = (0..self.width)
                    .map(|x| {
                        Self::glyph(
                            &self.cells[usize::from(y) * usize::from(self.width) + usize::from(x)],
                        )
                    })
                    .collect();
                line.trim_end().to_owned()
            })
            .collect()
    }
}

/// Computes the overlay rectangle for the 'now' progress box so that
/// it does not collide with the currently selected node in the viewport.
pub fn compute_progress_overlay(
    inner: Rect,
    selected_node: Option<&Node>,
    offset: (u16, u16),
    progress_width: u16,
    progress_height: u16,
) -> Option<Rect> {
    compute_progress_overlay_with_nodes(
        inner,
        selected_node,
        &[],
        offset,
        progress_width,
        progress_height,
    )
}

/// Computes the overlay rectangle for the 'now' progress box checking against
/// both selected node and all visible nodes in the viewport.
///
/// Default position is bottom-right. If collisions exist, it tests top-right,
/// top-left, and bottom-left. If all corners collide, it yields and returns None.
pub fn compute_progress_overlay_with_nodes(
    inner: Rect,
    selected_node: Option<&Node>,
    visible_nodes: &[&Node],
    offset: (u16, u16),
    progress_width: u16,
    progress_height: u16,
) -> Option<Rect> {
    if inner.height < progress_height + 2 || inner.width < progress_width {
        return None;
    }
    let default_x = inner.right().saturating_sub(progress_width);
    let bottom_y = inner.bottom().saturating_sub(progress_height);

    let candidates = [
        Rect::new(default_x, bottom_y, progress_width, progress_height), // bottom-right
        Rect::new(default_x, inner.y, progress_width, progress_height),  // top-right
        Rect::new(inner.x, inner.y, progress_width, progress_height),    // top-left
        Rect::new(inner.x, bottom_y, progress_width, progress_height),   // bottom-left
    ];

    for overlay in candidates {
        let collides_selected = selected_node.filter(|n| n.visible).is_some_and(|node| {
            let screen_x = inner.x as i32 + node.x as i32 - offset.0 as i32;
            let screen_y = inner.y as i32 + node.y as i32 - offset.1 as i32;
            let node_rect = Rect::new(
                screen_x.max(0) as u16,
                screen_y.max(0) as u16,
                WIDTH,
                node.height,
            );
            node_rect.intersects(overlay)
        });
        if collides_selected {
            continue;
        }

        let collides_any_visible = visible_nodes.iter().any(|node| {
            let screen_x = inner.x as i32 + node.x as i32 - offset.0 as i32;
            let screen_y = inner.y as i32 + node.y as i32 - offset.1 as i32;
            if screen_x + WIDTH as i32 > inner.x as i32
                && screen_x < inner.right() as i32
                && screen_y + node.height as i32 > inner.y as i32
                && screen_y < inner.bottom() as i32
            {
                let node_rect = Rect::new(
                    screen_x.max(0) as u16,
                    screen_y.max(0) as u16,
                    WIDTH,
                    node.height,
                );
                node_rect.intersects(overlay)
            } else {
                false
            }
        });
        if !collides_any_visible {
            return Some(overlay);
        }
    }
    None
}

#[cfg(test)]
mod tests {
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
        let style = ratatui::style::Style::default().fg(ratatui::style::Color::Cyan);
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

    use super::*;
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
                    let filtered = Graph::new_with_viewport(
                        &view,
                        Some(query),
                        &collapsed,
                        grouping,
                        viewport,
                    );
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
                    assert!(rows[1].contains("kind: implementation"), "{rows:?}");
                } else {
                    assert!(rows[1].contains("kind: orchestration"), "{rows:?}");
                }
            }
        }
    }

    #[test]
    fn summaries_are_bounded_and_truncated_at_grapheme_boundaries() {
        let mut row = task("t1", &[]);
        row.summary = Some("👩‍💻e\u{301}".repeat(100));
        row.verifier_summary = Some("Check the summary.".into());
        let view = view(vec![row]);
        let lines = card_lines(&view, "worker:t1");
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
            assert_eq!(card_lines(&view, "worker:t1"), ["Storage"]);
            assert_eq!(card_lines(&view, "verifier:t1"), ["Verify Storage."]);
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
            card_lines(&view(vec![row.clone()]), "worker:t1"),
            ["analysis"]
        );
        row.kind.clear();
        assert_eq!(card_lines(&view(vec![row]), "verifier:t1"), ["Verify t1."]);
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
            assert_eq!(card_lines(&view, key), [description]);
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
        assert_eq!(at("worker:t1"), (column_x(0), row_y(1)));
        assert_eq!(at("verifier:t1"), at("worker:t1"));
        assert_eq!(at("worker:t3"), (column_x(0), row_y(2)));
        assert_eq!(at("worker:t2"), (column_x(1), row_y(1)));
        assert_eq!(at(ORCHESTRATOR), (column_x(1), 0));
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
            !rows[3].contains("codex"),
            "the activity line is independent of the provider: {}",
            rows[3]
        );
        assert!(
            rows[usize::from(ACTIVITY_ROW)].contains("doing: cargo check"),
            "{}",
            rows[3]
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
        assert_eq!(BODY_ROW, ACTIVITY_ROW + 1);
        assert_eq!(EFFECT_ROW, BODY_ROW + MAX_SUMMARY_LINES as u16);
        assert_eq!(HEIGHT, EFFECT_ROW + MAX_EFFECT_LINES as u16 + 1);
        let mut row = task("t1", &["t0"]);
        row.summary = Some(
            "Repair the storage engine so a restart never loses a queued write, at all.".into(),
        );
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
        view.tasks[1].integration = Some(json!({"outcome": "committed", "detail": "as abc12"}));
        let rows = render(&view);
        assert_eq!(
            effect_lines(&view, "worker:t1").join(""),
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

    #[test]
    fn ac_resp_04_progress_overlay_collision_flip() {
        let inner = Rect::new(0, 0, 80, 25);
        let progress_width = 30;
        let progress_height = 6;

        // 1. Node in bottom-right corner intersects default bottom-right overlay
        // Default bottom-right overlay: x = 80 - 30 = 50, y = 25 - 6 = 19, w = 30, h = 6.
        let colliding_node = Node {
            key: "worker:t1".into(),
            x: 52,
            y: 20,
            ..Default::default()
        };
        let overlay = compute_progress_overlay(
            inner,
            Some(&colliding_node),
            (0, 0),
            progress_width,
            progress_height,
        )
        .expect("overlay fits");
        // Flips to top-right corner without obscuring selection
        assert_eq!(overlay.x, 50);
        assert_eq!(overlay.y, inner.y);

        // 2. Node in top-left does NOT collide with bottom-right overlay
        let non_colliding_node = Node {
            key: "orchestrator".into(),
            x: 5,
            y: 2,
            ..Default::default()
        };
        let overlay2 = compute_progress_overlay(
            inner,
            Some(&non_colliding_node),
            (0, 0),
            progress_width,
            progress_height,
        )
        .expect("overlay fits");
        // Stays at bottom-right
        assert_eq!(overlay2.x, 50);
        assert_eq!(overlay2.y, 19);

        // 3. Tiny area returns None
        let tiny = Rect::new(0, 0, 20, 5);
        assert!(
            compute_progress_overlay(tiny, None, (0, 0), progress_width, progress_height).is_none()
        );

        // 4. Centered node intersecting all 4 potential overlay placements returns None
        let small_inner = Rect::new(0, 0, 40, 10);
        let center_node = Node {
            key: "worker:t2".into(),
            x: 10,
            y: 2,
            ..Default::default()
        };
        assert!(
            compute_progress_overlay(small_inner, Some(&center_node), (0, 0), 25, 5,).is_none(),
            "when all four corners intersect selected node, overlay is hidden"
        );
    }
}
