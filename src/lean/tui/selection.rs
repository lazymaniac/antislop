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
            if let Some((start, end)) = self.range() {
                let position = |index: usize| {
                    let cell = &self.cells[index];
                    cell.document_position(self.origin, self.area.y)
                };
                self.pinned = self.text().map(|text| PinnedSelection {
                    start: position(start),
                    end: position(end),
                    text,
                });
            }
            self.anchor = None;
            self.end = None;
            self.dragging = false;
            self.last_click = None;
        }
        if changed {
            self.anchor = None;
            self.end = None;
            self.dragging = false;
            self.last_click = None;
            self.pinned = None;
        }
        self.origin = origin;
        self.area = area;
        self.context = context;
        self.cells = cells;
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
        let hit = self
            .cells
            .iter()
            .position(|cell| cell.contains(event.column, event.row));
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.pinned = None;
                self.anchor = hit;
                self.end = None;
                self.dragging = hit.is_some();
                if let Some(index) = hit {
                    let now = std::time::Instant::now();
                    if self.last_click.is_some_and(|(previous, time)| {
                        previous == index
                            && now.duration_since(time) < std::time::Duration::from_millis(450)
                    }) {
                        self.select_word(index);
                        self.dragging = false;
                    }
                    self.last_click = Some((index, now));
                }
            }
            MouseEventKind::Drag(MouseButton::Left) if self.dragging => {
                if hit.is_some() {
                    self.end = hit;
                }
            }
            MouseEventKind::Up(MouseButton::Left) => self.dragging = false,
            _ => {}
        }
    }

    fn select_word(&mut self, index: usize) {
        let row = self.cells[index].row;
        let text: String = self
            .cells
            .iter()
            .filter(|c| c.row == row)
            .map(|c| c.text.as_str())
            .collect();
        let first = self
            .cells
            .iter()
            .position(|c| c.row == row)
            .unwrap_or(index);
        let offset: usize = self.cells[first..index].iter().map(|c| c.text.len()).sum();
        use unicode_segmentation::UnicodeSegmentation;
        for (start, word) in text.split_word_bound_indices() {
            if offset >= start && offset < start + word.len() {
                let mut byte = 0;
                for i in first..self.cells.len() {
                    if self.cells[i].row != row {
                        break;
                    }
                    if byte == start {
                        self.anchor = Some(i);
                    }
                    if byte < start + word.len() {
                        self.end = Some(i);
                    }
                    byte += self.cells[i].text.len();
                }
                break;
            }
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

#[cfg(test)]
mod selection_tests {
    use super::*;
    use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    #[test]
    fn double_click_word_and_backwards_drag() {
        let area = ratatui::layout::Rect::new(0, 0, 12, 2);
        let mut buf = ratatui::buffer::Buffer::empty(area);
        buf.set_string(0, 0, "one café", ratatui::style::Style::default());
        buf.set_string(0, 1, "two", ratatui::style::Style::default());
        let mut selection = TextSelection::default();
        selection.draw_scrolled(&mut buf, Some(area), "chat".into(), 0);
        let mouse = |kind, column, row| MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        selection.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 6, 0));
        selection.mouse(mouse(MouseEventKind::Up(MouseButton::Left), 6, 0));
        selection.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 6, 0));
        assert_eq!(selection.text().as_deref(), Some("café"));
        selection.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 2, 1));
        selection.mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 4, 0));
        assert_eq!(selection.text().as_deref(), Some("café\ntwo"));
        buf.set_string(0, 0, "changed", ratatui::style::Style::default());
        selection.draw_scrolled(&mut buf, Some(area), "chat".into(), 0);
        assert!(selection.text().is_none());
    }
    #[test]
    fn wrapped_rows_keep_whole_clusters_and_viewport_storage() {
        let area = ratatui::layout::Rect::new(0, 0, 4, 3);
        let mut buffer = ratatui::buffer::Buffer::empty(area);
        let rows = super::super::view::wrap("ab👩‍👩‍👧‍👦café", 4);
        for (y, row) in rows.iter().enumerate() {
            buffer.set_string(0, y as u16, row, ratatui::style::Style::default());
        }
        let mut selection = TextSelection::default();
        selection.draw_scrolled(&mut buffer, Some(area), "wrapped".into(), 0);
        let mouse = |kind, column, row| MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        };
        // Start on the second display column of the family emoji.
        selection.mouse(mouse(MouseEventKind::Down(MouseButton::Left), 3, 0));
        selection.mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 3, 1));
        assert_eq!(selection.text().as_deref(), Some("👩‍👩‍👧‍👦\ncafé"));
        assert!(selection.cells.len() <= usize::from(area.width) * usize::from(area.height));
    }
}
