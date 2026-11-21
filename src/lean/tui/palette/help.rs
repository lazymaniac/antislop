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
            .chain(
                matches!(self, Help::Chat | Help::Graph | Help::Details | Help::Diff)
                    .then_some(help_global())
                    .into_iter()
                    .flatten(),
            )
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
    let theme = crate::lean::tui::theme::Theme::from_env();
    let height = (lines.len() as u16 + 2).min(area.height).max(1);
    let box_area = help_box_area(area, width, height);
    let visible = usize::from(height.saturating_sub(2));
    let max_scroll = lines.len().saturating_sub(visible);
    let shown = lines
        .iter()
        .skip(scroll.min(max_scroll))
        .take(visible)
        .map(|line| Line::raw(theme.chrome(line)))
        .collect::<Vec<_>>();
    let title = help_title(width, max_scroll > 0);
    frame.render_widget(Clear, box_area);
    frame.render_widget(
        Paragraph::new(shown).block(
            Block::default()
                .borders(Borders::ALL)
                .border_set(theme.borders())
                .border_style(Style::default())
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
        return aligned_help_line(line, inner);
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

/// A wide box keeps keys in their column and wraps a long description
/// under its own start.
fn aligned_help_line(line: &str, inner: usize) -> Vec<String> {
    let column = line.find("  ").and_then(|gap| {
        line[gap..]
            .find(|ch: char| ch != ' ')
            .map(|desc| gap + desc)
    });
    let Some(column) = column.filter(|_| line.chars().count() > inner) else {
        return vec![truncate(line, inner)];
    };
    let indent = line[..column].chars().count();
    if indent + 12 > inner {
        return vec![truncate(line, inner)];
    }
    let mut out = Vec::new();
    let mut current = line[..column].to_string();
    for word in line[column..].split_whitespace() {
        let used = current.chars().count();
        let words = used > indent;
        if words && used + 1 + word.chars().count() > inner {
            out.push(std::mem::replace(&mut current, " ".repeat(indent)));
        } else if words {
            current.push(' ');
        }
        current.push_str(word);
    }
    out.push(current);
    out.into_iter().map(|row| truncate(&row, inner)).collect()
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
        "Enter            send to the displayed recipient",
        "Ctrl+V           attach an image from the system clipboard",
        "Alt+Backspace    remove the latest pending image",
        "@t2 text         message worker t2 (copied in chat)",
        "Alt+Enter, Ctrl+J  newline in draft",
        "↑/↓              line in draft; sent history at edges",
        "PgUp/PgDn        scroll conversation",
        SELECTION_HELP,
        "Ctrl+D / Ctrl+U  half page down / up (empty draft)",
        "Ctrl+R           change recipient (including broadcast)",
        "Ctrl+A / Ctrl+E  line start / end",
        "Ctrl+U / Ctrl+K  editors: delete to start / end of line",
        "Ctrl+W           delete word left",
        "Ctrl+E           move to end of line",
        "Ctrl+X           cancel running work",
        "Space            pause / continue outside editors",
        "Ctrl+N / O / G    new / switch conversation / project",
        "Ctrl+F / Ctrl+L  search conversation / open plan",
        "Ctrl+T           session Workbench",
        "Ctrl+P           command palette",
        "F1 / [?]         help",
        "d                decisions outside editors",
        "Planning starts after you confirm the scope",
        "h / u            plan history / recovery outside editors",
        "F7 / F8          reject proposed plan / configuration",
        "/                slash commands (/direct, /rigor, /edit, /accept)",
        "text             always sent as chat; actions use keys or Ctrl+P",
        "Esc              close composer and retain draft",
        "Ctrl+C           exit outside native input",
    ]
}

fn help_graph() -> &'static [&'static str] {
    &[
        "arrows           select a work card",
        "Enter            open selected or remembered stage in Workbench",
        "Ctrl+B           show / hide the graph inspector",
        "/                find agent",
        "d                decisions inbox",
        "Ctrl+K           give direction to the orchestrator",
        "Space            pause / continue",
        "o / v            selected stage results / changes",
        "u                agent actions: restart orchestrator/worker or unblock",
        "c                comment on selected element",
        "g                toggle grouping (dependencies / state)",
        "1-4              show / hide dependencies, review, messages, supervision",
        "f                filters: relations, cards by state and provider",
        "z                fold / unfold a work card",
        "PgUp/PgDn, wheel scroll the graph (Shift+wheel sideways)",
        SELECTION_HELP,
        "Ctrl+D / Ctrl+U  half page down / up",
        "Ctrl+P           command palette",
        "Ctrl+T           session Workbench",
        "Ctrl+R           resume plan when available",
        "F1 / ? / [?]     help",
        "Tab/Shift+Tab    select a stage in the inspector",
        "q                quit (work continues in daemon)",
        "e / a / S        flow editor: edit task / add dependent / split",
        "h / D / l        plan history / retire task / link dependency",
        "s / R            steer agent / workflow rigor",
    ]
}

