//! Grapheme-safe selection of the rendered viewport.

use super::sanitize::sanitize;

#[derive(Debug, PartialEq, Eq)]
struct VisibleGrapheme {
    column: u16,
    row: u16,
    width: u16,
    text: String,
}

impl VisibleGrapheme {
    fn document_position(&self, origin: usize, top: u16) -> (usize, u16) {
        (origin + usize::from(self.row - top), self.column)
    }

    fn contains(&self, column: u16, row: u16) -> bool {
        self.row == row && column >= self.column && column < self.column + self.width
    }
}

struct PinnedSelection {
    start: (usize, u16),
    end: (usize, u16),
    text: String,
}

/// A viewport-sized snapshot: never retains an agent's full output.
#[derive(Default)]
pub struct TextSelection {
    area: ratatui::layout::Rect,
    context: String,
    cells: Vec<VisibleGrapheme>,
    anchor: Option<usize>,
    end: Option<usize>,
    dragging: bool,
    last_click: Option<(usize, std::time::Instant)>,
    origin: usize,
    pinned: Option<PinnedSelection>,
}

impl TextSelection {
    pub fn invalidate_context(&mut self, context: &str) {
        if self.context != context {
            self.anchor = None;
            self.end = None;
            self.pinned = None;
            self.dragging = false;
            self.last_click = None;
        }
    }

    pub fn draw_scrolled(
        &mut self,
        buf: &mut ratatui::buffer::Buffer,
        area: Option<ratatui::layout::Rect>,
        context: String,
        origin: usize,
    ) {
        let area = area.unwrap_or_default().intersection(buf.area);
        let cells = Self::capture(buf, area);
        self.update_snapshot(area, context, origin, cells);
        self.highlight(buf);
    }

    fn capture(buf: &ratatui::buffer::Buffer, area: ratatui::layout::Rect) -> Vec<VisibleGrapheme> {
        use unicode_segmentation::UnicodeSegmentation;
        use unicode_width::UnicodeWidthStr;
        let mut cells = Vec::new();
        for y in area.y..area.bottom() {
            let mut x = area.x;
            while x < area.right() {
                let symbol = buf[(x, y)].symbol();
                // Ratatui stores an extended grapheme in its leading cell.
                let symbol = symbol.graphemes(true).next().unwrap_or(" ");
                let width = symbol.width().max(1).min(usize::from(area.right() - x)) as u16;
                cells.push(VisibleGrapheme {
                    column: x,
                    row: y,
                    width,
                    text: symbol.to_owned(),
                });
                x += width;
            }
        }
        cells
    }

    fn update_snapshot(
        &mut self,
        area: ratatui::layout::Rect,
        context: String,
        origin: usize,
        cells: Vec<VisibleGrapheme>,
    ) {
        let changed = self.area != area
            || self.context != context
            || (self.origin == origin && self.cells != cells);
        if !changed && self.origin != origin {
            self.pin_selection();
            self.clear_active_selection();
        }
        if changed {
            self.clear_active_selection();
            self.pinned = None;
        }
        self.origin = origin;
        self.area = area;
        self.context = context;
        self.cells = cells;
    }

    fn pin_selection(&mut self) {
        let Some((start, end)) = self.range() else {
            return;
        };
        let position = |index: usize| self.cells[index].document_position(self.origin, self.area.y);
        self.pinned = self.text().map(|text| PinnedSelection {
            start: position(start),
            end: position(end),
            text,
        });
    }

    fn clear_active_selection(&mut self) {
        self.anchor = None;
        self.end = None;
        self.dragging = false;
        self.last_click = None;
    }

    fn highlight(&self, buf: &mut ratatui::buffer::Buffer) {
        let range = self.range();
        for (index, grapheme) in self.cells.iter().enumerate() {
            let selected = if let Some(pinned) = &self.pinned {
                let position = grapheme.document_position(self.origin, self.area.y);
                position >= pinned.start && position <= pinned.end
            } else {
                range.is_some_and(|(start, end)| index >= start && index <= end)
            };
            if selected {
                for column in grapheme.column..grapheme.column + grapheme.width {
                    let cell = &mut buf[(column, grapheme.row)];
                    cell.set_style(
                        cell.style()
                            .add_modifier(ratatui::style::Modifier::REVERSED),
                    );
                }
            }
        }
    }

    fn range(&self) -> Option<(usize, usize)> {
        let (a, b) = (self.anchor?, self.end?);
        Some((a.min(b), a.max(b)))
    }

    pub fn mouse(&mut self, event: crossterm::event::MouseEvent) {
        use crossterm::event::{MouseButton, MouseEventKind};
        let hit = self.hit_test(event.column, event.row);
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => self.mouse_down(hit),
            MouseEventKind::Drag(MouseButton::Left) if self.dragging => {
                if hit.is_some() {
                    self.end = hit;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => self.dragging = false,
            _ => {}
        }
    }

    fn hit_test(&self, column: u16, row: u16) -> Option<usize> {
        self.cells
            .iter()
            .position(|cell| cell.contains(column, row))
    }

    fn mouse_down(&mut self, hit: Option<usize>) {
        self.pinned = None;
        self.anchor = hit;
        self.end = None;
        self.dragging = hit.is_some();
        let Some(index) = hit else {
            return;
        };
        let now = std::time::Instant::now();
        if is_double_click(self.last_click, index, now) {
            self.select_word(index);
            self.dragging = false;
        }
        self.last_click = Some((index, now));
    }

    fn select_word(&mut self, index: usize) {
        let row = self.cells[index].row;
        let first = self.row_start(row, index);
        let text = self.row_text(row);
        let offset: usize = self.cells[first..index].iter().map(|c| c.text.len()).sum();
        use unicode_segmentation::UnicodeSegmentation;
        for (start, word) in text.split_word_bound_indices() {
            if offset >= start && offset < start + word.len() {
                self.select_word_range(first, row, start, word.len());
                break;
            }
        }
    }

    fn row_start(&self, row: u16, fallback: usize) -> usize {
        self.cells
            .iter()
            .position(|cell| cell.row == row)
            .unwrap_or(fallback)
    }

    fn row_text(&self, row: u16) -> String {
        self.cells
            .iter()
            .filter(|cell| cell.row == row)
            .map(|cell| cell.text.as_str())
            .collect()
    }

    fn select_word_range(&mut self, first: usize, row: u16, start: usize, length: usize) {
        let (mut byte, end) = (0, start + length);
        for index in first..self.cells.len() {
            if self.cells[index].row != row {
                break;
            }
            if byte == start {
                self.anchor = Some(index);
            }
            if byte < end {
                self.end = Some(index);
            }
            byte += self.cells[index].text.len();
        }
    }

    pub fn text(&self) -> Option<String> {
        if let Some(pinned) = &self.pinned {
            return Some(pinned.text.clone());
        }
        let (start, end) = self.range()?;
        let mut text = String::new();
        let mut row = self.cells[start].row;
        for cell in &self.cells[start..=end] {
            if cell.row != row {
                text.truncate(text.trim_end_matches(' ').len());
                text.push('\n');
                row = cell.row;
            }
            text.push_str(&cell.text);
        }
        Some(sanitize(text.trim_end_matches(' ')))
    }
}

fn is_double_click(
    previous: Option<(usize, std::time::Instant)>,
    index: usize,
    now: std::time::Instant,
) -> bool {
    previous.is_some_and(|(last, time)| {
        last == index && now.duration_since(time) < std::time::Duration::from_millis(450)
    })
}

#[cfg(test)]
#[path = "selection/tests.rs"]
mod tests;
