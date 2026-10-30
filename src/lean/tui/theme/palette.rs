//! Color and text operations for the shared TUI theme.

use super::super::sanitize::sanitize;
use super::ascii::ascii_grapheme;
use super::tokens::{ASCII, Glyphs, UNICODE};
use super::tokens::{Theme, Tone, ayu, tone};
use ratatui::style::{Color, Modifier, Style};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

fn fg(color: Color) -> Style {
    Style::default().fg(color)
}

fn card_style(bg: Color, text: Color) -> Style {
    Style::default().bg(bg).fg(text)
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

    /// One frame of the busy indicator for the given UI tick.
    pub fn spinner(&self, tick: u64) -> &'static str {
        let frames = self.glyphs().spinner;
        frames[(tick as usize) % frames.len()]
    }

    /// The icon that tells agent kinds apart: `key` is a graph node key.
    pub fn role_icon(&self, key: &str) -> &'static str {
        let g = self.glyphs();
        match key.split_once(':').map_or(key, |(role, _)| role) {
            "worker" => g.worker,
            "verifier" => g.verifier,
            "plan_verifier" | "plan-verifier" => g.plan_verifier,
            "auditor" => g.auditor,
            _ => g.orchestrator,
        }
    }

    /// The accent color of an agent kind; `kind` is the task's work kind.
    pub fn role_accent(&self, key: &str, kind: &str) -> Style {
        let role = key.split_once(':').map_or(key, |(role, _)| role);
        let color = match role {
            "plan_verifier" | "plan-verifier" => ayu::PURPLE,
            "auditor" => ayu::CYAN,
            "worker" | "verifier" => {
                let base = match kind {
                    "analysis" => ayu::BLUE,
                    "implementation" => ayu::GREEN,
                    "static_analysis" => ayu::YELLOW,
                    "testing" => ayu::ORANGE,
                    "integration" => ayu::PINK,
                    _ => ayu::CYAN,
                };
                // A verifier shares its worker's hue, dimmed below.
                base
            }
            _ => ayu::PURPLE,
        };
        let style = self.pick(fg(color), Style::default());
        if role == "verifier" {
            style.add_modifier(Modifier::DIM)
        } else {
            style
        }
    }

    fn pick(&self, color: Style, plain: Style) -> Style {
        if self.color { color } else { plain }
    }

    pub fn text(&self) -> Style {
        self.pick(fg(ayu::FG), Style::default())
    }

    pub fn dim(&self) -> Style {
        self.pick(fg(ayu::DIM), Style::default().add_modifier(Modifier::DIM))
    }

    pub fn faint(&self) -> Style {
        self.pick(fg(ayu::FAINT), Style::default().add_modifier(Modifier::DIM))
    }

    pub fn accent(&self) -> Style {
        self.pick(fg(ayu::BLUE), Style::default())
            .add_modifier(Modifier::BOLD)
    }

    pub fn title(&self) -> Style {
        self.pick(fg(ayu::TITLE), Style::default())
            .add_modifier(Modifier::BOLD)
    }

    /// Header and footer bars.
    pub fn bar(&self) -> Style {
        self.pick(card_style(ayu::LINE, ayu::FG), Style::default())
    }

    /// Full-panel overlays (details, palette, confirmation).
    pub fn overlay(&self) -> Style {
        self.pick(card_style(ayu::BG, ayu::FG), Style::default())
    }

    pub fn card(&self) -> Style {
        self.pick(card_style(ayu::PANEL, ayu::FG), Style::default())
    }

    /// Floating overlays above the graph.
    pub fn panel(&self) -> Style {
        self.pick(card_style(ayu::LINE, ayu::FG), Style::default())
    }

    /// The focused card or the active composer.
    pub fn raised(&self) -> Style {
        self.pick(card_style(ayu::RAISED, ayu::TITLE), Style::default())
    }

    pub fn selected(&self) -> Style {
        self.pick(
            card_style(ayu::SELECTION, ayu::TITLE),
            Style::default().add_modifier(Modifier::REVERSED),
        )
        .add_modifier(Modifier::BOLD)
    }

    /// The primary phase action above the composer: a filled, bold button.
    pub fn button(&self) -> Style {
        self.pick(
            card_style(ayu::GOLD, ayu::BG),
            Style::default().add_modifier(Modifier::REVERSED),
        )
        .add_modifier(Modifier::BOLD)
    }

    pub fn separator(&self, focused: bool) -> Style {
        self.pick(
            fg(if focused { ayu::BLUE } else { ayu::FAINT }),
            Style::default(),
        )
    }

    pub fn status(&self, status: &str) -> Style {
        let color = match tone(status) {
            Tone::Active => ayu::BLUE,
            Tone::Done => ayu::GREEN,
            Tone::Failed => ayu::RED,
            Tone::Waiting => ayu::YELLOW,
            Tone::Queued => ayu::DIM,
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
            "user" | "You" => ayu::BLUE,
            "orchestrator" => ayu::PURPLE,
            "*" => ayu::YELLOW,
            _ => ayu::CYAN,
        };
        self.pick(fg(color), Style::default())
    }

    pub fn edge(&self, kind: &str) -> Style {
        let color = match kind {
            "verifies" | "verified by" => ayu::GREEN,
            "depends_on" | "depends on" => ayu::YELLOW,
            "communicates_with" | "communicates with" => ayu::PURPLE,
            _ => ayu::DIM,
        };
        self.pick(fg(color), Style::default())
    }

    pub fn ok(&self) -> Style {
        self.pick(fg(ayu::GREEN), Style::default())
    }

    pub fn info(&self) -> Style {
        self.pick(fg(ayu::CYAN), Style::default())
    }

    pub fn warn(&self) -> Style {
        self.pick(fg(ayu::YELLOW), Style::default())
    }

    pub fn err(&self) -> Style {
        self.pick(fg(ayu::RED), Style::default())
    }

    pub fn purple(&self) -> Style {
        self.pick(fg(ayu::PURPLE), Style::default())
    }

    pub fn warning(&self) -> Style {
        self.pick(
            fg(ayu::YELLOW),
            Style::default().add_modifier(Modifier::BOLD),
        )
    }

    pub fn error(&self) -> Style {
        self.pick(fg(ayu::RED), Style::default().add_modifier(Modifier::BOLD))
    }

    /// Removes control sequences and, in ASCII mode, replaces non-ASCII graphemes.
    pub fn clean(&self, text: &str) -> String {
        let text = sanitize(text);
        if self.ascii {
            text.graphemes(true).map(ascii_grapheme).collect()
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
