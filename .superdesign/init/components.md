# Current terminal primitives
Rust Ratatui 0.30.2, crossterm 0.29.0. No HTML/CSS/React frontend exists.
## `src/tui/editor.rs`
```rust
//! Grapheme-aware editing. Byte indices never escape this module.
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, Default)]
pub struct Editor {
    pub text: String,
    cursor: usize,
    preferred_column: Option<usize>,
}
impl Editor {
    pub fn cursor(&self) -> usize {
        self.cursor
    }
    pub fn set(&mut self, text: String) {
        self.cursor = text.len();
        self.text = text;
        self.preferred_column = None;
    }
    pub fn insert(&mut self, text: &str) {
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
        // A combining character can merge with the preceding grapheme.
        self.cursor = self
            .text
            .grapheme_indices(true)
            .map(|(i, _)| i)
            .find(|i| *i >= self.cursor)
            .unwrap_or(self.text.len());
        self.preferred_column = None;
    }
    pub fn position(&self) -> (usize, usize) {
        let before = &self.text[..self.cursor];
        (
            before.matches('\n').count(),
            before.rsplit('\n').next().unwrap_or("").width(),
        )
    }
    fn previous(&self) -> usize {
        self.text[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i)
    }
    fn next(&self) -> usize {
        self.text[self.cursor..]
            .graphemes(true)
            .next()
            .map_or(self.cursor, |g| self.cursor + g.len())
    }
    fn vertical(&mut self, down: bool) {
        let (row, col) = self.position();
        let desired = self.preferred_column.unwrap_or(col);
        let target = if down { row + 1 } else { row.saturating_sub(1) };
        let mut start = 0;
        if let Some(line) = self.text.split('\n').nth(target) {
            for line in self.text.split('\n').take(target) {
                start += line.len() + 1;
            }
            let mut width = 0;
            self.cursor = start;
            for g in line.graphemes(true) {
                if width + g.width() > desired {
                    break;
                }
                width += g.width();
                self.cursor += g.len();
            }
        }
        self.preferred_column = Some(desired);
    }
    pub fn key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char(c)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.insert(&c.to_string())
            }
            KeyCode::Enter => self.insert("\n"),
            KeyCode::Left => {
                self.cursor = self.previous();
                self.preferred_column = None;
            }
            KeyCode::Right => {
                self.cursor = self.next();
                self.preferred_column = None;
            }
            KeyCode::Up => self.vertical(false),
            KeyCode::Down => self.vertical(true),
            KeyCode::Home => {
                self.cursor -= self.text[..self.cursor]
                    .rsplit('\n')
                    .next()
                    .unwrap_or("")
                    .len();
                self.preferred_column = None;
            }
            KeyCode::End => {
                self.cursor += self.text[self.cursor..]
                    .split('\n')
                    .next()
                    .unwrap_or("")
                    .len();
                self.preferred_column = None;
            }
            KeyCode::Backspace if self.cursor > 0 => {
                let previous = self.previous();
                self.text.replace_range(previous..self.cursor, "");
                self.cursor = previous;
                self.preferred_column = None;
            }
            KeyCode::Delete => {
                let next = self.next();
                self.text.replace_range(self.cursor..next, "");
                self.preferred_column = None;
            }
            _ => (),
        }
    }
    /// Unwrapped viewport keeps wide and combining graphemes aligned with the
    /// cursor, including when the draft is wider/taller than its panel.
    pub fn viewport(&self, width: usize, height: usize) -> (String, u16, u16) {
        if width == 0 || height == 0 {
            return (String::new(), 0, 0);
        }
        let (row, col) = self.position();
        let top = row.saturating_sub(height - 1);
        let left = col.saturating_sub(width - 1);
        let mut actual_left = left;
        let lines: Vec<_> = self
            .text
            .split('\n')
            .skip(top)
            .take(height)
            .enumerate()
            .map(|(index, line)| {
                let mut at = 0;
                let mut out = String::new();
                for grapheme in line.graphemes(true) {
                    let next = at + grapheme.width();
                    if at < left && next > left {
                        if index + top == row {
                            actual_left = next;
                        }
                    } else if at >= left && next <= left + width {
                        out.push_str(grapheme);
                    }
                    at = next;
                }
                out
            })
            .collect();
        (
            lines.join("\n"),
            col.saturating_sub(actual_left).min(width - 1) as u16,
            (row - top) as u16,
        )
    }
}

```
## `src/tui/graph.rs`
```rust
//! Stable two-dimensional agent graph. Layout is independent of status updates
//! and transcript activity; the viewport follows keyboard selection.
use super::{app::App, model::Agent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    widgets::Widget,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

pub const NODE_WIDTH: i32 = 25;
pub const COLUMN: i32 = 34;
pub const ROW: i32 = 7;

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

pub struct Graph<'a> {
    pub app: &'a App,
    pub visible: &'a BTreeSet<String>,
}
impl Widget for Graph<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.is_empty() {
            return;
        }
        let app = self.app;
        let positions = &app.graph.positions;
        let origin = app
            .selected_agent_id
            .as_ref()
            .and_then(|id| positions.get(id))
            .copied()
            .unwrap_or((0, 0));
        let offset_x = (origin.0 - (area.width as i32 - NODE_WIDTH).max(0) / 2).max(0);
        let offset_y = (origin.1 - (area.height as i32 - 4).max(0) / 2).max(0);
        let cell = |buf: &mut Buffer, x: i32, y: i32, text: &str, style: Style| {
            let (x, y) = (x - offset_x, y - offset_y);
            if x >= 0 && y >= 0 && x < area.width as i32 && y < area.height as i32 {
                buf[(area.x + x as u16, area.y + y as u16)]
                    .set_symbol(text)
                    .set_style(style);
            }
        };
        let h = if app.ascii { "-" } else { "─" };
        let v = if app.ascii { "|" } else { "│" };
        let normal = Style::default();
        for agent in &app.snapshot.agents {
            if !self.visible.contains(&agent.id) {
                continue;
            }
            let Some(&(tx, ty)) = positions.get(&agent.id) else {
                continue;
            };
            let mut edges: BTreeSet<_> = agent
                .parent
                .as_ref()
                .map(|id| ("supervises", id.as_str()))
                .into_iter()
                .collect();
            {
                edges.extend(
                    agent
                        .relations
                        .iter()
                        .filter(|(kind, _)| {
                            kind != "supervised by"
                                && (app.relations || kind != "communicates with")
                        })
                        .map(|(kind, id)| (kind.as_str(), id.as_str())),
                );
            }
            for (kind, from) in edges {
                if !self.visible.contains(from) || from == agent.id {
                    continue;
                }
                let Some(&(sx, sy)) = positions.get(from) else {
                    continue;
                };
                let (start_x, end_x) = if sx < tx {
                    (sx + NODE_WIDTH, tx - 1)
                } else {
                    (sx - 1, tx + NODE_WIDTH)
                };
                let (start_y, end_y) = (sy + 1, ty + 1);
                let middle = (start_x + end_x) / 2;
                // Clip segments before iterating: large offscreen graphs stay cheap.
                for x in start_x.min(middle).max(offset_x)
                    ..=start_x.max(middle).min(offset_x + area.width as i32)
                {
                    cell(buf, x, start_y, h, normal);
                }
                for y in start_y.min(end_y).max(offset_y)
                    ..=start_y.max(end_y).min(offset_y + area.height as i32)
                {
                    cell(buf, middle, y, v, normal);
                }
                for x in middle.min(end_x).max(offset_x)
                    ..=middle.max(end_x).min(offset_x + area.width as i32)
                {
                    cell(buf, x, end_y, h, normal);
                }
                let label = match kind {
                    "verifies" | "verified by" => "V",
                    "depends_on" | "depends on" => "D",
                    "communicates_with" | "communicates with" => "M",
                    _ => "S",
                };
                cell(buf, middle, end_y, label, normal);
                cell(buf, end_x, end_y, if sx < tx { ">" } else { "<" }, normal);
            }
        }
        for agent in &app.snapshot.agents {
            if !self.visible.contains(&agent.id) {
                continue;
            }
            let Some(&(x, y)) = positions.get(&agent.id) else {
                continue;
            };
            let selected = app.selected_agent_id.as_deref() == Some(agent.id.as_str());
            let style = if selected {
                normal.add_modifier(Modifier::BOLD | Modifier::REVERSED)
            } else {
                normal
            };
            for dy in 0..5 {
                for dx in 0..NODE_WIDTH {
                    cell(buf, x + dx, y + dy, " ", style);
                }
            }
            for dx in 1..NODE_WIDTH - 1 {
                cell(buf, x + dx, y, h, style);
                cell(buf, x + dx, y + 4, h, style);
            }
            for dy in 1..4 {
                cell(buf, x, y + dy, v, style);
                cell(buf, x + NODE_WIDTH - 1, y + dy, v, style);
            }
            for (dx, dy, symbol) in [
                (0, 0, "┌"),
                (NODE_WIDTH - 1, 0, "┐"),
                (0, 4, "└"),
                (NODE_WIDTH - 1, 4, "┘"),
            ] {
                cell(
                    buf,
                    x + dx,
                    y + dy,
                    if app.ascii { "+" } else { symbol },
                    style,
                );
            }
            for (dy, label) in [
                (
                    1,
                    format!("{}{}", if selected { "> " } else { "" }, agent.role),
                ),
                (
                    2,
                    format!("{} | {} {}", agent.status, agent.provider, agent.model),
                ),
                (3, agent.task.clone()),
            ] {
                let safe = super::sanitize::sanitize(&label);
                // Terminal widths and combining characters are handled by Buffer.
                let left = x + 1 - offset_x;
                let top = y + dy - offset_y;
                if left >= 0 && top >= 0 && left < area.width as i32 && top < area.height as i32 {
                    buf.set_stringn(
                        area.x + left as u16,
                        area.y + top as u16,
                        safe,
                        (NODE_WIDTH - 2).min(area.width as i32 - left) as usize,
                        style,
                    );
                }
            }
        }
    }
}

```
