//! Ctrl+P palette of commands, `/` search for an agent of the graph and
//! Ctrl+F search of the conversation: type to narrow, arrows to choose,
//! Enter to run, Esc to close. Also the `?` help of the active view.

use super::graph::Graph;
use super::view::{View, agent_name, dim, truncate};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Clear, Paragraph},
};
use unicode_width::UnicodeWidthStr;

const SELECTION_HELP: &str = "Ctrl+Y           copy selection (drag / double click selects)";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Config,
    Accept,
    Reject,
    Plan,
    Resume,
    Cancel,
    Pause,
    Continue,
    Autonomous,
    Supervised,
    NewConversation,
    SwitchConversation,
    Find,
    Search,
    Details,
    Diff,
    Comment,
    Filter,
    Help,
    Quit,
    AddressMessage,
    ConversationDetails,
    Projects,
}

const COMMANDS: [(Command, &str); 23] = [
    (Command::Config, "Edit configuration live · /config"),
    (Command::Accept, "Accept proposed plan · Ctrl+A"),
    (Command::Reject, "Reject proposed plan · /reject"),
    (Command::Plan, "Open plan document viewer · Ctrl+L / /plan"),
    (Command::Resume, "Resume plan · Ctrl+R"),
    (Command::Cancel, "Cancel running work · Ctrl+X"),
    (
        Command::Pause,
        "Pause new steps (current ones finish) · /pause",
    ),
    (Command::Continue, "Continue paused work · /continue"),
    (
        Command::Autonomous,
        "Autonomous mode: plans and rounds start automatically · /mode autonomous",
    ),
    (
        Command::Supervised,
        "Supervised mode: plans wait for approval · /mode supervised",
    ),
    (Command::NewConversation, "New conversation · Ctrl+N"),
    (Command::SwitchConversation, "Switch conversation · Ctrl+O"),
    (
        Command::Projects,
        "Open or switch project · Ctrl+G / /project",
    ),
    (Command::Find, "Find agent · /"),
    (Command::Search, "Search conversation · Ctrl+F"),
    (Command::Details, "Selected agent details · Enter"),
    (
        Command::ConversationDetails,
        "Conversation details (inspector) · Ctrl+I",
    ),
    (Command::AddressMessage, "Address message · @"),
    (Command::Diff, "Selected agent task diff · d"),
    (Command::Comment, "Comment on selected element · c"),
    (Command::Filter, "Next graph relation filter · f"),
    (Command::Help, "Help · ?"),
    (Command::Quit, "Detach (work continues) · q"),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectItem {
    pub path: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationItem {
    pub id: String,
    pub project: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Run(Command),
    /// Select this graph node.
    Select(String),
    /// Show this entry of the conversation.
    Show(usize),
    /// Switch to another conversation.
    SwitchConversation(String),
    /// Address a message to a recipient.
    Address(String),
    /// Open this project (a directory) or switch to it.
    OpenProject(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Commands,
    Agents,
    History,
    Conversations,
    Recipients,
    Projects,
}

#[derive(Debug)]
pub struct Picker {
    pub kind: Kind,
    pub query: String,
    pub selected: usize,
    pub conversations: Vec<ConversationItem>,
    pub projects: Vec<ProjectItem>,
}

impl Picker {
    pub fn new(kind: Kind) -> Self {
        Picker {
            kind,
            query: String::new(),
            selected: 0,
            conversations: Vec::new(),
            projects: Vec::new(),
        }
    }

    /// The entries matching what was typed, each word anywhere in the label.
    pub fn items(&self, view: &View) -> Vec<(String, Action)> {
        let all: Vec<(String, Action)> = match self.kind {
            Kind::Commands => COMMANDS
                .iter()
                .map(|(command, label)| ((*label).to_owned(), Action::Run(*command)))
                .collect(),
            Kind::Agents => Graph::new(view)
                .nodes
                .iter()
                .filter(|node| node.visible)
                .map(|node| {
                    let task = node
                        .key
                        .split_once(':')
                        .and_then(|(_, id)| view.tasks.iter().find(|task| task.id == id));
                    let label = match task {
                        Some(task) => {
                            format!("{} task · {} · worker verifier", task.id, task.title)
                        }
                        None => agent_name(&node.key),
                    };
                    (label, Action::Select(node.key.clone()))
                })
                .collect(),
            // Newest first.
            Kind::History => view
                .transcript
                .iter()
                .enumerate()
                .rev()
                .filter(|(_, entry)| !entry.text.trim().is_empty())
                .map(|(index, entry)| (entry.text.trim().replace('\n', " "), Action::Show(index)))
                .collect(),
            Kind::Conversations => self
                .conversations
                .iter()
                .map(|c| (c.title.clone(), Action::SwitchConversation(c.id.clone())))
                .collect(),
            Kind::Projects => self
                .projects
                .iter()
                .map(|p| (p.title.clone(), Action::OpenProject(p.path.clone())))
                .collect(),
            Kind::Recipients => {
                let mut list = vec![
                    (
                        "orchestrator · default recipient".to_owned(),
                        Action::Address("orchestrator".into()),
                    ),
                    (
                        "all · broadcast to all active tasks".to_owned(),
                        Action::Address("all".into()),
                    ),
                    (
                        "plan-verifier · plan verifier".to_owned(),
                        Action::Address("plan-verifier".into()),
                    ),
                    (
                        "auditor · final auditor".to_owned(),
                        Action::Address("auditor".into()),
                    ),
                ];
                for task in &view.tasks {
                    list.push((
                        format!(
                            "worker:{} · {} ({})",
                            task.id,
                            task.title,
                            if task.writes {
                                "modifies files"
                            } else {
                                "read-only"
                            }
                        ),
                        Action::Address(format!("worker:{}", task.id)),
                    ));
                    list.push((
                        format!("verifier:{} · task verifier {}", task.id, task.title),
                        Action::Address(format!("verifier:{}", task.id)),
                    ));
                }
                list
            }
        };
        let words: Vec<String> = self
            .query
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        let mut matching: Vec<(String, Action)> = all
            .into_iter()
            .filter(|(label, _)| {
                let label = label.to_lowercase();
                words.iter().all(|word| label.contains(word.as_str()))
            })
            .collect();
        // A typed path can be opened even when it is in no list.
        let typed = self.query.trim();
        if self.kind == Kind::Projects && (typed.starts_with(['/', '~', '.'])) {
            matching.push((
                format!("Open {typed}"),
                Action::OpenProject(typed.to_owned()),
            ));
        }
        matching
    }

    pub fn draw(&mut self, frame: &mut Frame, view: &View) {
        self.draw_with_cursor(frame, view, true);
    }

    pub(super) fn draw_with_cursor(&mut self, frame: &mut Frame, view: &View, show_cursor: bool) {
        let items = self.items(view);
        self.selected = self.selected.min(items.len().saturating_sub(1));
        let area = frame.area();
        if area.width == 0 || area.height == 0 {
            return;
        }
        let width = if area.width >= 8 {
            area.width.saturating_sub(4).min(64)
        } else {
            area.width
        };
        let rows = items.len().clamp(1, 12) as u16;
        let height = (rows + 3).min(area.height).max(1);
        let box_area = Rect::new(
            area.x + (area.width.saturating_sub(width)) / 2,
            area.y + area.height.saturating_sub(height) / 3,
            width,
            height,
        );
        let inner = usize::from(width.saturating_sub(2));
        let visible_rows = usize::from(height.saturating_sub(3)).max(1);
        let start = if self.selected >= visible_rows {
            self.selected + 1 - visible_rows
        } else {
            0
        };
        let mut lines = vec![Line::raw(truncate(&format!("> {}", self.query), inner))];
        if items.is_empty() {
            let empty_msg = if self.query.is_empty() {
                match self.kind {
                    Kind::Commands => "No commands available.",
                    Kind::Agents => "No agents in graph.",
                    Kind::History => "No entries in conversation history.",
                    Kind::Conversations => "No other conversations in this project.",
                    Kind::Recipients => "No recipients matching query.",
                    Kind::Projects => "No other projects yet: type a directory path (/…, ~/…).",
                }
            } else {
                "Nothing matches the query."
            };
            lines.push(Line::styled(truncate(empty_msg, inner), dim()));
        }
        for (index, (label, _)) in items.iter().enumerate().skip(start).take(visible_rows) {
            let style = if index == self.selected {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            lines.push(Line::styled(truncate(label, inner), style));
        }
        let title = match self.kind {
            Kind::Commands if width >= 42 => " commands · Enter run · Esc close ",
            Kind::Commands if width >= 20 => " commands · Esc ",
            Kind::Commands => " commands ",
            Kind::Agents if width >= 45 => " find agent · Enter select · Esc close ",
            Kind::Agents if width >= 20 => " agents · Esc ",
            Kind::Agents => " agents ",
            Kind::History if width >= 45 => " search conversation · Enter show · Esc close ",
            Kind::History if width >= 20 => " search · Esc ",
            Kind::History => " search ",
            Kind::Conversations if width >= 45 => " switch conversation · Enter open · Esc close ",
            Kind::Conversations if width >= 20 => " conversations · Esc ",
            Kind::Conversations => " conversations ",
            Kind::Recipients if width >= 45 => " message recipient · Enter select · Esc close ",
            Kind::Recipients if width >= 20 => " recipients · Esc ",
            Kind::Recipients => " recipients ",
            Kind::Projects if width >= 50 => " projects · Enter open · type a path · Esc close ",
            Kind::Projects if width >= 20 => " projects · Esc ",
            Kind::Projects => " projects ",
        };
        frame.render_widget(Clear, box_area);
        frame.render_widget(
            Paragraph::new(lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(title),
            ),
            box_area,
        );
        if show_cursor && box_area.width >= 3 && box_area.height >= 2 {
            let query_display_width = self.query.width();
            let prefix_width = 2;
            let max_col = inner.saturating_sub(1);
            let col = (prefix_width + query_display_width).min(max_col) as u16;
            frame.set_cursor_position((box_area.x + 1 + col, box_area.y + 1));
        }
    }
}

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
            Help::Chat => &[
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
                "/pause, /continue  pause new steps / continue",
                "/mode autonomous   autonomous mode for next plans",
                "/mode supervised   supervised mode (Ctrl+T toggles)",
                "Ctrl+N / O / G    new / switch conversation / project",
                "Ctrl+F / Ctrl+L  search conversation / open plan",
                "Ctrl+P           command palette",
                "Esc              back/clear draft (daemon runs)",
                "Ctrl+C / q       quit (q when not editing text)",
            ],
            Help::Graph => &[
                "arrows           nearest node in that direction",
                "Enter            agent details",
                "/                find agent",
                "d                task diff",
                "u                unblock the selected stopped step (or the first one)",
                "c                comment on selected element",
                "z, Space         collapse / expand task",
                "g                toggle grouping (dependencies / state)",
                "f                relation filter: all, dependencies, verification, messages, supervision",
                "PgUp/PgDn, wheel scroll full task / verification text",
                SELECTION_HELP,
                "Ctrl+D / Ctrl+U  half page down / up",
                "Ctrl+P           command palette",
                "Tab, Esc         chat",
                "q                quit (work continues in daemon)",
            ],
            Help::Details => &[
                "1–5, ←/→, Tab    tabs (task: Overview, Messages, Result, Checks, Comments)",
                "↑/↓ PgUp/PgDn    scroll; on Comments select",
                "Ctrl+D / Ctrl+U  half page down / up",
                "c                comment on agent",
                SELECTION_HELP,
                "r / o / x        resolve / reopen / delete comment",
                "Enter            fold / unfold prompts (Overview)",
                "d                task diff",
                "Esc              back to graph",
            ],
            Help::Diff => &[
                "↑/↓ PgUp/PgDn    select line (or file)",
                "Ctrl+D / Ctrl+U  half page down / up",
                "←/→              previous / next file",
                "Tab              file list / lines",
                "c                comment on line or file",
                "R                code review of this change",
                "r                refresh",
                "Esc              back",
            ],
            Help::Comment => &[
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
            ],
            Help::Picker => &[
                "text             filter list by typed words",
                "Backspace        delete last query character",
                "↑/↓              select item from list",
                "Enter            confirm selected item",
                "Esc              close list (chat draft stays)",
            ],
        }
    }

    pub fn lines_for_width(self, inner: usize) -> Vec<String> {
        let lines = self.lines();
        let mut out = Vec::new();
        for &line in lines {
            if inner < 60 {
                let compact = compact_help_line(line);
                if compact.chars().count() <= inner {
                    out.push(compact);
                } else if let Some((key, desc)) = compact.split_once(": ") {
                    let prefix = format!("{key}: ");
                    let indent = "   ";
                    let mut cur = prefix;
                    for word in desc.split_whitespace() {
                        let cur_len = cur.chars().count();
                        let word_len = word.chars().count();
                        let needed = if cur.ends_with(": ") || cur.ends_with("   ") {
                            word_len
                        } else {
                            word_len + 1
                        };
                        if cur_len + needed > inner && cur_len > 3 {
                            out.push(cur);
                            cur = format!("{indent}{word}");
                        } else {
                            if !cur.ends_with(": ") && !cur.ends_with("   ") {
                                cur.push(' ');
                            }
                            cur.push_str(word);
                        }
                    }
                    if !cur.is_empty() {
                        out.push(cur);
                    }
                } else {
                    out.push(truncate(&compact, inner));
                }
            } else if line.chars().count() <= inner {
                out.push(line.to_string());
            } else {
                out.push(truncate(line, inner));
            }
        }
        out
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
        let width = if area.width >= 8 {
            area.width.saturating_sub(2).min(84)
        } else {
            area.width
        };
        let inner = usize::from(width.saturating_sub(2));
        let lines = self.lines_for_width(inner);
        let height = (lines.len() as u16 + 2).min(area.height).max(1);
        let box_area = Rect::new(
            area.x + (area.width.saturating_sub(width)) / 2,
            area.y + area.height.saturating_sub(height) / 3,
            width,
            height,
        );
        let visible_lines = usize::from(height.saturating_sub(2));
        let max_scroll = lines.len().saturating_sub(visible_lines);
        let scroll = scroll.min(max_scroll);
        let title = if max_scroll > 0 {
            if width >= 44 {
                " help · ↑/↓ scroll · Esc closes "
            } else if width >= 26 {
                " help · ↑/↓ · Esc "
            } else {
                " help "
            }
        } else if width >= 36 {
            " help · Esc closes "
        } else if width >= 14 {
            " help · Esc "
        } else {
            " help "
        };
        let shown_lines: Vec<Line> = lines
            .iter()
            .skip(scroll)
            .take(visible_lines)
            .map(|line| Line::raw(line.clone()))
            .collect();
        frame.render_widget(Clear, box_area);
        frame.render_widget(
            Paragraph::new(shown_lines).block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::Cyan))
                    .title(title),
            ),
            box_area,
        );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_narrows_commands_by_every_word() {
        let view = View::default();
        let mut picker = Picker::new(Kind::Commands);
        assert_eq!(picker.items(&view).len(), COMMANDS.len());
        picker.query = "plan res".into();
        let items = picker.items(&view);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].1, Action::Run(Command::Resume));
    }

    #[test]
    fn the_conversation_is_searched_newest_first() {
        let mut view = View::default();
        for (seq, text) in [(1, "plan importu"), (2, "inne"), (3, "Import gotowy")] {
            view.apply(&serde_json::json!({"seq": seq, "kind": "message",
                "payload": {"author": "orchestrator", "text": text}}));
        }
        let mut picker = Picker::new(Kind::History);
        picker.query = "import".into();
        let shown: Vec<Action> = picker
            .items(&view)
            .into_iter()
            .map(|(_, action)| action)
            .collect();
        assert_eq!(shown.len(), 2);
        let Action::Show(newest) = shown[0] else {
            panic!("{shown:?}")
        };
        assert!(view.transcript[newest].text.contains("Import gotowy"));
    }

    #[test]
    fn agents_are_found_by_role_or_task_title() {
        let mut view = View::default();
        view.apply(
            &serde_json::json!({"seq": 1, "kind": "plan_proposed", "payload": {
            "plan": "p",
            "tasks": [{"id": "t1", "title": "Parser"}, {"id": "t2", "title": "Docs"}]}}),
        );
        let mut picker = Picker::new(Kind::Agents);
        picker.query = "pars veri".into();
        let items = picker.items(&view);
        assert_eq!(
            items,
            [(
                "t1 task · Parser · worker verifier".to_owned(),
                Action::Select("worker:t1".into())
            )]
        );
    }

    #[test]
    fn picker_cursor_bounded_and_display_width_aware() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let view = View::default();
        let mut picker = Picker::new(Kind::Commands);

        // 1. Standard ASCII query
        picker.query = "pla".into();
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                picker.draw(frame, &view);
            })
            .unwrap();
        // Box width is min(80-4, 64) = 64. Left x is (80-64)/2 = 8.
        // Inner cursor: prefix "> " (2 cols) + "pla" (3 cols) = 5 cols from box_area.x + 1.
        // So cursor_x should be 8 + 1 + 5 = 14.
        let (cursor_x, cursor_y) = terminal.get_cursor_position().unwrap().into();
        assert_eq!(cursor_x, 14);
        assert_eq!(cursor_y, 6);

        // 2. Double-width CJK character takes 2 visual columns
        picker.query = "漢".into();
        terminal
            .draw(|frame| {
                picker.draw(frame, &view);
            })
            .unwrap();
        let (cursor_x, _) = terminal.get_cursor_position().unwrap().into();
        // "> " (2) + "漢" (2) = 4 cols -> 8 + 1 + 4 = 13
        assert_eq!(cursor_x, 13);

        // 3. Overflowing long query: cursor must be bounded within box interior!
        picker.query = "a".repeat(200);
        terminal
            .draw(|frame| {
                picker.draw(frame, &view);
            })
            .unwrap();
        let (cursor_x, _) = terminal.get_cursor_position().unwrap().into();
        // Inner width is 64 - 2 = 62. Max col is 61.
        // Bounded cursor_x should be 8 + 1 + 61 = 70.
        // Box right is 8 + 64 = 72, interior right is 71. Cursor <= 70 is safely inside border!
        assert_eq!(cursor_x, 70);
        assert!(cursor_x < 8 + 64 - 1);
    }

    #[test]
    fn picker_and_help_render_deterministically_on_narrow_and_tiny_terminals() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let view = View::default();
        let mut picker = Picker::new(Kind::Commands);

        // Narrow terminal: 40x12
        let backend = TestBackend::new(40, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                picker.draw(frame, &view);
            })
            .unwrap();
        let buffer = terminal.backend().buffer().clone();
        assert!(buffer.area.width == 40 && buffer.area.height == 12);

        // Tiny terminal: 10x4
        let backend = TestBackend::new(10, 4);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                picker.draw(frame, &view);
            })
            .unwrap();

        // Extreme tiny terminal: 2x2 (should not panic)
        let backend = TestBackend::new(2, 2);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                picker.draw(frame, &view);
                Help::Chat.draw(frame);
            })
            .unwrap();
    }

    #[test]
    fn picker_empty_state_and_narrow_title() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let view = View::default();
        let mut picker = Picker::new(Kind::History);
        assert!(picker.items(&view).is_empty());

        let backend = TestBackend::new(60, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                picker.draw(frame, &view);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let rendered: String = buffer.content.iter().map(|c| c.symbol()).collect();
        // Empty state for history
        assert!(
            rendered.contains("No entries in conversation history")
                || rendered.contains("No entries")
        );
    }

    #[test]
    fn picker_at_60x12_keeps_selected_item_visible_when_items_len_ge_10() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let view = View::default();
        let mut picker = Picker::new(Kind::Commands);
        let items = picker.items(&view);
        assert!(items.len() >= 10, "Commands should have >= 10 items");

        // Select the last item (e.g. index 11)
        picker.selected = items.len() - 1;

        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                picker.draw(frame, &view);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        // Verify that the selected item is rendered with REVERSED modifier
        let mut found_reversed_selected = false;
        let last_item_label = &items[picker.selected].0;
        let last_word = last_item_label.split_whitespace().next().unwrap();

        for y in 0..12 {
            let row_chars: String = (0..60).map(|x| buffer[(x, y)].symbol()).collect();
            if row_chars.contains(last_word) {
                for x in 0..60 {
                    let cell = &buffer[(x, y)];
                    if cell.modifier.contains(Modifier::REVERSED) {
                        found_reversed_selected = true;
                        break;
                    }
                }
            }
        }
        assert!(
            found_reversed_selected,
            "Selected item at index {} must be visible with REVERSED modifier at 60x12",
            picker.selected
        );
    }

    #[test]
    fn help_at_60x12_renders_all_controls_and_documents_shortcuts() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();

        // 1. Initial page (scroll = 0): verify first set of shortcuts and full descriptions
        terminal
            .draw(|frame| {
                Help::Chat.draw(frame);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let rendered: String = buffer.content.iter().map(|c| c.symbol()).collect();

        // Check shortcuts and their complete descriptions
        assert!(rendered.contains("send message to orchestrator"));
        assert!(rendered.contains("newline in draft"));
        assert!(rendered.contains("line in draft"));
        assert!(rendered.contains("scroll conversation"));
        assert!(rendered.contains("accept/resume"));
        assert!(rendered.contains("delete to start / end"));

        // 2. Scrolled page: verify subsequent shortcuts and descriptions
        terminal
            .draw(|frame| {
                Help::Chat.draw_scroll(frame, usize::MAX);
            })
            .unwrap();

        let buffer2 = terminal.backend().buffer();
        let rendered2: String = buffer2.content.iter().map(|c| c.symbol()).collect();

        assert!(rendered2.contains("cancel running work"));
        assert!(rendered2.contains("pause new steps / continue"));
        assert!(rendered2.contains("/mode autonomous"));
        assert!(rendered2.contains("/mode supervised"));
        assert!(rendered2.contains("search conversation"));
        assert!(rendered2.contains("command palette"));
        assert!(rendered2.contains("back/clear draft"));

        // 3. Graph help at 60x12: verify long filter relation description is fully accessible without clipping
        terminal
            .draw(|frame| {
                Help::Graph.draw_scroll(frame, 2);
            })
            .unwrap();

        let buffer_graph = terminal.backend().buffer();
        let rendered_graph: String = buffer_graph.content.iter().map(|c| c.symbol()).collect();
        assert!(rendered_graph.contains("relation filter"));
        assert!(rendered_graph.contains("all"));
        assert!(rendered_graph.contains("dependencies"));
        assert!(rendered_graph.contains("verification"));
        assert!(rendered_graph.contains("messages"));
        assert!(rendered_graph.contains("supervision"));
    }
}
