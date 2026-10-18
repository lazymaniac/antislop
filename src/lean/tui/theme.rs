//! Visual tokens shared by every panel.
//! Cards are separated by background and accents, not heavy borders;
//! NO_COLOR keeps only modifiers, and ASCII mode keeps only ASCII glyphs.

use super::sanitize::sanitize;
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

pub const UNICODE: Glyphs = Glyphs {
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

pub const ASCII: Glyphs = Glyphs {
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
        "running" | "verifying" | "working" | "starting" | "sent" | "delivered" | "replied" => {
            Tone::Active
        }
        "verified" | "completed" | "approved" | "accepted" | "merged" | "applied" => Tone::Done,
        "failed" | "blocked" | "rejected" | "cancelled" | "changes_requested" | "error" => {
            Tone::Failed
        }
        "paused" | "waiting" | "waiting_agent" | "awaiting_verification" | "needs_user"
        | "pending_unknown" => Tone::Waiting,
        _ => Tone::Queued,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub color: bool,
    pub ascii: bool,
}

fn fg(index: u8) -> Style {
    Style::default().fg(Color::Indexed(index))
}

fn card_style(bg: u8, text: u8) -> Style {
    Style::default()
        .bg(Color::Indexed(bg))
        .fg(Color::Indexed(text))
}

impl Theme {
    pub fn new(color: bool, ascii: bool) -> Self {
        Self { color, ascii }
    }

    pub fn from_env() -> Self {
        let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
        let ascii = if std::env::var_os("ANTISLOP_ASCII").is_some_and(|v| !v.is_empty()) {
            true
        } else {
            let locale = ["LC_ALL", "LC_CTYPE", "LANG"]
                .iter()
                .find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()))
                .unwrap_or_default()
                .to_ascii_lowercase();
            !(locale.contains("utf-8") || locale.contains("utf8"))
        };
        Self::new(!no_color, ascii)
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
        self.pick(fg(245), Style::default().add_modifier(Modifier::DIM))
    }

    pub fn faint(&self) -> Style {
        self.pick(fg(240), Style::default().add_modifier(Modifier::DIM))
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
        self.pick(card_style(236, 250), Style::default())
    }

    /// Full-panel overlays (details, palette, confirmation).
    pub fn overlay(&self) -> Style {
        self.pick(card_style(234, 252), Style::default())
    }

    pub fn card(&self) -> Style {
        self.pick(card_style(235, 252), Style::default())
    }

    /// Floating overlays above the graph.
    pub fn panel(&self) -> Style {
        self.pick(card_style(236, 252), Style::default())
    }

    /// The focused card or the active composer.
    pub fn raised(&self) -> Style {
        self.pick(card_style(237, 254), Style::default())
    }

    pub fn selected(&self) -> Style {
        self.pick(
            card_style(24, 255),
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

    /// A stable color for a message sender or recipient.
    pub fn person(&self, name: &str) -> Style {
        let color = match name {
            "user" | "Ty" => 74,
            "orchestrator" | "orkiestrator" => 176,
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
        self.pick(fg(179), Style::default().add_modifier(Modifier::BOLD))
    }

    pub fn error(&self) -> Style {
        self.pick(fg(203), Style::default().add_modifier(Modifier::BOLD))
    }

    /// Removes control sequences and, in ASCII mode, replaces non-ASCII graphemes.
    pub fn clean(&self, text: &str) -> String {
        let text = sanitize(text);
        if self.ascii {
            text.graphemes(true)
                .map(|g| {
                    if g.is_ascii() {
                        g.to_string()
                    } else {
                        // Polish diacritics stand-in
                        match g {
                            "ą" => "a".into(),
                            "ć" => "c".into(),
                            "ę" => "e".into(),
                            "ł" => "l".into(),
                            "ń" => "n".into(),
                            "ó" => "o".into(),
                            "ś" => "s".into(),
                            "ź" | "ż" => "z".into(),
                            "Ą" => "A".into(),
                            "Ć" => "C".into(),
                            "Ę" => "E".into(),
                            "Ł" => "L".into(),
                            "Ń" => "N".into(),
                            "Ó" => "O".into(),
                            "Ś" => "S".into(),
                            "Ź" | "Ż" => "Z".into(),
                            "→" | "▶" | "↳" => ">".into(),
                            "←" | "◀" => "<".into(),
                            "↑" | "▲" => "^".into(),
                            "↓" | "▼" => "v".into(),
                            "✓" => "+".into(),
                            "✕" | "✗" => "x".into(),
                            "·" | "–" | "—" | "−" => "-".into(),
                            "…" => ".".into(),
                            _ => "?".repeat(g.width().max(1)),
                        }
                    }
                })
                .collect()
        } else {
            text
        }
    }

    /// Cuts text to a display width, marking the cut with an ellipsis.
    pub fn fit(&self, text: &str, width: usize) -> String {
        if text.width() <= width {
            return text.into();
        }
        let ellipsis = self.glyphs().ellipsis;
        let ellipsis_w = ellipsis.width();
        let room = width.saturating_sub(ellipsis_w);
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
        if width >= ellipsis_w {
            out.push_str(ellipsis);
        }
        out
    }
}

/// Word wrap by display width, respecting grapheme cluster boundaries.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = vec![];
    let sanitized = sanitize(text);
    for paragraph in sanitized.split('\n') {
        let mut line = String::new();
        let mut used = 0;
        for word in paragraph.split_inclusive(' ') {
            let w = word.width();
            if used > 0 && used + w.min(width) > width {
                lines.push(std::mem::take(&mut line));
                used = 0;
            }
            for g in word.graphemes(true) {
                let gw = g.width();
                if used + gw > width && used > 0 {
                    lines.push(std::mem::take(&mut line));
                    used = 0;
                }
                line.push_str(g);
                used += gw;
            }
        }
        lines.push(line);
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

/// A thin scroll indicator; nothing is drawn when everything fits.
pub fn scrollbar(buf: &mut Buffer, theme: &Theme, area: Rect, total: usize, top: usize, view: usize) {
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
            buf[(area.x, py)]
                .set_symbol(symbol)
                .set_style(style);
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

/// A card: a colored accent bar, a heading row, wrapped body rows and a gap.
pub fn card(
    theme: &Theme,
    fill: Style,
    accent: Style,
    head: Vec<Span<'static>>,
    right: String,
    body: &str,
    max_body: usize,
    width: u16,
) -> Vec<Row> {
    let w = usize::from(width);
    if w == 0 {
        return vec![];
    }
    let bar_span = || Span::styled(theme.glyphs().bar, fill.patch(accent));
    let mut left = vec![bar_span(), Span::styled(" ", fill)];
    left.extend(
        head.into_iter()
            .map(|span| Span::styled(span.content.into_owned(), fill.patch(span.style))),
    );
    let right_width = w / 2;
    let mut out = vec![split_row(
        fill,
        left,
        Span::styled(theme.fit(&right, right_width), fill.patch(theme.dim())),
        w,
    )];
    let room = w.saturating_sub(3);
    let lines = if body.is_empty() {
        vec![]
    } else {
        wrap(&theme.clean(body), room)
    };
    let cut = lines.len() > max_body;
    for (i, line) in lines.iter().take(max_body).enumerate() {
        let line = if cut && i + 1 == max_body {
            theme.fit(&format!("{line}{}", theme.glyphs().ellipsis), room)
        } else {
            line.clone()
        };
        out.push(Row::new(
            fill,
            vec![bar_span(), Span::styled(" ", fill), Span::styled(line, fill.patch(theme.text()))],
        ));
    }
    out.push(Row::gap());
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_tones_and_glyphs() {
        assert_eq!(tone("running"), Tone::Active);
        assert_eq!(tone("verified"), Tone::Done);
        assert_eq!(tone("failed"), Tone::Failed);
        assert_eq!(tone("paused"), Tone::Waiting);
        assert_eq!(tone("queued"), Tone::Queued);

        let t_uni = Theme::new(true, false);
        assert_eq!(t_uni.glyphs().done, "✓");
        assert_eq!(t_uni.status_glyph("completed"), "✓");

        let t_ascii = Theme::new(false, true);
        assert_eq!(t_ascii.glyphs().done, "x");
        assert_eq!(t_ascii.status_glyph("completed"), "x");
    }

    #[test]
    fn theme_grapheme_wrapping_and_fit() {
        let t = Theme::new(true, false);
        let wrapped = wrap("ala ma kota", 7);
        assert_eq!(wrapped, ["ala ma ", "kota"]);

        let fit_res = t.fit("bardzodlugitekst", 10);
        assert!(fit_res.ends_with('…'));
        assert_eq!(fit_res.width(), 10);
    }
}
