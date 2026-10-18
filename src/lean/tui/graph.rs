//! The team as a two-dimensional graph: boxes for agents, orthogonal edges for
//! what relates them. The layout depends only on the plan's shape, so nodes
//! stay put while events arrive.

use super::view::{AUDITOR, ORCHESTRATOR, PLAN_VERIFIER, View, agent_name, polish, truncate};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use std::collections::HashMap;
use std::time::Duration;

pub const WIDTH: u16 = 26;
pub const HEIGHT: u16 = 4;
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
            Self::All => "wszystko",
            Self::Depends => "zależności",
            Self::Verifies => "weryfikacja",
            Self::Mail => "komunikacja",
            Self::Supervises => "nadzór",
        }
    }

    /// Supervision edges reach every worker, so only their own filter shows
    /// them.
    fn shows(self, kind: Kind) -> bool {
        match self {
            Self::All => kind != Kind::Supervises,
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
            Self::Dependency => "zależności",
            Self::Status => "stan",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub key: String,
    pub x: u16,
    pub y: u16,
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
            visible: true,
            group: None,
            collapsed: false,
        }
    }

    fn mid_y(&self) -> u16 {
        self.y + HEIGHT / 2
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
                            || t.goal.to_lowercase().contains(&q_lower)
                            || t.focus.to_lowercase().contains(&q_lower)
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
            let is_task_collapsed =
                collapsed.contains(id) || collapsed.contains(&format!("task:{id}"));
            let worker_vis = matches_query(&format!("worker:{id}"));
            let verifier_vis = !is_task_collapsed && matches_query(&format!("verifier:{id}"));
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

        if let Some((vp_w, vp_h)) = viewport {
            let max_cols = ((vp_w.saturating_sub(MARGIN) + GAP_X) / (WIDTH + GAP_X)).max(1);
            let visible_rows = ((vp_h + GAP_Y) / (HEIGHT + GAP_Y)).max(1);
            let mut col_next_row: Vec<u16> = vec![0; max_cols as usize];

            let mut place_admin = |key: &str, col: u16, row: u16, vis: bool| {
                graph.nodes.push(Node {
                    key: key.to_string(),
                    x: if vis { column_x(col) } else { 0 },
                    y: if vis { row_y(row) } else { 0 },
                    visible: vis,
                    group: None,
                    collapsed: false,
                });
            };

            if max_cols >= 3 {
                if pv_vis {
                    place_admin(PLAN_VERIFIER, 0, 0, true);
                    col_next_row[0] = 1;
                } else {
                    place_admin(PLAN_VERIFIER, 0, 0, false);
                }
                if orch_vis {
                    place_admin(ORCHESTRATOR, 1, 0, true);
                    col_next_row[1] = 1;
                } else {
                    place_admin(ORCHESTRATOR, 0, 0, false);
                }
                if aud_vis {
                    place_admin(AUDITOR, 2, 0, true);
                    col_next_row[2] = 1;
                } else {
                    place_admin(AUDITOR, 0, 0, false);
                }
            } else if max_cols == 2 {
                if pv_vis {
                    place_admin(PLAN_VERIFIER, 0, 0, true);
                    col_next_row[0] = 1;
                } else {
                    place_admin(PLAN_VERIFIER, 0, 0, false);
                }
                if orch_vis && aud_vis {
                    place_admin(ORCHESTRATOR, 1, 0, true);
                    place_admin(AUDITOR, 1, 1, true);
                    col_next_row[1] = 2;
                } else if orch_vis {
                    place_admin(ORCHESTRATOR, 1, 0, true);
                    place_admin(AUDITOR, 0, 0, false);
                    col_next_row[1] = 1;
                } else if aud_vis {
                    place_admin(AUDITOR, 1, 0, true);
                    place_admin(ORCHESTRATOR, 0, 0, false);
                    col_next_row[1] = 1;
                } else {
                    place_admin(ORCHESTRATOR, 0, 0, false);
                    place_admin(AUDITOR, 0, 0, false);
                }
            } else {
                if orch_vis {
                    place_admin(ORCHESTRATOR, 0, 0, true);
                    col_next_row[0] = 1;
                } else {
                    place_admin(ORCHESTRATOR, 0, 0, false);
                }
                place_admin(PLAN_VERIFIER, 1, 0, pv_vis);
                place_admin(AUDITOR, 1, 1, aud_vis);
            }

            for &rel_depth in &sorted_rel_depths {
                let tasks_at_depth = rel_depth_tasks.get_mut(&rel_depth).unwrap();
                tasks_at_depth.sort_unstable();

                for (slot, &task_idx) in tasks_at_depth.iter().enumerate() {
                    placed_tasks.insert(task_idx);
                    let task = &view.tasks[task_idx];
                    let id = &task.id;
                    let is_task_collapsed =
                        collapsed.contains(id) || collapsed.contains(&format!("task:{id}"));
                    let worker_key = format!("worker:{id}");
                    let verifier_key = format!("verifier:{id}");
                    let worker_vis = matches_query(&worker_key);
                    let verifier_vis = !is_task_collapsed && matches_query(&verifier_key);
                    let task_rows =
                        (if worker_vis { 1 } else { 0 }) + (if verifier_vis { 1 } else { 0 });

                    let pref_col = (rel_depth + slot as u16) % max_cols;
                    let target_col = if col_next_row[pref_col as usize] + task_rows <= visible_rows
                    {
                        pref_col
                    } else if let Some(alt_col) = (0..max_cols)
                        .find(|&c| col_next_row[c as usize] + task_rows <= visible_rows)
                    {
                        alt_col
                    } else {
                        (0..max_cols)
                            .min_by_key(|&c| (col_next_row[c as usize], c != pref_col))
                            .unwrap_or(pref_col)
                    };

                    let row = col_next_row[target_col as usize];

                    let mut worker = Node::new(worker_key, column_x(target_col), row_y(row));
                    worker.collapsed = is_task_collapsed;
                    worker.group = Some(format!("task:{id}"));
                    worker.visible = worker_vis;
                    graph.nodes.push(worker);

                    let next_row = if worker_vis { row + 1 } else { row };

                    let mut verifier =
                        Node::new(verifier_key, column_x(target_col), row_y(next_row));
                    verifier.group = Some(format!("task:{id}"));
                    verifier.visible = verifier_vis;
                    graph.nodes.push(verifier);

                    let end_row = if verifier_vis { next_row + 1 } else { next_row };
                    col_next_row[target_col as usize] = end_row;
                }
            }
        } else {
            let mut place_admin = |key: &str, col: u16, row: u16, vis: bool| {
                graph.nodes.push(Node {
                    key: key.to_string(),
                    x: if vis { column_x(col) } else { 0 },
                    y: if vis { row_y(row) } else { 0 },
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
                let mut current_row = if admin_vis { 1 } else { 0 };

                for &task_idx in tasks_at_depth.iter() {
                    placed_tasks.insert(task_idx);
                    let task = &view.tasks[task_idx];
                    let id = &task.id;
                    let is_task_collapsed =
                        collapsed.contains(id) || collapsed.contains(&format!("task:{id}"));
                    let worker_key = format!("worker:{id}");
                    let verifier_key = format!("verifier:{id}");
                    let worker_vis = matches_query(&worker_key);
                    let verifier_vis = !is_task_collapsed && matches_query(&verifier_key);

                    let mut worker = Node::new(worker_key, column_x(rel_depth), row_y(current_row));
                    worker.collapsed = is_task_collapsed;
                    worker.group = Some(format!("task:{id}"));
                    worker.visible = worker_vis;
                    graph.nodes.push(worker);

                    let next_row = if worker_vis {
                        current_row + 1
                    } else {
                        current_row
                    };

                    let mut verifier =
                        Node::new(verifier_key, column_x(rel_depth), row_y(next_row));
                    verifier.group = Some(format!("task:{id}"));
                    verifier.visible = verifier_vis;
                    graph.nodes.push(verifier);

                    current_row = if verifier_vis { next_row + 1 } else { next_row };
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
                    visible: false,
                    group: Some(format!("task:{id}")),
                    collapsed: is_task_collapsed,
                });
                graph.nodes.push(Node {
                    key: format!("verifier:{id}"),
                    x: 0,
                    y: 0,
                    visible: false,
                    group: Some(format!("task:{id}")),
                    collapsed: false,
                });
            }
        }

        let mut edge = |from: String, to: String, kind: Kind, label: Option<String>| {
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
            edge(
                format!("verifier:{}", task.id),
                worker.clone(),
                Kind::Verifies,
                Some("V".into()),
            );
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
        let mut links: Vec<_> = view.links.iter().collect();
        links.sort_by(|a, b| a.0.cmp(b.0));
        for ((from, to), link) in links {
            edge(
                from.clone(),
                to.clone(),
                Kind::Mail {
                    recent: link.last.elapsed() < RECENT,
                },
                Some(format!("M✉{}", link.count)),
            );
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
            .map(|node| node.y + HEIGHT)
            .max()
            .unwrap_or(0);
        graph
    }

    pub fn node(&self, key: &str) -> Option<&Node> {
        self.nodes.iter().find(|node| node.key == key)
    }

    /// The nearest node from `key` in a direction (dx, dy), each -1, 0 or 1.
    /// Only navigates between visible nodes.
    pub fn neighbour(&self, key: &str, dx: i32, dy: i32) -> Option<String> {
        let from = self.node(key)?;
        let center = |node: &Node| {
            (
                i32::from(node.x) + i32::from(WIDTH) / 2,
                i32::from(node.y) + i32::from(HEIGHT) / 2,
            )
        };
        let (fx, fy) = center(from);
        self.nodes
            .iter()
            .filter(|node| node.visible && node.key != key)
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
            let is_adjacent = (from.y as i32 - to.y as i32).abs() <= (HEIGHT + GAP_Y) as i32;
            if is_adjacent {
                let x = to.x + WIDTH / 2;
                if from.y > to.y {
                    return vec![(x, from.y - 1), (x, to.y + HEIGHT)];
                }
                if from.y < to.y {
                    return vec![(x, from.y + HEIGHT), (x, to.y.saturating_sub(1))];
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
                HEIGHT
            } else if to.y > from.y {
                to.y.saturating_sub(1)
            } else {
                to.y + HEIGHT
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
        for pair in path.windows(2) {
            let (p1, p2) = (pair[0], pair[1]);
            let x_min = p1.0.min(p2.0);
            let x_max = p1.0.max(p2.0);
            let y_min = p1.1.min(p2.1);
            let y_max = p1.1.max(p2.1);
            for x in x_min..=x_max {
                for y in y_min..=y_max {
                    if x >= node.x && x < node.x + WIDTH && y >= node.y && y < node.y + HEIGHT {
                        return true;
                    }
                }
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
        for node in self.nodes.iter().filter(|n| n.visible) {
            draw_node(view, canvas, node, node.key == selected);
        }
    }
}

/// What a node shows: its border color, mark, status line and detail line.
fn describe(view: &View, key: &str) -> (Color, String, String) {
    let agent = view.agents.get(key);
    let provider = agent
        .map(|agent| agent.provider.as_str())
        .filter(|provider| !provider.is_empty());
    let with_provider = |text: String| match provider {
        Some(provider) => format!("{text} · {provider}"),
        None => text,
    };
    let activity = agent.map_or("", |agent| agent.activity.as_str());
    if let Some((who, id)) = key.split_once(':') {
        let Some(task) = view.tasks.iter().find(|task| task.id == id) else {
            return (Color::DarkGray, "?".into(), String::new());
        };
        let (mark, color) = mark(&task.status);
        let round = if task.round > 1 {
            format!(" r{}", task.round)
        } else {
            String::new()
        };
        return if who == "worker" {
            let active = task.status == "working";
            let state = if task.status == "verifying" {
                format!("○ czeka na werdykt{round}")
            } else {
                format!("{mark} {}{round}", polish(&task.status))
            };
            (
                if active { Color::Cyan } else { color },
                with_provider(state),
                if active && !activity.is_empty() {
                    activity.to_owned()
                } else {
                    task.title.clone()
                },
            )
        } else {
            let active = task.status == "verifying";
            let state = match task.status.as_str() {
                "verifying" => "● sprawdza".to_owned(),
                "approved" => "✓ zatwierdził".to_owned(),
                "needs_user" | "failed" => "✗ odrzucił".to_owned(),
                "proposed" | "queued" | "working" => "○ czeka na workera".to_owned(),
                other => format!("– {}", polish(other)),
            };
            (
                if active { Color::Cyan } else { color },
                with_provider(state),
                if active && !activity.is_empty() {
                    activity.to_owned()
                } else {
                    task.focus.clone()
                },
            )
        };
    }
    let running = agent.is_some_and(|agent| agent.running);
    let state = if running {
        "● pracuje".to_owned()
    } else if activity.is_empty() {
        "○ bezczynny".to_owned()
    } else {
        activity.to_owned()
    };
    (
        if running { Color::Cyan } else { Color::Gray },
        with_provider(state),
        if running {
            activity.to_owned()
        } else {
            String::new()
        },
    )
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

fn draw_node(view: &View, canvas: &mut Canvas, node: &Node, selected: bool) {
    let (color, state, detail) = describe(view, &node.key);
    let mut border = Style::default().fg(color);
    if selected {
        border = border.fg(Color::White).add_modifier(Modifier::BOLD);
    }
    let (h, v, corners) = if selected {
        ('═', '║', ['╔', '╗', '╚', '╝'])
    } else {
        ('─', '│', ['╭', '╮', '╰', '╯'])
    };
    let (x, y) = (node.x, node.y);
    let right = x + WIDTH - 1;
    let bottom = y + HEIGHT - 1;
    for cx in x..=right {
        canvas.put(cx, y, h, border);
        canvas.put(cx, bottom, h, border);
    }
    for cy in y..=bottom {
        canvas.put(x, cy, v, border);
        canvas.put(right, cy, v, border);
    }
    canvas.put(x, y, corners[0], border);
    canvas.put(right, y, corners[1], border);
    canvas.put(x, bottom, corners[2], border);
    canvas.put(right, bottom, corners[3], border);
    let inner = usize::from(WIDTH - 4);
    let title_badge = if node.collapsed {
        format!(
            " {} [+] ",
            truncate(&agent_name(&node.key), inner.saturating_sub(6))
        )
    } else {
        format!(" {} ", truncate(&agent_name(&node.key), inner - 2))
    };
    canvas.text(x + 2, y, &title_badge, border);
    // Badges for relations V/D/M on the bottom border
    let mut rel_badges = String::new();
    if let Some((_, id)) = node.key.split_once(':')
        && let Some(t) = view.tasks.iter().find(|t| t.id == id)
    {
        if !t.depends_on.is_empty() {
            rel_badges.push_str("D ");
        }
        rel_badges.push_str("V ");
        if view
            .links
            .keys()
            .any(|(from, to)| from == &node.key || to == &node.key)
        {
            rel_badges.push('M');
        }
    }
    let rel_badges = rel_badges.trim();
    if !rel_badges.is_empty() && inner >= rel_badges.len() + 2 {
        canvas.text(
            right.saturating_sub(rel_badges.len() as u16 + 2),
            bottom,
            &format!(" {rel_badges} "),
            Style::default().fg(Color::DarkGray),
        );
    }
    for cy in y + 1..bottom {
        for cx in x + 1..right {
            canvas.put(cx, cy, ' ', Style::default());
        }
    }
    let open = view
        .comments_on(&node.key)
        .iter()
        .filter(|comment| comment.status == "open")
        .count();
    if open > 0 {
        canvas.text(
            right - 4,
            y,
            &format!("✎{}", open.min(9)),
            Style::default().fg(Color::Cyan),
        );
    }
    canvas.text(x + 2, y + 1, &truncate(&state, inner), Style::default());
    canvas.text(
        x + 2,
        y + 2,
        &truncate(&detail, inner),
        Style::default().fg(Color::DarkGray),
    );
}

const UP: u8 = 1;
const DOWN: u8 = 2;
const LEFT: u8 = 4;
const RIGHT: u8 = 8;

#[derive(Debug, Clone)]
struct Cell {
    lines: u8,
    ch: Option<char>,
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
            cell.ch = Some(ch);
            cell.style = style;
        }
    }

    fn text(&mut self, x: u16, y: u16, text: &str, style: Style) {
        let mut cx = x;
        for ch in text.chars() {
            self.put(cx, y, ch, style);
            cx += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0) as u16;
        }
    }

    /// Joins line segments so crossings and corners get box-drawing joints.
    fn path(&mut self, points: &[(u16, u16)], style: Style) {
        for pair in points.windows(2) {
            let ((mut x, mut y), (tx, ty)) = (pair[0], pair[1]);
            while (x, y) != (tx, ty) {
                let (nx, ny, out, back) = if x < tx {
                    (x + 1, y, RIGHT, LEFT)
                } else if x > tx {
                    (x - 1, y, LEFT, RIGHT)
                } else if y < ty {
                    (x, y + 1, DOWN, UP)
                } else {
                    (x, y - 1, UP, DOWN)
                };
                for (cx, cy, bit) in [(x, y, out), (nx, ny, back)] {
                    if let Some(cell) = self.cell(cx, cy) {
                        cell.lines |= bit;
                        cell.style = style;
                    }
                }
                (x, y) = (nx, ny);
            }
        }
    }

    fn glyph(cell: &Cell) -> char {
        if let Some(ch) = cell.ch {
            return ch;
        }
        match cell.lines {
            0 => ' ',
            UP | DOWN => '│',
            LEFT | RIGHT => '─',
            lines if lines == UP | DOWN => '│',
            lines if lines == LEFT | RIGHT => '─',
            lines if lines == DOWN | RIGHT => '┌',
            lines if lines == DOWN | LEFT => '┐',
            lines if lines == UP | RIGHT => '└',
            lines if lines == UP | LEFT => '┘',
            lines if lines == UP | DOWN | RIGHT => '├',
            lines if lines == UP | DOWN | LEFT => '┤',
            lines if lines == LEFT | RIGHT | DOWN => '┬',
            lines if lines == LEFT | RIGHT | UP => '┴',
            _ => '┼',
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
                    target.set_char(Self::glyph(cell)).set_style(cell.style);
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

/// Computes the overlay rectangle for the 'teraz' progress box so that
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

/// Computes the overlay rectangle for the 'teraz' progress box checking against
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
                HEIGHT,
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
                && screen_y + HEIGHT as i32 > inner.y as i32
                && screen_y < inner.bottom() as i32
            {
                let node_rect = Rect::new(
                    screen_x.max(0) as u16,
                    screen_y.max(0) as u16,
                    WIDTH,
                    HEIGHT,
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
    fn tasks_sit_in_columns_by_dependency_depth_with_their_verifier_below() {
        let view = view(vec![task("t1", &[]), task("t2", &["t1"]), task("t3", &[])]);
        let graph = Graph::new(&view);
        let at = |key: &str| {
            let node = graph.node(key).unwrap();
            (node.x, node.y)
        };
        assert_eq!(at("worker:t1"), (column_x(0), row_y(1)));
        assert_eq!(at("verifier:t1"), (column_x(0), row_y(2)));
        assert_eq!(at("worker:t3"), (column_x(0), row_y(3)));
        assert_eq!(at("worker:t2"), (column_x(1), row_y(1)));
        assert_eq!(at(ORCHESTRATOR), (column_x(1), 0));
    }

    #[test]
    fn arrows_move_to_the_nearest_node_in_that_direction() {
        let view = view(vec![task("t1", &[]), task("t2", &["t1"]), task("t3", &[])]);
        let graph = Graph::new(&view);
        let go = |from: &str, dx, dy| graph.neighbour(from, dx, dy);
        assert_eq!(go("worker:t1", 1, 0).as_deref(), Some("worker:t2"));
        assert_eq!(go("worker:t1", 0, 1).as_deref(), Some("verifier:t1"));
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
        assert_eq!(chars[usize::from(next.x)], '║', "the selected box");
        // The verifier's edge points up into its worker.
        let x = usize::from(worker.x + WIDTH / 2);
        assert_eq!(
            lines[usize::from(worker.y + HEIGHT)].chars().nth(x),
            Some('▲')
        );
        assert!(lines[usize::from(worker.y)].contains("t1 worker"));
        // A message between them is counted beside the source.
        assert!(lines[usize::from(worker.mid_y() - 1)].contains("✉2"));
        assert!(lines[usize::from(worker.y + 1)].contains("proponowane"));
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
