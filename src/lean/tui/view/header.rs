use super::*;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// The bar's buttons: padded with an icon on wide terminals, plain words
/// on medium ones and icons alone on narrow ones. A click finds them by
/// their brackets.
pub const PAUSE: &str = "[ ‖ Pause ]";
pub const CONTINUE: &str = "[ ▶ Continue ]";
pub const RESUME: &str = "[ ↻ Resume ]";
pub const CANCEL: &str = "[ ✕ Cancel ]";
pub const PLAN: &str = "[Plan]";
pub const NEW: &str = "[ + New ]";
pub const PAUSE_SHORT: &str = "[Pause]";
pub const CONTINUE_SHORT: &str = "[Continue]";
pub const RESUME_SHORT: &str = "[Resume]";
pub const CANCEL_SHORT: &str = "[Cancel]";
pub const NEW_SHORT: &str = "[+ New]";
pub const PAUSE_ICON: &str = "[‖]";
pub const CONTINUE_ICON: &str = "[▶]";
pub const RESUME_ICON: &str = "[↻]";
pub const CANCEL_ICON: &str = "[✕]";
pub const NEW_ICON: &str = "[+]";
/// From this width the buttons carry their icon and padding.
const BIG_BUTTONS: u16 = 140;

/// Builds the responsive header according to width, mode, status, and available actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Controls {
    pub pause: bool,
    pub resume_paused: bool,
    pub resume_plan: bool,
    pub cancel: bool,
}

impl Default for Controls {
    /// All of them, as before the header knew the state.
    fn default() -> Self {
        Self {
            pause: true,
            resume_paused: true,
            resume_plan: true,
            cancel: true,
        }
    }
}

impl View {
    /// What the buttons can do in the state the conversation is in.
    pub fn controls(&self) -> Controls {
        Controls {
            pause: !self.idle && !self.paused,
            resume_paused: self.paused,
            resume_plan: self.plan_resumable(),
            cancel: !self.idle,
        }
    }
}

pub fn render_header(
    project: &str,
    conversation: &str,
    mode: Option<bool>,
    status: &str,
    focus: PanelFocus,
    width: u16,
) -> Line<'static> {
    render_header_with(
        project,
        conversation,
        mode,
        status,
        focus,
        width,
        Controls::default(),
    )
}

pub fn render_header_with(
    project: &str,
    conversation: &str,
    mode: Option<bool>,
    status: &str,
    focus: PanelFocus,
    width: u16,
    controls: Controls,
) -> Line<'static> {
    let theme = crate::lean::tui::theme::Theme::from_env();
    let label_style = theme.text().add_modifier(Modifier::BOLD);
    let secondary = theme.text();
    let active = active_style(&theme);
    if width < 25 {
        return compact_header(project, label_style);
    }
    let short = conversation.get(..8).unwrap_or(conversation);
    let mut spans = vec![Span::styled(
        format!(" {} ", clean_line(project)),
        label_style,
    )];
    append_panels(&mut spans, width, focus, &theme, secondary);
    append_identity(&mut spans, short, mode, width, secondary, active);
    append_status_and_actions(&mut spans, status, controls, width, &theme, secondary);
    Line::from(spans)
}

fn compact_header(project: &str, style: Style) -> Line<'static> {
    Line::from(vec![Span::styled(
        format!(" {} ", clean_line(project)),
        style,
    )])
}

fn active_style(theme: &crate::lean::tui::theme::Theme) -> Style {
    let raised = if theme.color {
        theme.raised().fg(crate::lean::tui::theme::ayu::ORANGE)
    } else {
        theme.raised()
    };
    raised.add_modifier(Modifier::BOLD)
}

fn append_panels(
    spans: &mut Vec<Span<'static>>,
    width: u16,
    focus: PanelFocus,
    theme: &crate::lean::tui::theme::Theme,
    secondary: Style,
) {
    if width >= SIDE_BY_SIDE_BREAKPOINT {
        return;
    }
    let active = active_style(theme);
    let inactive = theme.panel().patch(secondary);
    let (chat, graph) = if focus == PanelFocus::Chat {
        (active, inactive)
    } else {
        (inactive, active)
    };
    spans.push(Span::styled(" [Chat] ", chat));
    spans.push(Span::styled("[Team] ", graph));
    if width >= 65 {
        spans.push(Span::styled("(Tab: switch) ", secondary));
    }
}

fn append_identity(
    spans: &mut Vec<Span<'static>>,
    short: &str,
    mode: Option<bool>,
    width: u16,
    secondary: Style,
    active: Style,
) {
    if width >= 50 {
        let mode = match mode {
            Some(true) => " · [Mode: autonomous]",
            Some(false) => " · [Mode: supervised]",
            None => "",
        };
        spans.push(Span::styled(format!(" · {short}{mode} "), secondary));
    }
    if (50..95).contains(&width) {
        spans.push(Span::styled(format!("{PLAN} "), active));
    }
    if (35..50).contains(&width) {
        match mode {
            Some(true) => spans.push(Span::styled("[auto] ", secondary)),
            Some(false) => spans.push(Span::styled("[sup] ", secondary)),
            None => {}
        }
    }
}

