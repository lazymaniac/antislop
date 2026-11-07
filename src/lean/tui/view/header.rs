use super::*;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

mod footer;
pub use footer::{main_footer_hint, transcript_empty_state};

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
    status: &str,
    focus: PanelFocus,
    width: u16,
) -> Line<'static> {
    render_header_with(
        project,
        conversation,
        status,
        focus,
        width,
        Controls::default(),
    )
}

pub fn render_header_with(
    project: &str,
    conversation: &str,
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
    // Each part with the rank it is dropped in when the bar is too narrow:
    // the highest goes first, the project label never.
    let mut parts: Vec<(u8, Vec<Span<'static>>)> = Vec::new();
    parts.push((
        0,
        vec![Span::styled(
            format!(" {} ", clean_line(project)),
            label_style,
        )],
    ));
    let mut panels = Vec::new();
    append_panels(&mut panels, width, focus, &theme, secondary);
    parts.push((0, panels));
    if (65..SIDE_BY_SIDE_BREAKPOINT).contains(&width) {
        parts.push((4, vec![Span::styled("(Tab: switch) ", secondary)]));
    }
    let (identity, plan) = identity_parts(short, width, secondary, active);
    parts.push((5, identity));
    parts.push((2, plan));
    let mut status_part = Vec::new();
    let mut actions = Vec::new();
    append_status_and_actions(
        &mut status_part,
        &mut actions,
        status,
        controls,
        width,
        &theme,
        secondary,
    );
    parts.push((3, status_part));
    parts.push((1, actions));
    let fits = |parts: &[(u8, Vec<Span<'static>>)]| {
        parts
            .iter()
            .flat_map(|(_, spans)| spans)
            .map(Span::width)
            .sum::<usize>()
            <= usize::from(width)
    };
    while !fits(&parts) {
        let Some(drop) = parts
            .iter()
            .enumerate()
            .filter(|(_, (rank, spans))| *rank > 0 && !spans.is_empty())
            .max_by_key(|(index, (rank, _))| (*rank, *index))
            .map(|(index, _)| index)
        else {
            break;
        };
        parts[drop].1.clear();
    }
    let mut spans: Vec<Span<'static>> = parts.into_iter().flat_map(|(_, spans)| spans).collect();
    let overflow = spans
        .iter()
        .map(Span::width)
        .sum::<usize>()
        .saturating_sub(usize::from(width));
    if overflow > 0 {
        let label = &mut spans[0];
        let room = label.width().saturating_sub(overflow + 2);
        label.content = format!(" {} ", truncate(&clean_line(project), room)).into();
    }
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
}

/// The conversation's id and, below the full action row, its `[Plan]`.
fn identity_parts(
    short: &str,
    width: u16,
    secondary: Style,
    active: Style,
) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    if width < 50 {
        return (Vec::new(), Vec::new());
    }
    let identity = vec![Span::styled(format!(" · {short} "), secondary)];
    let plan = if width < 95 {
        vec![Span::styled(format!("{PLAN} "), active)]
    } else {
        Vec::new()
    };
    (identity, plan)
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
    status_spans: &mut Vec<Span<'static>>,
    spans: &mut Vec<Span<'static>>,
    status: &str,
    controls: Controls,
    width: u16,
    theme: &crate::lean::tui::theme::Theme,
    secondary: Style,
) {
    if width >= 40 && !status.trim().is_empty() {
        status_spans.push(Span::styled(
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
    // Pause and Continue share one slot: only the action the state allows is shown.
    let toggle = match (controls.resume_paused, big) {
        (true, true) => (CONTINUE, true),
        (true, false) => (CONTINUE_SHORT, true),
        (false, true) => (PAUSE, controls.pause),
        (false, false) => (PAUSE_SHORT, controls.pause),
    };
    let labels = if big {
        [toggle.0, RESUME, CANCEL]
    } else {
        [toggle.0, RESUME_SHORT, CANCEL_SHORT]
    };
    let enabled = [toggle.1, controls.resume_plan, controls.cancel];
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
