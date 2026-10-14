//! Stable two-dimensional agent graph. Layout is independent of status updates
//! and transcript activity; the viewport follows keyboard selection.
use super::{app::App, model::Agent, theme::Theme};
use ratatui::{buffer::Buffer, layout::Rect, style::Style, widgets::Widget};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub const NODE_WIDTH: i32 = 28;
pub const NODE_HEIGHT: i32 = 4;
pub const COLUMN: i32 = 40;
pub const ROW: i32 = 6;

#[derive(Debug, Default)]
pub struct Layout {
    pub positions: BTreeMap<String, (i32, i32)>,
    grouped: bool,
}
impl Layout {
    pub fn update(&mut self, agents: &[Agent], grouped: bool) {
        if self.grouped != grouped {
            self.positions.clear();
            self.grouped = grouped;
        }
        let ids: BTreeSet<_> = agents.iter().map(|a| a.id.as_str()).collect();
        self.positions.retain(|id, _| ids.contains(id.as_str()));
        let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        let mut roots = BTreeSet::new();
        for agent in agents {
            match agent.parent.as_deref().filter(|p| ids.contains(p)) {
                Some(parent) => children.entry(parent).or_default().push(&agent.id),
                None => {
                    roots.insert(agent.id.as_str());
                }
            }
        }
        for list in children.values_mut() {
            list.sort_unstable();
        }
        let mut ranks = BTreeMap::new();
        let mut queue: VecDeque<_> = roots.into_iter().map(|id| (id, 0)).collect();
        // A disconnected cycle remains observable. Iteration never recurses.
        for root in &ids {
            if queue.is_empty() && !ranks.contains_key(root) {
                queue.push_back((*root, 0));
            }
            while let Some((id, rank)) = queue.pop_front() {
                if ranks.contains_key(id) {
                    continue;
                }
                ranks.insert(id, rank);
                if let Some(list) = children.get(id) {
                    queue.extend(
                        list.iter()
                            .filter(|id| !ranks.contains_key(**id))
                            .map(|id| (*id, rank + 1)),
                    );
                }
            }
        }
        let mut next_row: BTreeMap<i32, i32> = BTreeMap::new();
        for (column, row) in self.positions.values() {
            next_row
                .entry(*column)
                .and_modify(|n| *n = (*n).max(row + ROW))
                .or_insert(row + ROW);
        }
        let parents: BTreeMap<_, _> = agents
            .iter()
            .map(|a| (a.id.as_str(), a.parent.as_deref()))
            .collect();
        if grouped {
            for agent in agents {
                let rank = match agent.group.as_str() {
                    "discovery" => 0,
                    "planning" => 1,
                    "execution" => 2,
                    "validation" => 3,
                    "integration" => 4,
                    "delivery" => 5,
                    _ => *ranks.get(agent.id.as_str()).unwrap_or(&0),
                };
                ranks.insert(agent.id.as_str(), rank);
            }
        }
        let mut ranked: Vec<_> = ranks.into_iter().collect();
        ranked.sort_by_key(|(id, rank)| (*rank, *id));
        for (id, rank) in ranked {
            if self.positions.contains_key(id) {
                continue;
            }
            let column = rank * COLUMN;
            let parent_row = parents
                .get(id)
                .and_then(|p| *p)
                .and_then(|p| self.positions.get(p))
                .map_or(0, |p| p.1);
            let row = next_row.entry(column).or_default();
            *row = (*row).max(parent_row);
            self.positions.insert(id.into(), (column, *row));
            *row += ROW;
        }
    }
    pub fn neighbor<'a>(
        &self,
        selected: &str,
        visible: &'a [String],
        dx: i32,
        dy: i32,
    ) -> Option<&'a str> {
        let &(x, y) = self.positions.get(selected)?;
        visible
            .iter()
            .filter(|id| id.as_str() != selected)
            .filter_map(|id| {
                let &(nx, ny) = self.positions.get(id)?;
                let forward = (nx - x) * dx + (ny - y) * dy;
                if forward <= 0 {
                    return None;
                }
                let sideways = ((nx - x) * dy - (ny - y) * dx).abs();
                Some((forward as i64 + sideways as i64 * 3, id.as_str()))
            })
            .min()
            .map(|(_, id)| id)
    }
}

const NORTH: u8 = 1;
const SOUTH: u8 = 2;
const EAST: u8 = 4;
const WEST: u8 = 8;

