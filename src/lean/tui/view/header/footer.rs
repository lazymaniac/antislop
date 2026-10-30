use super::*;

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
