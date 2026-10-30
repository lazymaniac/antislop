use super::*;

const UP: u8 = 1;
const DOWN: u8 = 2;
const LEFT: u8 = 4;
const RIGHT: u8 = 8;

#[derive(Debug, Clone)]
pub(super) struct Cell {
    lines: u8,
    ch: Option<String>,
    pub(super) style: Style,
}

/// A character grid for rendering graph nodes and orthogonal edges.
/// Memory is strictly bounded to the viewport window when windowing is used.
pub struct Canvas {
    pub(super) origin_x: u16,
    pub(super) origin_y: u16,
    pub(super) width: u16,
    pub(super) height: u16,
    pub(super) cells: Vec<Cell>,
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

    pub(super) fn cell(&mut self, x: u16, y: u16) -> Option<&mut Cell> {
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

    pub(super) fn put(&mut self, x: u16, y: u16, ch: char, style: Style) {
        if let Some(cell) = self.cell(x, y) {
            cell.ch = Some(ch.to_string());
            cell.style = style;
        }
    }

    pub(super) fn text(&mut self, x: u16, y: u16, text: &str, style: Style) {
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
    pub(super) fn path(&mut self, points: &[(u16, u16)], style: Style) {
        if self.width == 0 || self.height == 0 {
            return;
        }
        for pair in points.windows(2) {
            self.segment(pair[0], pair[1], style);
        }
    }

    fn segment(&mut self, from: (u16, u16), to: (u16, u16), style: Style) {
        self.horizontal(from, to, style);
        self.vertical(from, to, style);
    }

    fn horizontal(&mut self, from: (u16, u16), to: (u16, u16), style: Style) {
        let (x, y) = from;
        let (tx, _) = to;
        if x == tx || !self.row_visible(y) {
            return;
        }
        let (lo, hi) = (x.min(tx), x.max(tx));
        let start = lo.max(self.origin_x);
        let end = hi.min(self.origin_x.saturating_add(self.width).saturating_sub(1));
        for column in start..=end {
            if let Some(cell) = self.cell(column, y) {
                cell.lines |=
                    if column > lo { LEFT } else { 0 } | if column < hi { RIGHT } else { 0 };
                cell.style = style;
            }
        }
    }

    fn vertical(&mut self, from: (u16, u16), to: (u16, u16), style: Style) {
        let (_, y) = from;
        let (x, ty) = to;
        if y == ty || !self.column_visible(x) {
            return;
        }
        let (lo, hi) = (y.min(ty), y.max(ty));
        let start = lo.max(self.origin_y);
        let end = hi.min(self.origin_y.saturating_add(self.height).saturating_sub(1));
        for row in start..=end {
            if let Some(cell) = self.cell(x, row) {
                cell.lines |= if row > lo { UP } else { 0 } | if row < hi { DOWN } else { 0 };
                cell.style = style;
            }
        }
    }

    fn row_visible(&self, row: u16) -> bool {
        row >= self.origin_y && row < self.origin_y.saturating_add(self.height)
    }

    fn column_visible(&self, column: u16) -> bool {
        column >= self.origin_x && column < self.origin_x.saturating_add(self.width)
    }

    pub(super) fn glyph(cell: &Cell) -> &str {
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