fn help_details() -> &'static [&'static str] {
    &[
        "Ctrl+Left/Right  previous / next Workbench tab (available for this role)",
        "Tab/Shift+Tab    sidebar / content / terminal focus",
        "Ctrl+B           show / hide the task and agent list",
        "z                zoom focused panel / restore split",
        "Alt+Up/Down      resize terminal in split view",
        "p                pin / unpin terminal owner",
        "Alt+Left/Right   previous / next result document",
        "Workbench       , / . select round; . on newest follows live work",
        "F5 / u          task recovery actions; 1–3 session completion actions",
        "Space / Ctrl+X  pause/continue or cancel the current session",
        "Ctrl+P          command palette",
        "t / Ctrl+K       native CLI input / selected stage composer",
        "Ctrl+R           composer: change recipient, including broadcast",
        "Ctrl+T           composer: queue / steer / interrupt the running turn",
        "Ctrl+]           leave native PTY input; return to Workbench navigation",
        "PTY: Ctrl+]      return to navigation; every other key goes to CLI",
        "Ctrl+W           switch worker / reviewer target",
        "h / r            plan history / retry native attachment",
        "↑/↓ PgUp/PgDn    scroll; on Comments select",
        "gg / G           first / last entry; G follows live session",
        "m                task dashboard / terminals (Overview)",
        "Ctrl+D / Ctrl+U  half page down / up",
        "c                comment on agent",
        SELECTION_HELP,
        "r / o / x        resolve / reopen / delete comment",
        "Enter            fold / expand dashboard card (Overview)",
        "d                decisions inbox",
        "Esc              back to graph",
    ]
}

fn help_diff() -> &'static [&'static str] {
    &[
        "↑/↓ PgUp/PgDn    select line (or file)",
        "Ctrl+D / Ctrl+U  half page down / up",
        "[/] or ←/→       previous / next file",
        "Workbench: Tab   sidebar / content / terminal (Shift+Tab reverses)",
        "Ctrl+Left/Right  previous / next Workbench tab",
        "Ctrl+B / z       Workbench list / zoom focused panel",
        "Alt+Up/Down / p  resize split / pin terminal owner",
        ", / .            Workbench round; . on newest follows live",
        "Terminal: t      focus and enter native PTY; Ctrl+] returns",
        "f                hide / show the file list",
        "c                comment on line or file",
        "R                code review of this change",
        "r                refresh",
        "Esc              Workbench Summary / back from graph inspector",
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
        "Ctrl+U / Ctrl+K  editors: delete to start / end of line",
        "Ctrl+W, Alt+Backspace  delete word left",
        "Esc              cancel comment (chat draft stays)",
    ]
}

fn help_picker() -> &'static [&'static str] {
    &[
        "text             filter list by typed words",
        "Backspace        delete last query character",
        "↑/↓              select item from list",
        "PgUp/PgDn        move through a long list",
        "Home/End         first / last item",
        "Enter            confirm selected item",
        "Esc              close list (chat draft stays)",
        "Configuration editor",
        "Ctrl+Left/Right  previous / next section",
        "Up / Down        previous / next field",
        "Tab              move between fields",
        "Enter            cycle a choice",
        "Ctrl+N / Ctrl+D  add / delete provider, profile or route",
        "Ctrl+S           validate and apply the draft",
        "Ctrl+R           refresh without losing a dirty draft",
        "Ctrl+P           open command palette",
        "Esc              close configuration",
    ]
}

fn help_global() -> &'static [&'static str] {
    &[
        "Global navigation (outside editors, forms and native PTY)",
        "F1 / F2          context help / decision inbox",
        "Ctrl+P           command palette",
        "Ctrl+T / Ctrl+I  session Workbench (Ctrl+I where distinct from Tab)",
        "Ctrl+F / Ctrl+K  conversation search / session composer",
        "Global workflow actions (also available through Ctrl+P):",
        "Ctrl+N / Ctrl+O  new / switch conversation",
        "Ctrl+G / Ctrl+L  project picker / plan",
        "Ctrl+M / Ctrl+Enter  workflow rigor (Ctrl+M where distinct from Enter)",
        "F3 / F4 / F5     interview question / plan history / task actions",
        "F6 / F7 / F8     pause-continue / reject plan / configuration",
        "Ctrl+X           cancel running work",
        "Ctrl+Y           copy selected application text",
        "Ctrl+C           quit frontend; daemon work continues",
        "Editors keep their text-editing keys; Esc leaves the editor first",
        "Native PTY owns all keys except Ctrl+]; leave it before shortcuts",
    ]
}
