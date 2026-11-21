//! Theme glyph catalogs, status tones, and palette constants.

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
    pub waiting: &'static str,
    pub problem: &'static str,
    pub orchestrator: &'static str,
    pub plan_reviewer: &'static str,
    pub auditor: &'static str,
    pub worker: &'static str,
    pub reviewer: &'static str,
    /// Frames of the busy indicator, advanced by the UI tick.
    pub spinner: &'static [&'static str],
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
    waiting: "‖",
    problem: "✕",
    orchestrator: "◆",
    plan_reviewer: "◇",
    auditor: "≡",
    worker: "▸",
    reviewer: "◈",
    spinner: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
    junctions: [
        " ", "│", "│", "│", "─", "└", "┌", "├", "─", "┘", "┐", "┤", "─", "┴", "┬", "┼",
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
    waiting: "=",
    problem: "x",
    orchestrator: "O",
    plan_reviewer: "P",
    auditor: "A",
    worker: ">",
    reviewer: "R",
    spinner: &["|", "/", "-", "\\"],
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
        "running" | "reviewing" | "working" | "starting" | "sent" | "delivered" | "replied" => {
            Tone::Active
        }
        "done" | "reviewed" | "completed" | "approved" | "accepted" | "merged" | "applied" => {
            Tone::Done
        }
        "failed" | "blocked" | "rejected" | "cancelled" | "changes_requested" | "error" => {
            Tone::Failed
        }
        "paused" | "waiting" | "waiting_agent" | "awaiting_review" | "coordinating"
        | "needs_user" | "pending_unknown" => Tone::Waiting,
        _ => Tone::Queued,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub color: bool,
    pub ascii: bool,
}

/// The Ayu Dark palette: every colour the TUI paints comes from here.
pub mod ayu {
    use ratatui::style::Color;

    pub const BG: Color = Color::Rgb(0x10, 0x16, 0x19);
    pub const PANEL: Color = Color::Rgb(0x19, 0x22, 0x26);
    pub const LINE: Color = Color::Rgb(0x19, 0x22, 0x26);
    pub const RAISED: Color = Color::Rgb(0x22, 0x30, 0x35);
    pub const SELECTION: Color = Color::Rgb(0x2A, 0x3C, 0x40);
    pub const FG: Color = Color::Rgb(0xE7, 0xED, 0xEB);
    pub const TITLE: Color = Color::Rgb(0xE7, 0xED, 0xEB);
    pub const DIM: Color = Color::Rgb(0xA1, 0xB1, 0xB3);
    pub const FAINT: Color = Color::Rgb(0xA1, 0xB1, 0xB3);
    pub const BLUE: Color = Color::Rgb(0x59, 0xC2, 0xFF);
    pub const GREEN: Color = Color::Rgb(0xAA, 0xD9, 0x4C);
    pub const YELLOW: Color = Color::Rgb(0xFF, 0xB4, 0x54);
    pub const ORANGE: Color = Color::Rgb(0xFF, 0x8F, 0x40);
    pub const RED: Color = Color::Rgb(0xF0, 0x71, 0x78);
    pub const PURPLE: Color = Color::Rgb(0xD2, 0xA6, 0xFF);
    pub const CYAN: Color = Color::Rgb(0xA4, 0xDF, 0xCC);
    pub const GOLD: Color = Color::Rgb(0xE9, 0xC5, 0x93);
    pub const PINK: Color = Color::Rgb(0xF2, 0x96, 0x68);
}
