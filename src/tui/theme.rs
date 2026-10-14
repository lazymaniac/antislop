//! Visual tokens shared by every panel. Cards are separated by background,
//! not borders; NO_COLOR keeps only modifiers and ASCII mode keeps only ASCII.
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub struct Glyphs {
    pub logo: &'static str,
    pub bar: &'static str,
    pub vertical: &'static str,
    pub horizontal: &'static str,
    pub dashed_vertical: &'static str,
    pub dashed_horizontal: &'static str,
    pub arrow_right: &'static str,
    pub arrow_left: &'static str,
    pub to: &'static str,
    pub dot: &'static str,
    pub active: &'static str,
    pub queued: &'static str,
    pub done: &'static str,
    pub failed: &'static str,
    pub paused: &'static str,
    pub ellipsis: &'static str,
    pub thumb: &'static str,
    pub track: &'static str,
    pub pointer: &'static str,
    /// Junctions indexed by a N=1, S=2, E=4, W=8 mask.
    pub junctions: [&'static str; 16],
}
const UNICODE: Glyphs = Glyphs {
    logo: "◆",
    bar: "▌",
    vertical: "│",
    horizontal: "─",
    dashed_vertical: "┆",
    dashed_horizontal: "┄",
    arrow_right: "▸",
    arrow_left: "◂",
    to: "→",
    dot: "·",
    active: "●",
    queued: "○",
    done: "✓",
    failed: "✕",
    paused: "‖",
    ellipsis: "…",
    thumb: "┃",
    track: "│",
    pointer: "▸",
    junctions: [
        " ", "│", "│", "│", "─", "╰", "╭", "├", "─", "╯", "╮", "┤", "─", "┴", "┬", "┼",
    ],
};
const ASCII: Glyphs = Glyphs {
    logo: "*",
    bar: "|",
    vertical: "|",
    horizontal: "-",
    dashed_vertical: ":",
    dashed_horizontal: ".",
    arrow_right: ">",
    arrow_left: "<",
    to: ">",
    dot: ".",
    active: "*",
    queued: "o",
    done: "x",
    failed: "x",
    paused: "=",
    ellipsis: "~",
    thumb: "#",
    track: "|",
    pointer: ">",
    junctions: [
        " ", "|", "|", "|", "-", "+", "+", "+", "-", "+", "+", "+", "-", "+", "+", "+",
    ],
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Active,
    Queued,
    Done,
    Failed,
    Waiting,
}
pub fn tone(status: &str) -> Tone {
    match status {
        "running" | "verifying" | "starting" | "sent" | "delivered" | "replied" => Tone::Active,
        "verified" | "completed" | "accepted" | "merged" | "applied" => Tone::Done,
        "failed" | "blocked" | "rejected" | "cancelled" | "changes_requested" | "error" => {
            Tone::Failed
        }
        "paused" | "waiting_agent" | "awaiting_verification" | "pending_unknown" => Tone::Waiting,
        _ => Tone::Queued,
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub color: bool,
    pub ascii: bool,
}
fn fg(index: u8) -> Style {
    Style::default().fg(Color::Indexed(index))
}
fn card(bg: u8, text: u8) -> Style {
    Style::default()
        .bg(Color::Indexed(bg))
        .fg(Color::Indexed(text))
}
impl Theme {
    pub fn new(color: bool, ascii: bool) -> Self {
        Self { color, ascii }
    }
    pub fn glyphs(&self) -> &'static Glyphs {
        if self.ascii { &ASCII } else { &UNICODE }
    }
    fn pick(&self, color: Style, plain: Style) -> Style {
        if self.color { color } else { plain }
    }
    pub fn text(&self) -> Style {
        self.pick(fg(252), Style::default())
    }
    pub fn dim(&self) -> Style {
        self.pick(fg(245), Style::default())
    }
    pub fn faint(&self) -> Style {
        self.pick(fg(240), Style::default())
    }
    pub fn accent(&self) -> Style {
        self.pick(fg(74), Style::default())
            .add_modifier(Modifier::BOLD)
    }
    pub fn title(&self) -> Style {
        self.pick(fg(255), Style::default())
            .add_modifier(Modifier::BOLD)
    }
    /// Header and footer bars.
    pub fn bar(&self) -> Style {
        self.pick(card(236, 250), Style::default())
    }
    /// Full-panel overlays (details, palette, confirmation).
    pub fn overlay(&self) -> Style {
        self.pick(card(234, 252), Style::default())
    }
    pub fn card(&self) -> Style {
        self.pick(card(235, 252), Style::default())
    }
    /// Floating overlays above the graph.
    pub fn panel(&self) -> Style {
        self.pick(card(236, 252), Style::default())
    }
    /// The focused card or the active composer.
    pub fn raised(&self) -> Style {
        self.pick(card(237, 254), Style::default())
    }
    pub fn selected(&self) -> Style {
        self.pick(
            card(24, 255),
            Style::default().add_modifier(Modifier::REVERSED),
        )
        .add_modifier(Modifier::BOLD)
    }
    pub fn separator(&self, focused: bool) -> Style {
        self.pick(fg(if focused { 74 } else { 238 }), Style::default())
    }
    pub fn status(&self, status: &str) -> Style {
        let color = match tone(status) {
            Tone::Active => 75,
            Tone::Done => 114,
            Tone::Failed => 203,
            Tone::Waiting => 179,
            Tone::Queued => 245,
        };
        self.pick(fg(color), Style::default())
    }
    pub fn status_glyph(&self, status: &str) -> &'static str {
        let g = self.glyphs();
        match tone(status) {
            Tone::Active => g.active,
            Tone::Done => g.done,
            Tone::Failed => g.failed,
            Tone::Waiting => g.paused,
            Tone::Queued => g.queued,
        }
    }
    /// A checklist box: open tasks are empty, others carry their status glyph.
    pub fn checkbox(&self, status: &str) -> String {
        match tone(status) {
            Tone::Queued => "[ ]".into(),
            _ => format!("[{}]", self.status_glyph(status)),
        }
    }
    /// A stable color for a message sender or edge kind.
    pub fn person(&self, name: &str) -> Style {
        let color = match name {
            "user" => 74,
            "orchestrator" => 176,
            "*" => 179,
            _ => 110,
        };
        self.pick(fg(color), Style::default())
    }
    pub fn edge(&self, kind: &str) -> Style {
        let color = match kind {
            "verifies" | "verified by" => 114,
            "depends_on" | "depends on" => 179,
            "communicates_with" | "communicates with" => 141,
            _ => 247,
        };
        self.pick(fg(color), Style::default())
    }
    pub fn warning(&self) -> Style {
        self.pick(fg(179), Style::default())
    }
    pub fn error(&self) -> Style {
        self.pick(fg(203), Style::default())
    }
    /// Removes control sequences and, in ASCII mode, every non-ASCII grapheme.
    pub fn clean(&self, text: &str) -> String {
        let text = super::sanitize::sanitize(text);
        if self.ascii {
            text.graphemes(true)
                .map(|g| {
                    if g.is_ascii() {
                        g.to_string()
                    } else {
                        "?".repeat(g.width())
                    }
                })
                .collect()
        } else {
            text
        }
    }
    /// Cuts text to a display width, marking the cut.
    pub fn fit(&self, text: &str, width: usize) -> String {
        if text.width() <= width {
            return text.into();
        }
        let ellipsis = self.glyphs().ellipsis;
        let room = width.saturating_sub(ellipsis.width());
        let mut out = String::new();
        let mut used = 0;
        for g in text.graphemes(true) {
            let w = g.width();
            if used + w > room {
                break;
            }
            out.push_str(g);
            used += w;
        }
        if width >= ellipsis.width() {
            out.push_str(ellipsis);
        }
        out
    }
}

/// Word wrap by display width. Long words are split; empty lines are kept.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = vec![];
    for paragraph in text.split('\n') {
        let mut line = String::new();
        let mut used = 0;
        for word in paragraph.split_word_bounds() {
            let w = word.width();
            if used + w > width && used > 0 {
                lines.push(std::mem::take(&mut line).trim_end().to_string());
                used = 0;
                if word.trim().is_empty() {
                    continue;
                }
            }
            if w > width {
                for g in word.graphemes(true) {
                    let gw = g.width();
                    if used + gw > width && used > 0 {
                        lines.push(std::mem::take(&mut line));
                        used = 0;
                    }
                    line.push_str(g);
                    used += gw;
                }
            } else {
                line.push_str(word);
                used += w;
            }
        }
        lines.push(line.trim_end().to_string());
    }
    lines
}

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
