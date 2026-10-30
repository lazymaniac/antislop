use super::super::{App, Focus};
use super::{Action, Command, Help, Kind, Picker};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

impl App {
    pub(in crate::lean::tui) fn help_here(&self) -> Help {
        if self.picker.is_some() {
            Help::Picker
        } else if self.editor.is_some() {
            Help::Comment
        } else if self.diff.is_some() {
            Help::Diff
        } else if self.details.is_some() {
            Help::Details
        } else if self.focus == Focus::Graph {
            Help::Graph
        } else {
            Help::Chat
        }
    }

    pub fn show_help(&mut self, help: Help) {
        self.help = Some(help);
        self.help_scroll = 0;
    }

    /// Returns true while help owns the key, including the key that closes it.
    pub(in crate::lean::tui) fn help_key(&mut self, key: KeyEvent) -> bool {
        let max_scroll = self.help_max_scroll();
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.help_scroll = self.help_scroll.saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.help_scroll = (self.help_scroll + 1).min(max_scroll)
            }
            KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(5),
            KeyCode::PageDown => self.help_scroll = (self.help_scroll + 5).min(max_scroll),
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.help_scroll = self.help_scroll.saturating_sub(3)
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.help_scroll = (self.help_scroll + 3).min(max_scroll)
            }
            KeyCode::Home => self.help_scroll = 0,
            KeyCode::End => self.help_scroll = max_scroll,
            KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('?') => {
                self.help = None;
                self.help_scroll = 0;
                return true;
            }
            _ => {}
        }
        true
    }

    /// The picker consumes input while open and applies its selected action on Enter.
    pub(in crate::lean::tui) fn picker_key(&mut self, key: KeyEvent) -> bool {
        let Some(picker) = &mut self.picker else {
            return true;
        };
        match key.code {
            KeyCode::Esc => self.picker = None,
            KeyCode::Up => picker.selected = picker.selected.saturating_sub(1),
            KeyCode::Down => picker.selected += 1,
            KeyCode::Backspace => Self::erase_picker_query(picker),
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                picker.query.push(ch);
                picker.selected = 0;
            }
            KeyCode::Enter => return self.pick_selected(),
            _ => {}
        }
        true
    }

    fn erase_picker_query(picker: &mut Picker) {
        let end = picker
            .query
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(index, _)| index);
        picker.query.truncate(end);
        picker.selected = 0;
    }

    fn pick_selected(&mut self) -> bool {
        let action = self.picker.as_ref().and_then(|picker| {
            picker
                .items(&self.view)
                .into_iter()
                .nth(picker.selected)
                .map(|(_, action)| action)
        });
        self.picker = None;
        self.picker_action(action)
    }

    fn picker_action(&mut self, action: Option<Action>) -> bool {
        match action {
            Some(Action::Run(command)) => self.run(command),
            Some(Action::Select(key)) => {
                self.select_picker_node(key);
                true
            }
            Some(Action::Show(entry)) => {
                self.show_picker_entry(entry);
                true
            }
            Some(Action::SwitchConversation(id)) => {
                self.switch_conversation(id);
                true
            }
            Some(Action::OpenProject(path)) => {
                self.open_project(&path);
                true
            }
            Some(Action::Address(recipient)) => {
                self.address_recipient(&recipient);
                true
            }
            None => true,
        }
    }

    fn select_picker_node(&mut self, key: String) {
        self.close_picker_views();
        self.focus = Focus::Graph;
        self.select_node(key);
    }

    fn show_picker_entry(&mut self, entry: usize) {
        self.close_picker_views();
        self.focus = Focus::Chat;
        self.found = Some(entry);
        self.scroll = usize::MAX;
    }

    fn close_picker_views(&mut self) {
        self.details = None;
        self.diff = None;
        self.plan_view = None;
    }

    fn switch_conversation(&mut self, id: String) {
        if id == self.conversation {
            return;
        }
        let owner = self.projects.iter().position(|project| {
            project
                .conversations
                .iter()
                .any(|conversation| conversation["id"] == id.as_str())
        });
        match owner {
            Some(index) if index != self.active_project => self.switch_project(index, Some(id)),
            _ => self.open(id),
        }
    }

    fn address_recipient(&mut self, recipient: &str) {
        self.close_picker_views();
        self.focus = Focus::Chat;
        let formatted = format!("@{recipient} ");
        let text = self.input.text.trim_end();
        if text.is_empty() {
            self.input.clear();
            self.input.insert_str(&formatted);
        } else if text.ends_with('@') {
            self.input.text.truncate(text.len() - 1);
            self.input.cursor = self.input.text.len();
            self.input.insert_str(&formatted);
        } else {
            self.input.insert_str(&format!(" @{recipient} "));
        }
    }

    /// Run a command selected from the palette.
    pub(in crate::lean::tui) fn run(&mut self, command: Command) -> bool {
        match command {
            Command::Quit => false,
            command => {
                self.run_command(command);
                true
            }
        }
    }

    fn run_command(&mut self, command: Command) {
        let selected = self.selected.clone();
        match command {
            Command::Find => self.picker = Some(Picker::new(Kind::Agents)),
            Command::Search => self.picker = Some(Picker::new(Kind::History)),
            Command::Details => self.open_details(selected),
            Command::ConversationDetails => self.open_details("conversation".into()),
            Command::AddressMessage => self.picker = Some(Picker::new(Kind::Recipients)),
            Command::Diff => self.open_diff(&selected),
            Command::Comment => self.open_comment(selected),
            Command::Filter => self.open_graph_filters(),
            Command::Help => self.show_help(self.help_here()),
            command => self.run_workflow(command),
        }
    }

    fn open_comment(&mut self, target: String) {
        let input = self.get_comment_draft(&target);
        self.editor = Some(super::super::Editor { target, input });
    }
}
