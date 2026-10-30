use super::catalog::SELECTION_HELP;
use super::*;
/// Where the help was opened, for the keys it lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Help {
    Chat,
    Graph,
    Details,
    Diff,
    Comment,
    Picker,
}

impl Help {
    fn lines(self) -> &'static [&'static str] {
        match self {
            Help::Chat => help_chat(),
            Help::Graph => help_graph(),
            Help::Details => help_details(),
            Help::Diff => help_diff(),
            Help::Comment => help_comment(),
            Help::Picker => help_picker(),
        }
    }

    pub fn lines_for_width(self, inner: usize) -> Vec<String> {
        self.lines()
            .iter()
            .flat_map(|line| help_line(line, inner))
            .collect()
    }

    pub fn max_scroll(self, area: Rect) -> usize {
        if area.width == 0 || area.height == 0 {
            return 0;
        }
        let width = if area.width >= 8 {
            area.width.saturating_sub(2).min(84)
        } else {
            area.width
        };
        let inner = usize::from(width.saturating_sub(2));
        let lines = self.lines_for_width(inner);
        let height = (lines.len() as u16 + 2).min(area.height).max(1);
        let visible_lines = usize::from(height.saturating_sub(2));
        lines.len().saturating_sub(visible_lines)
    }

    pub fn draw(self, frame: &mut Frame) {
        self.draw_scroll(frame, 0);
    }

    pub fn draw_scroll(self, frame: &mut Frame, scroll: usize) {
        let area = frame.area();
        if area.width == 0 || area.height == 0 {
            return;
        }
        let width = help_width(area.width);
        let inner = usize::from(width.saturating_sub(2));
        let lines = self.lines_for_width(inner);
        draw_help_box(frame, area, width, lines, scroll);
    }
}

fn help_width(width: u16) -> u16 {
    if width >= 8 {
        width.saturating_sub(2).min(84)
    } else {
        width
    }
}

fn draw_help_box(frame: &mut Frame, area: Rect, width: u16, lines: Vec<String>, scroll: usize) {
    let height = (lines.len() as u16 + 2).min(area.height).max(1);
    let box_area = help_box_area(area, width, height);
    let visible = usize::from(height.saturating_sub(2));
    let max_scroll = lines.len().saturating_sub(visible);
    let shown = lines
        .iter()
        .skip(scroll.min(max_scroll))
        .take(visible)
        .map(|line| Line::raw(line.clone()))
        .collect::<Vec<_>>();
    let title = help_title(width, max_scroll > 0);
    frame.render_widget(Clear, box_area);
    frame.render_widget(
        Paragraph::new(shown).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(super::super::theme::ayu::BLUE))
                .title(title),
        ),
        box_area,
    );
}

fn help_box_area(area: Rect, width: u16, height: u16) -> Rect {
    Rect::new(
        area.x + (area.width.saturating_sub(width)) / 2,
        area.y + area.height.saturating_sub(height) / 3,
        width,
        height,
    )
}

fn help_title(width: u16, scrolling: bool) -> &'static str {
    match (scrolling, width) {
        (true, 44..) => " help · ↑/↓ scroll · Esc closes ",
        (true, 26..) => " help · ↑/↓ · Esc ",
        (false, 36..) => " help · Esc closes ",
        (false, 14..) => " help · Esc ",
        _ => " help ",
    }
}

fn compact_help_line(line: &str) -> String {
    let mut parts = line.splitn(2, "  ");
    if let (Some(key), Some(desc)) = (parts.next(), parts.next()) {
        let key = key.trim();
        let desc = desc.trim();
        if !key.is_empty() && !desc.is_empty() {
            return format!("{key}: {desc}");
        }
    }
    line.to_string()
}

fn help_line(line: &str, inner: usize) -> Vec<String> {
    if inner >= 60 {
        return vec![truncate(line, inner)];
    }
    let compact = compact_help_line(line);
    if compact.chars().count() <= inner {
        return vec![compact];
    }
    if let Some((key, desc)) = compact.split_once(": ") {
        return wrap_help_description(key, desc, inner);
    }
    vec![truncate(&compact, inner)]
}