/// Edge cells inside the viewport. Overlapping edges merge into junctions.
#[derive(Default)]
struct Canvas {
    cells: BTreeMap<(i32, i32), (u8, Style, bool)>,
    labels: Vec<(i32, i32, &'static str, Style)>,
}
impl Canvas {
    fn mark(&mut self, x: i32, y: i32, mask: u8, style: Style, dashed: bool) {
        let cell = self.cells.entry((x, y)).or_insert((0, style, dashed));
        cell.0 |= mask;
        cell.1 = style;
        cell.2 &= dashed;
    }
    /// Horizontal and vertical runs are clipped before iteration, so large
    /// offscreen graphs stay cheap.
    fn horizontal(&mut self, y: i32, a: i32, b: i32, clip: Rect32, style: Style, dashed: bool) {
        if y < clip.top || y >= clip.bottom {
            return;
        }
        let (min, max) = (a.min(b), a.max(b));
        for x in min.max(clip.left - 1)..=max.min(clip.right) {
            let mask = if x < max { EAST } else { 0 } | if x > min { WEST } else { 0 };
            self.mark(x, y, mask, style, dashed);
        }
    }
    fn vertical(&mut self, x: i32, a: i32, b: i32, clip: Rect32, style: Style, dashed: bool) {
        if x < clip.left || x >= clip.right {
            return;
        }
        let (min, max) = (a.min(b), a.max(b));
        for y in min.max(clip.top - 1)..=max.min(clip.bottom) {
            let mask = if y < max { SOUTH } else { 0 } | if y > min { NORTH } else { 0 };
            self.mark(x, y, mask, style, dashed);
        }
    }
}
#[derive(Clone, Copy)]
struct Rect32 {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

pub struct Graph<'a> {
    pub app: &'a App,
    pub theme: &'a Theme,
    pub visible: &'a BTreeSet<String>,
    /// A screen region covered by an overlay; the selection is kept out of it.
    pub avoid: Rect,
}
impl Graph<'_> {
    fn edges<'b>(&self, agent: &'b Agent) -> BTreeSet<(&'b str, &'b str)> {
        let mut edges: BTreeSet<_> = agent
            .parent
            .as_ref()
            .map(|id| ("supervises", id.as_str()))
            .into_iter()
            .collect();
        edges.extend(
            agent
                .relations
                .iter()
                .filter(|(kind, _)| {
                    kind != "supervised by" && (self.app.relations || kind != "communicates with")
                })
                .map(|(kind, id)| (kind.as_str(), id.as_str())),
        );
        edges
    }
}
impl Widget for Graph<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }
        let app = self.app;
        let theme = self.theme;
        let glyphs = theme.glyphs();
        let positions = &app.graph.positions;
        let origin = app
            .selected_agent_id
            .as_ref()
            .and_then(|id| positions.get(id))
            .copied()
            .unwrap_or((0, 0));
        // The viewport scrolls only when the graph does not fit, and then
        // centers the selection without scrolling past the graph's extent.
        let extent = app
            .snapshot
            .agents
            .iter()
            .filter(|a| self.visible.contains(&a.id))
            .filter_map(|a| positions.get(&a.id))
            .fold((0, 0), |(w, h), &(x, y)| {
                (w.max(x + NODE_WIDTH + 2), h.max(y + NODE_HEIGHT))
            });
        let scroll = |origin: i32, node: i32, view: i32, extent: i32| {
            (origin - (view - node).max(0) / 2).clamp(0, (extent - view).max(0))
        };
        let mut offset_x = scroll(origin.0, NODE_WIDTH, area.width as i32, extent.0);
        let mut offset_y = scroll(origin.1, NODE_HEIGHT, area.height as i32, extent.1);
        let avoid = self.avoid.intersection(area);
        if !avoid.is_empty() && app.selected_agent_id.is_some() {
            let (left, top) = (origin.0 - offset_x, origin.1 - offset_y);
            let (avoid_x, avoid_y) = ((avoid.x - area.x) as i32, (avoid.y - area.y) as i32);
            let covered = left + NODE_WIDTH > avoid_x
                && left < avoid_x + avoid.width as i32
                && top + NODE_HEIGHT > avoid_y
                && top < avoid_y + avoid.height as i32;
            // Move the view so the selection sits above the overlay, or to its
            // left when the overlay is too tall.
            if covered && avoid_y >= NODE_HEIGHT {
                offset_y = origin.1 + NODE_HEIGHT - avoid_y;
            } else if covered && avoid_x >= NODE_WIDTH {
                offset_x = origin.0 + NODE_WIDTH - avoid_x;
            }
        }
        let clip = Rect32 {
            left: offset_x,
            top: offset_y,
            right: offset_x + area.width as i32,
            bottom: offset_y + area.height as i32,
        };
        let put = |buf: &mut Buffer, x: i32, y: i32, text: &str, style: Style| {
            let (x, y) = (x - offset_x, y - offset_y);
            if x >= 0 && y >= 0 && x < area.width as i32 && y < area.height as i32 {
                buf[(area.x + x as u16, area.y + y as u16)]
                    .set_symbol(text)
                    .set_style(style);
            }
        };
        // Supervision edges first, so typed relations draw over the tree.
        let mut canvas = Canvas::default();
        for pass in [true, false] {
            for agent in &app.snapshot.agents {
                if !self.visible.contains(&agent.id) {
                    continue;
                }
                let Some(&(tx, ty)) = positions.get(&agent.id) else {
                    continue;
                };
                for (kind, from) in self.edges(agent) {
                    if (kind == "supervises") != pass
                        || !self.visible.contains(from)
                        || from == agent.id
                    {
                        continue;
                    }
                    let Some(&(sx, sy)) = positions.get(from) else {
                        continue;
                    };
                    // Same-column routes leave one row lower than they enter,
                    // so an outgoing port never shares a cell with an arrow.
                    let (start_y, end_y) = (sy + 1 + i32::from(sx == tx), ty + 1);
                    let (start_x, end_x, middle) = if sx == tx {
                        // Same column: a side lane per relation kind keeps the
                        // route outside the cards.
                        let lane = match kind {
                            "verifies" | "verified by" => 2,
                            "depends_on" | "depends on" => 3,
                            _ => 4,
                        };
                        (sx + NODE_WIDTH, tx + NODE_WIDTH, sx + NODE_WIDTH + lane)
                    } else if sx < tx {
                        (sx + NODE_WIDTH, tx - 1, (sx + NODE_WIDTH + tx - 1) / 2)
                    } else {
                        (sx - 1, tx + NODE_WIDTH, (sx - 1 + tx + NODE_WIDTH) / 2)
                    };
                    let style = theme.edge(kind);
                    let dashed = kind.starts_with("communicates");
                    canvas.horizontal(start_y, start_x, middle, clip, style, dashed);
                    canvas.vertical(middle, start_y, end_y, clip, style, dashed);
                    canvas.horizontal(end_y, middle, end_x, clip, style, dashed);
                    canvas.labels.push((
                        end_x,
                        end_y,
                        if sx < tx {
                            glyphs.arrow_right
                        } else {
                            glyphs.arrow_left
                        },
                        style,
                    ));
                    // Color is not the only carrier of the relation kind.
                    let label = match kind {
                        "verifies" | "verified by" => Some("V"),
                        "depends_on" | "depends on" => Some("D"),
                        "communicates_with" | "communicates with" => Some("M"),
                        _ => None,
                    };
                    if let Some(label) = label {
                        let y = if start_y == end_y {
                            start_y
                        } else {
                            (start_y + end_y) / 2
                        };
                        canvas.labels.push((middle, y, label, style));
                    }
                }
            }
        }
        for (&(x, y), &(mask, style, dashed)) in &canvas.cells {
            let symbol = match mask {
                1..=3 if dashed => glyphs.dashed_vertical,
                12 | 4 | 8 if dashed => glyphs.dashed_horizontal,
                _ => glyphs.junctions[mask as usize],
            };
            put(buf, x, y, symbol, style);
        }
        for (x, y, label, style) in canvas.labels {
            put(buf, x, y, label, style);
        }
        for agent in &app.snapshot.agents {
            if !self.visible.contains(&agent.id) {
                continue;
            }
            let Some(&(x, y)) = positions.get(&agent.id) else {
                continue;
            };
            if x + NODE_WIDTH < clip.left
                || x > clip.right
                || y + NODE_HEIGHT < clip.top
                || y > clip.bottom
            {
                continue;
            }
            let selected = app.selected_agent_id.as_deref() == Some(agent.id.as_str());
            let fill = if selected {
                theme.selected()
            } else {
                theme.card()
            };
            for dy in 0..NODE_HEIGHT {
                for dx in 0..NODE_WIDTH {
                    put(buf, x + dx, y + dy, " ", fill);
                }
                put(
                    buf,
                    x,
                    y + dy,
                    glyphs.bar,
                    fill.patch(theme.status(&agent.status)),
                );
            }
            let room = (NODE_WIDTH - 3) as usize;
            let status = theme.status_glyph(&agent.status);
            let task = super::theme::wrap(&theme.clean(&agent.task), room);
            let meta = [agent.provider.as_str(), agent.model.as_str()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let mut rows = vec![
                vec![(
                    theme.fit(&theme.clean(&agent.role), room - 2),
                    fill.patch(theme.title()),
                )],
                vec![
                    (
                        theme.clean(&agent.status),
                        if selected {
                            fill
                        } else {
                            fill.patch(theme.status(&agent.status))
                        },
                    ),
                    (
                        if meta.is_empty() {
                            String::new()
                        } else {
                            theme.fit(
                                &format!(" {} {}", glyphs.dot, theme.clean(&meta)),
                                room.saturating_sub(agent.status.width()),
                            )
                        },
                        if selected {
                            fill
                        } else {
                            fill.patch(theme.dim())
                        },
                    ),
                ],
            ];
            for (i, line) in task.iter().take(2).enumerate() {
                let line = if i == 1 && task.len() > 2 {
                    theme.fit(&format!("{line}{}", glyphs.ellipsis), room)
                } else {
                    line.clone()
                };
                rows.push(vec![(line, fill)]);
            }
            for (dy, spans) in rows.into_iter().enumerate() {
                let mut left = x + 2;
                let limit = x + 2 + room as i32;
                for (text, style) in spans {
                    for g in text.graphemes(true) {
                        let w = g.width() as i32;
                        if left + w > limit {
                            break;
                        }
                        // The terminal diff skips cells covered by wide graphemes.
                        put(buf, left, y + dy as i32, g, style);
                        left += w;
                    }
                }
            }
            put(
                buf,
                x + NODE_WIDTH - 2,
                y,
                status,
                fill.patch(theme.status(&agent.status)),
            );
        }
    }
}
