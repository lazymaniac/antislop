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

#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub key: String,
    pub x: u16,
    pub y: u16,
}

impl Node {
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
}

fn column_x(column: u16) -> u16 {
    MARGIN + column * (WIDTH + GAP_X)
}

fn row_y(row: u16) -> u16 {
    row * (HEIGHT + GAP_Y)
}

impl Graph {
    pub fn new(view: &View) -> Self {
        let mut graph = Graph::default();
        let mut place = |key: String, column: u16, row: u16| {
            graph.nodes.push(Node {
                key,
                x: column_x(column),
                y: row_y(row),
            });
        };
        place(PLAN_VERIFIER.into(), 0, 0);
        place(ORCHESTRATOR.into(), 1, 0);
        place(AUDITOR.into(), 2, 0);

        // A task's column is the length of its longest dependency chain.
        let index: HashMap<&str, usize> = view
            .tasks
            .iter()
            .enumerate()
            .map(|(index, task)| (task.id.as_str(), index))
            .collect();
        let mut depth = vec![None; view.tasks.len()];
        fn depth_of(
            task: usize,
            view: &View,
            index: &HashMap<&str, usize>,
            depth: &mut [Option<u16>],
            seen: usize,
        ) -> u16 {
            if let Some(depth) = depth[task] {
                return depth;
            }
            // A cycle never passes validation; the bound only keeps a
            // malformed event from recursing forever.
            let value = if seen > depth.len() {
                0
            } else {
                view.tasks[task]
                    .depends_on
                    .iter()
                    .filter_map(|id| index.get(id.as_str()))
                    .map(|&parent| depth_of(parent, view, index, depth, seen + 1) + 1)
                    .max()
                    .unwrap_or(0)
            };
            depth[task] = Some(value);
            value
        }
        let mut slots: HashMap<u16, u16> = HashMap::new();
        for task in 0..view.tasks.len() {
            let column = depth_of(task, view, &index, &mut depth, 0);
            let slot = slots.entry(column).or_default();
            let id = &view.tasks[task].id;
            place(format!("worker:{id}"), column, 1 + 2 * *slot);
            place(format!("verifier:{id}"), column, 2 + 2 * *slot);
            *slot += 1;
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
            None,
        );
        edge(AUDITOR.into(), ORCHESTRATOR.into(), Kind::Verifies, None);
        for task in &view.tasks {
            let worker = format!("worker:{}", task.id);
            edge(ORCHESTRATOR.into(), worker.clone(), Kind::Supervises, None);
            edge(
                format!("verifier:{}", task.id),
                worker.clone(),
                Kind::Verifies,
                None,
            );
            for parent in &task.depends_on {
                if index.contains_key(parent.as_str()) {
                    edge(
                        format!("worker:{parent}"),
                        worker.clone(),
                        Kind::Depends,
                        None,
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
                Some(format!("✉{}", link.count)),
            );
        }
        graph.width = graph.nodes.iter().map(Node::right).max().unwrap_or(0) + MARGIN;
        graph.height = graph
            .nodes
            .iter()
            .map(|node| node.y + HEIGHT)
            .max()
            .unwrap_or(0);
        graph
    }

    pub fn node(&self, key: &str) -> Option<&Node> {
        self.nodes.iter().find(|node| node.key == key)
    }

    /// The nearest node from `key` in a direction (dx, dy), each -1, 0 or 1.
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
            .filter(|node| node.key != key)
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
    fn route(from: &Node, to: &Node) -> Vec<(u16, u16)> {
        let half = GAP_X / 2;
        // A verifier sits right below its worker.
        if from.x == to.x && from.y == to.y + HEIGHT + GAP_Y {
            let x = to.x + WIDTH / 2;
            return vec![(x, from.y - 1), (x, to.y + HEIGHT)];
        }
        // Neighbours on one row face each other.
        if from.y == to.y && from.x + WIDTH + GAP_X == to.x {
            return vec![(from.right(), from.mid_y()), (to.x - 1, to.mid_y())];
        }
        if from.y == to.y && to.x + WIDTH + GAP_X == from.x {
            return vec![(from.x - 1, from.mid_y()), (to.right(), to.mid_y())];
        }
        let out = from.right() + half;
        let into = to.x - half;
        let mut path = vec![(from.right(), from.mid_y()), (out, from.mid_y())];
        if out != into {
            let lane = if to.y == 0 { HEIGHT } else { to.y - 1 };
            path.push((out, lane));
            path.push((into, lane));
        }
        path.push((into, to.mid_y()));
        path.push((to.x - 1, to.mid_y()));
        path
    }

    fn arrow(path: &[(u16, u16)]) -> char {
        let [.., (ax, ay), (bx, by)] = path else {
            return '▶';
        };
        match (bx.cmp(ax), by.cmp(ay)) {
            (std::cmp::Ordering::Less, _) => '◀',
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
            let path = Self::route(from, to);
            canvas.path(&path, edge.kind.style());
            if let Some(&(x, y)) = path.last() {
                canvas.put(x, y, Self::arrow(&path), edge.kind.style());
            }
            // Beside where the edge leaves its source, inside the gap.
            if let (Some(label), [(x, y), (next, _), ..]) = (&edge.label, path.as_slice()) {
                let x = if next < x { x.saturating_sub(3) } else { x + 1 };
                canvas.text(x, y.saturating_sub(1), label, edge.kind.style());
            }
        }
        for node in &self.nodes {
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
    canvas.text(
        x + 2,
        y,
        &format!(" {} ", truncate(&agent_name(&node.key), inner - 2)),
        border,
    );
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

/// A character grid the size of the whole graph; the frame shows a window
/// of it.
pub struct Canvas {
    width: u16,
    height: u16,
    cells: Vec<Cell>,
}

impl Canvas {
    pub fn new(width: u16, height: u16) -> Self {
        Canvas {
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
        (x < self.width && y < self.height)
            .then(|| &mut self.cells[usize::from(y) * usize::from(self.width) + usize::from(x)])
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
                if x >= self.width || y >= self.height {
                    continue;
                }
                let cell = &self.cells[usize::from(y) * usize::from(self.width) + usize::from(x)];
                if let Some(target) = buffer.cell_mut((area.x + column, area.y + row)) {
                    target.set_char(Self::glyph(cell)).set_style(cell.style);
                }
            }
        }
    }

    #[cfg(test)]
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
///
/// Default position is bottom-right. If the selected node intersects that
/// area, the overlay dynamically flips to top-right (or alternative corners)
/// so that the selected node remains visible.
pub fn compute_progress_overlay(
    inner: Rect,
    selected_node: Option<&Node>,
    offset: (u16, u16),
    progress_width: u16,
    progress_height: u16,
) -> Option<Rect> {
    if inner.height < progress_height + 2 || inner.width < progress_width {
        return None;
    }
    let default_x = inner.right().saturating_sub(progress_width);
    let bottom_y = inner.bottom().saturating_sub(progress_height);
    let mut overlay = Rect::new(default_x, bottom_y, progress_width, progress_height);

    if let Some(node) = selected_node {
        let screen_x = inner.x as i32 + node.x as i32 - offset.0 as i32;
        let screen_y = inner.y as i32 + node.y as i32 - offset.1 as i32;

        let node_rect = Rect::new(
            screen_x.max(0) as u16,
            screen_y.max(0) as u16,
            WIDTH,
            HEIGHT,
        );

        if node_rect.intersects(overlay) {
            // Collision on bottom-right! Flip to top-right corner
            overlay.y = inner.y;
            if node_rect.intersects(overlay) {
                // Collision on top-right as well! Flip to top-left
                overlay.x = inner.x;
                if node_rect.intersects(overlay) {
                    // Collision on top-left! Flip to bottom-left
                    overlay.y = bottom_y;
                    if node_rect.intersects(overlay) {
                        return None;
                    }
                }
            }
        }
    }
    Some(overlay)
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
        };
        assert!(
            compute_progress_overlay(small_inner, Some(&center_node), (0, 0), 25, 5,).is_none(),
            "when all four corners intersect selected node, overlay is hidden"
        );
    }
}