fn wrap_help_description(key: &str, desc: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = format!("{key}: ");
    for word in desc.split_whitespace() {
        append_help_word(&mut current, word, width, &mut out);
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

fn append_help_word(current: &mut String, word: &str, width: usize, out: &mut Vec<String>) {
    let has_prefix = current.ends_with(": ") || current.ends_with("   ");
    let needed = word.chars().count() + usize::from(!has_prefix);
    if current.chars().count() + needed > width && current.chars().count() > 3 {
        out.push(std::mem::replace(current, format!("   {word}")));
    } else {
        if !has_prefix {
            current.push(' ');
        }
        current.push_str(word);
    }
}

fn help_chat() -> &'static [&'static str] {
    &[
        "Enter, Ctrl+S    send message to orchestrator",
        "@t2 text         message worker t2 (copied in chat)",
        "Alt+Enter, Ctrl+J  newline in draft",
        "↑/↓              line in draft; sent history at edges",
        "PgUp/PgDn        scroll conversation",
        SELECTION_HELP,
        "Ctrl+D / Ctrl+U  half page down / up",
        "Tab              team panel",
        "Ctrl+A / Ctrl+R  accept/resume (if empty); Home/search",
        "Ctrl+U / Ctrl+K  delete to start / end of line",
        "Ctrl+W           delete word left",
        "Ctrl+E           move to end of line",
        "Ctrl+X           cancel running work",
        "F6               pause new steps / continue",
        "Ctrl+T           autonomous / supervised mode",
        "Ctrl+N / O / G    new / switch conversation / project",
        "Ctrl+F / Ctrl+L  search conversation / open plan",
        "Ctrl+P           command palette",
        "F1 / [?]         help",
        "F2 / F3          requirements / current interview question",
        "Ctrl+A           proceed after reviewing the requirements summary",
        "F4 / F5          plan history / actions for a stopped task",
        "F7 / F8          reject proposed plan / configuration",
        "text             always sent as chat; actions use keys or Ctrl+P",
        "Esc              back/clear draft (daemon runs)",
        "Ctrl+C / q       quit (q when not editing text)",
    ]
}

fn help_graph() -> &'static [&'static str] {
    &[
        "arrows           nearest node in that direction",
        "Enter            agent details",
        "/                find agent",
        "d                task diff",
        "u                agent actions: restart orchestrator/worker or unblock",
        "c                comment on selected element",
        "g                toggle grouping (dependencies / state)",
        "1-4              show / hide dependencies, verification, messages, supervision",
        "f                filters: relations, cards by state and provider",
        "z, Space         fold / unfold a card (finished ones start folded)",
        "PgUp/PgDn, wheel scroll the graph (Shift+wheel sideways)",
        SELECTION_HELP,
        "Ctrl+D / Ctrl+U  half page down / up",
        "Ctrl+P           command palette",
        "Ctrl+A / Ctrl+R  accept / resume plan when available",
        "F1 / ? / [?]     help",
        "Tab, Esc         chat",
        "q                quit (work continues in daemon)",
    ]
}

fn help_details() -> &'static [&'static str] {
    &[
        "1–7, Tab/Shift+Tab  tabs; Left/Right select round",
        "i / a            native CLI focus / message composer",
        "Ctrl+]           return from native CLI to antislop",
        "Ctrl+W           switch worker / verifier panel",
        "h / r            JSON history / retry native attachment",
        "↑/↓ PgUp/PgDn    scroll; on Comments select",
        "gg / G           first / last entry; G follows live session",
        "m                task dashboard / terminals (Overview)",
        "Ctrl+D / Ctrl+U  half page down / up",
        "c                comment on agent",
        SELECTION_HELP,
        "r / o / x        resolve / reopen / delete comment",
        "Enter            fold / expand dashboard card (Overview)",
        "d                task diff",
        "Esc              back to graph",
    ]
}

fn help_diff() -> &'static [&'static str] {
    &[
        "↑/↓ PgUp/PgDn    select line (or file)",
        "Ctrl+D / Ctrl+U  half page down / up",
        "←/→              previous / next file",
        "Tab              file list / lines",
        "c                comment on line or file",
        "R                code review of this change",
        "r                refresh",
        "Esc              back",
    ]
}

fn help_comment() -> &'static [&'static str] {
    &[
        "Enter, Ctrl+S    send comment",
        "Alt+Enter, Ctrl+J  newline in comment",
        "↑/↓              line in comment",
        "←/→              character left / right",
        "Alt/Ctrl+←/→     word left / right",
        "Home/End         start / end of line",
        "Alt/Ctrl+Home/End  start / end of comment",
        "Ctrl+A / Ctrl+E  start / end of line",
        "Ctrl+U / Ctrl+K  delete to start / end of line",
        "Ctrl+W, Alt+Backspace  delete word left",
        "Esc              cancel comment (chat draft stays)",
    ]
}

fn help_picker() -> &'static [&'static str] {
    &[
        "text             filter list by typed words",
        "Backspace        delete last query character",
        "↑/↓              select item from list",
        "Enter            confirm selected item",
        "Esc              close list (chat draft stays)",
    ]
}
