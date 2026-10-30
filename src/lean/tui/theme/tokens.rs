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
    pub plan_verifier: &'static str,
    pub auditor: &'static str,
    pub worker: &'static str,
    pub verifier: &'static str,
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
    plan_verifier: "◇",
    auditor: "≡",
    worker: "▸",
    verifier: "◈",
    spinner: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
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
    waiting: "=",
    problem: "x",
    orchestrator: "O",
    plan_verifier: "P",
    auditor: "A",
    worker: ">",
    verifier: "V",
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
        "running" | "verifying" | "working" | "starting" | "sent" | "delivered" | "replied" => {
            Tone::Active
        }
        "verified" | "completed" | "approved" | "accepted" | "merged" | "applied" => Tone::Done,
        "failed" | "blocked" | "rejected" | "cancelled" | "changes_requested" | "error" => {
            Tone::Failed
        }
        "paused"
        | "waiting"
        | "waiting_agent"
        | "awaiting_verification"
        | "coordinating"
        | "needs_user"
        | "pending_unknown" => Tone::Waiting,
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

    pub const BG: Color = Color::Rgb(0x0B, 0x0E, 0x14);
    pub const PANEL: Color = Color::Rgb(0x0F, 0x13, 0x1A);
    pub const LINE: Color = Color::Rgb(0x15, 0x1A, 0x23);
    pub const RAISED: Color = Color::Rgb(0x1B, 0x21, 0x2C);
    pub const SELECTION: Color = Color::Rgb(0x27, 0x37, 0x47);
    pub const FG: Color = Color::Rgb(0xBF, 0xBD, 0xB6);
    pub const TITLE: Color = Color::Rgb(0xE6, 0xE1, 0xCF);
    pub const DIM: Color = Color::Rgb(0x56, 0x5B, 0x66);
    pub const FAINT: Color = Color::Rgb(0x3E, 0x44, 0x51);
    pub const BLUE: Color = Color::Rgb(0x59, 0xC2, 0xFF);
    pub const GREEN: Color = Color::Rgb(0xAA, 0xD9, 0x4C);
    pub const YELLOW: Color = Color::Rgb(0xFF, 0xB4, 0x54);
    pub const ORANGE: Color = Color::Rgb(0xFF, 0x8F, 0x40);
    pub const RED: Color = Color::Rgb(0xF0, 0x71, 0x78);
    pub const PURPLE: Color = Color::Rgb(0xD2, 0xA6, 0xFF);
    pub const CYAN: Color = Color::Rgb(0x95, 0xE6, 0xCB);
    pub const GOLD: Color = Color::Rgb(0xE6, 0xB4, 0x50);
    pub const PINK: Color = Color::Rgb(0xF2, 0x96, 0x68);
}
