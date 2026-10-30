//! Shared row, section, and card composition for TUI panels.

use super::{Theme, wrap};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::{Line, Span},
};
use unicode_width::UnicodeWidthStr;

/// One rendered row: `fill` covers the whole row, `line` is drawn on top.
#[derive(Debug, Clone, Default)]
pub struct Row {
    pub fill: Style,
    pub line: Line<'static>,
}

impl Row {
    pub fn gap() -> Self {
        Self::default()
    }

    pub fn new(fill: Style, spans: Vec<Span<'static>>) -> Self {
        Self {
            fill,
            line: Line::from(spans),
        }
    }
}

pub fn draw_rows<'a>(buf: &mut Buffer, area: Rect, rows: impl IntoIterator<Item = &'a Row>) {
    for (y, row) in (area.y..area.bottom()).zip(rows) {
        let line_area = Rect::new(area.x, y, area.width, 1);
        buf.set_style(line_area, row.fill);
        buf.set_line(area.x, y, &row.line, area.width);
    }
}

/// A row with `left` spans and `right` text aligned to the row end.
pub fn split_row(
    fill: Style,
    mut left: Vec<Span<'static>>,
    right: Span<'static>,
    width: usize,
) -> Row {
    let used: usize = left.iter().map(|s| s.content.width()).sum();
    let right_width = right.content.width();
    if right_width > 0 && used + right_width < width {
        left.push(Span::raw(" ".repeat(width - used - right_width)));
        left.push(right);
    }
    Row::new(fill, left)
}

/// A thin scroll indicator; nothing is drawn when everything fits.
pub fn scrollbar(
    buf: &mut Buffer,
    theme: &Theme,
    area: Rect,
    total: usize,
    top: usize,
    view: usize,
) {
    let height = area.height as usize;
    if area.width == 0 || height == 0 || total <= view {
        return;
    }
    let thumb = (view * height / total).clamp(1, height);
    let start = top.min(total - view) * (height - thumb) / (total - view);
    let glyphs = theme.glyphs();
    for y in 0..height {
        let (symbol, style) = if (start..start + thumb).contains(&y) {
            (glyphs.thumb, theme.dim())
        } else {
            (glyphs.track, theme.faint())
        };
        let py = area.y + y as u16;
        if py < area.bottom() && area.x < area.right() {
            buf[(area.x, py)].set_symbol(symbol).set_style(style);
        }
    }
}

/// Section headings replace panel borders.
pub fn section(theme: &Theme, title: &str, meta: String, focused: bool, width: u16) -> Row {
    let title_style = if focused {
        theme.accent()
    } else {
        theme.title()
    };
    let marker = if focused { theme.glyphs().pointer } else { " " };
    split_row(
        Style::default(),
        vec![
            Span::styled(marker, theme.accent()),
            Span::styled(format!(" {}", theme.clean(title)), title_style),
        ],
        Span::styled(
            theme.fit(&theme.clean(&meta), usize::from(width) / 2),
            theme.dim(),
        ),
        usize::from(width),
    )
}

/// Properties for constructing a visual card.
pub struct CardProps<'a> {
    pub fill: Style,
    pub accent: Style,
    pub head: Vec<Span<'static>>,
    pub right: String,
    pub body: &'a str,
    pub max_body: usize,
    pub width: u16,
}

/// A card: a colored accent bar, a heading row, wrapped body rows and a gap.
pub fn card(theme: &Theme, props: CardProps<'_>) -> Vec<Row> {
    let width = usize::from(props.width);
    if width == 0 {
        return vec![];
    }
    let mut rows = vec![card_heading(theme, &props, width)];
    rows.extend(card_body(theme, &props, width));
    rows.push(Row::gap());
    rows
}

fn card_heading(theme: &Theme, props: &CardProps<'_>, width: usize) -> Row {
    let bar = Span::styled(theme.glyphs().bar, props.fill.patch(props.accent));
    let mut left = vec![bar, Span::styled(" ", props.fill)];
    left.extend(props.head.iter().map(|span| {
        Span::styled(
            span.content.clone().into_owned(),
            props.fill.patch(span.style),
        )
    }));
    split_row(
        props.fill,
        left,
        Span::styled(
            theme.fit(&props.right, width / 2),
            props.fill.patch(theme.dim()),
        ),
        width,
    )
}

fn card_body(theme: &Theme, props: &CardProps<'_>, width: usize) -> Vec<Row> {
    let room = width.saturating_sub(3);
    let lines = if props.body.is_empty() {
        vec![]
    } else {
        wrap(&theme.clean(props.body), room)
    };
    let cut = lines.len() > props.max_body;
    lines
        .iter()
        .take(props.max_body)
        .enumerate()
        .map(|(index, line)| body_row(theme, props, line, index, cut, room))
        .collect()
}

fn body_row(
    theme: &Theme,
    props: &CardProps<'_>,
    line: &str,
    index: usize,
    cut: bool,
    room: usize,
) -> Row {
    let line = if cut && index + 1 == props.max_body {
        theme.fit(&format!("{line}{}", theme.glyphs().ellipsis), room)
    } else {
        line.to_owned()
    };
    Row::new(
        props.fill,
        vec![
            Span::styled(theme.glyphs().bar, props.fill.patch(props.accent)),
            Span::styled(" ", props.fill),
            Span::styled(line, props.fill.patch(theme.text())),
        ],
    )
}

pub fn title_row(theme: &Theme, title: &str, width: u16) -> Row {
    let fill = theme.overlay();
    let (name, hint) = title.split_once(" | ").unwrap_or((title, ""));
    split_row(
        fill,
        vec![
            Span::styled(" ", fill),
            Span::styled(
                theme.fit(&theme.clean(name), usize::from(width) / 2),
                fill.patch(theme.accent()),
            ),
        ],
        Span::styled(
            format!("{} ", theme.fit(&theme.clean(hint), usize::from(width) / 2)),
            fill.patch(theme.dim()),
        ),
        usize::from(width),
    )
}