fn button(
    label: &str,
    enabled: bool,
    theme: &crate::lean::tui::theme::Theme,
    secondary: Style,
) -> Span<'static> {
    let style = if enabled {
        active_style(theme)
    } else {
        theme.panel().patch(secondary)
    };
    Span::styled(label.to_owned(), style)
}

fn append_status_and_actions(
    spans: &mut Vec<Span<'static>>,
    status: &str,
    controls: Controls,
    width: u16,
    theme: &crate::lean::tui::theme::Theme,
    secondary: Style,
) {
    if width >= 40 && !status.trim().is_empty() {
        spans.push(Span::styled(
            format!(" · {} ", clean_line(status)),
            Style::default().fg(crate::lean::tui::theme::ayu::YELLOW),
        ));
    }
    if width >= 95 {
        append_full_actions(spans, controls, width, theme, secondary);
    } else if width >= 45 {
        append_short_actions(spans, controls, theme, secondary);
    }
}

fn append_full_actions(
    spans: &mut Vec<Span<'static>>,
    controls: Controls,
    width: u16,
    theme: &crate::lean::tui::theme::Theme,
    secondary: Style,
) {
    let big = width >= BIG_BUTTONS;
    spans.push(Span::styled(" · ", secondary));
    let labels = if big {
        [PAUSE, CONTINUE, RESUME, CANCEL]
    } else {
        [PAUSE_SHORT, CONTINUE_SHORT, RESUME_SHORT, CANCEL_SHORT]
    };
    let enabled = [
        controls.pause,
        controls.resume_paused,
        controls.resume_plan,
        controls.cancel,
    ];
    for (label, enabled) in labels.into_iter().zip(enabled) {
        spans.push(button(label, enabled, theme, secondary));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::styled(PLAN, active_style(theme)));
    spans.push(Span::raw(" "));
    let new = if big { NEW } else { NEW_SHORT };
    spans.push(Span::styled(new, active_style(theme)));
}

fn append_short_actions(
    spans: &mut Vec<Span<'static>>,
    controls: Controls,
    theme: &crate::lean::tui::theme::Theme,
    secondary: Style,
) {
    let (label, enabled) = if controls.resume_paused {
        (CONTINUE_ICON, true)
    } else {
        (PAUSE_ICON, controls.pause)
    };
    spans.push(button(label, enabled, theme, secondary));
    spans.push(button(RESUME_ICON, controls.resume_plan, theme, secondary));
    spans.push(button(CANCEL_ICON, controls.cancel, theme, secondary));
    spans.push(button(NEW_ICON, true, theme, secondary));
    spans.push(Span::raw(" "));
}

/// Prioritizes action hints and notices by available footer width.
pub fn main_footer_hint(
    focus: PanelFocus,
    plan_proposed: bool,
    plan_resumable: bool,
    notice: Option<&str>,
    width: usize,
) -> String {
    if let Some(notice) = notice {
        return truncate(notice, width);
    }
    let plan = plan_hint(plan_proposed, plan_resumable);
    match focus {
        PanelFocus::Chat => chat_footer(plan, width),
        PanelFocus::Graph => graph_footer(plan, width),
    }
}

fn plan_hint(proposed: bool, resumable: bool) -> &'static str {
    if proposed {
        "Ctrl+A accept plan · "
    } else if resumable {
        "Ctrl+R resume plan · "
    } else {
        ""
    }
}

fn chat_footer(plan: &str, width: usize) -> String {
    if width >= 115 {
        format!(
            "Enter send (@t2 … to agent) · Alt+Enter newline · {plan}Ctrl+P commands · F1 help · Tab team · Ctrl+C/q quit"
        )
    } else if width >= 75 {
        format!("Enter send · Alt+Enter newline · {plan}Ctrl+P · F1 help · Tab team · Esc")
    } else if width >= 50 {
        format!("Enter send · {plan}Tab team · Esc")
    } else {
        "Enter · Esc".into()
    }
}

fn graph_footer(plan: &str, width: usize) -> String {
    if width >= 115 {
        format!(
            "arrows select · PgUp/PgDn scroll text · Enter details · / find · d diff · c comment · {plan}? help · q quit"
        )
    } else if width >= 75 {
        "arrows select · PgUp/PgDn text · Enter details · / find · d diff · ? help · q".into()
    } else if width >= 50 {
        "arrows · Enter details · Tab chat · q".into()
    } else {
        "arrows · q".into()
    }
}

/// Empty state displayed when transcript contains no messages.
pub fn transcript_empty_state(width: usize) -> Vec<Line<'static>> {
    vec![
        Line::styled("No messages in this conversation.", dim()),
        Line::styled(
            truncate(
                "Type a message to the orchestrator below or press Ctrl+P to open the command palette.",
                width,
            ),
            dim(),
        ),
    ]
}
