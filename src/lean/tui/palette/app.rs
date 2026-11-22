use super::super::{App, Focus};
use super::{Action, Command, Help, Kind, Picker};
use crate::lean::tui::{InputBuffer, NavigationFocus};
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
        } else if self.navigation.background_focus == Focus::Graph {
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
        if picker.kind == Kind::Conversations && picker.confirm_delete {
            match key.code {
                KeyCode::Char('y') => return self.delete_selected_conversation(),
                KeyCode::Char('n') | KeyCode::Esc => {
                    picker.confirm_delete = false;
                    picker.delete_target = None;
                }
                _ => {}
            }
            return true;
        }
        match key.code {
            KeyCode::Esc => self.picker = None,
            KeyCode::Char('x') if picker.kind == Kind::Conversations => {
                picker.delete_target =
                    picker
                        .items(&self.view)
                        .get(picker.selected)
                        .and_then(|(_, action)| match action {
                            Action::SwitchConversation { id, project } => picker
                                .conversations
                                .iter()
                                .find(|item| item.id == *id && item.project == *project)
                                .cloned(),
                            _ => None,
                        });
                picker.confirm_delete = picker.delete_target.is_some();
            }
            KeyCode::Up => picker.selected = picker.selected.saturating_sub(1),
            KeyCode::Down => picker.selected += 1,
            KeyCode::PageUp => picker.selected = picker.selected.saturating_sub(8),
            KeyCode::PageDown => picker.selected = picker.selected.saturating_add(8),
            KeyCode::Home => picker.selected = 0,
            KeyCode::End => picker.selected = picker.items(&self.view).len().saturating_sub(1),
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

    fn delete_selected_conversation(&mut self) -> bool {
        let selected = self
            .picker
            .as_ref()
            .and_then(|picker| picker.delete_target.clone());
        let Some(selected) = selected else {
            return true;
        };
        if selected.id == self.conversation {
            self.notice =
                Some("Stop this session and switch to another session before deleting it.".into());
            if let Some(picker) = &mut self.picker {
                picker.confirm_delete = false;
            }
            return true;
        }
        let Some(client) = self
            .projects
            .iter()
            .find(|project| project.path.to_string_lossy() == selected.project)
            .map(|project| project.client.clone())
            .or_else(|| self.projects.is_empty().then(|| self.client.clone()))
        else {
            self.notice = Some(format!(
                "✗ Project connection for {} is unavailable",
                selected.project
            ));
            if let Some(picker) = &mut self.picker {
                picker.confirm_delete = false;
                picker.delete_target = None;
            }
            return true;
        };
        if !self
            .conversation_deletions
            .insert((selected.project.clone(), selected.id.clone()))
        {
            return true;
        }
        if let Some(picker) = &mut self.picker {
            picker.status = Some("Deleting session...".into());
            picker.confirm_delete = false;
            picker.delete_target = Some(selected.clone());
        }
        let updates = self.updates.clone();
        std::thread::spawn(move || {
            let result = client
                .call(&crate::lean::daemon::Request::Session(
                    crate::lean::daemon::SessionRequest::DeleteSession {
                        conversation: selected.id.clone(),
                    },
                ))
                .map_err(|error| error.to_string());
            let _ = updates.send(crate::lean::tui::Update::ConversationDeleted {
                project: selected.project,
                id: selected.id,
                result,
            });
        });
        true
    }

    pub(in crate::lean::tui) fn apply_conversation_deleted(
        &mut self,
        project: String,
        id: String,
        result: Result<serde_json::Value, String>,
    ) {
        self.conversation_deletions
            .remove(&(project.clone(), id.clone()));
        let selected = super::ConversationItem {
            project,
            id,
            title: String::new(),
        };
        if let Some(picker) = &mut self.picker
            && picker.kind == Kind::Conversations
            && picker.delete_target.as_ref().is_some_and(|target| {
                target.id == selected.id && target.project == selected.project
            })
        {
            picker.confirm_delete = false;
            picker.delete_target = None;
            picker.status = result.as_ref().err().cloned();
        }
        match result {
            Ok(_) => {
                if let Some(picker) = &mut self.picker
                    && picker.kind == Kind::Conversations
                {
                    let items = picker
                        .conversations
                        .iter()
                        .filter(|item| item.id != selected.id || item.project != selected.project)
                        .cloned()
                        .collect();
                    picker.set_conversations(items, &self.view);
                }
                self.conversation_states.remove(&selected.id);
                self.conversation_drafts.remove(&selected.id);
                self.interview_forms.remove(&selected.id);
                self.conversation_projects.remove(&selected.id);
                self.deleted_conversations.insert((
                    std::path::PathBuf::from(&selected.project),
                    selected.id.clone(),
                ));
                if let Some(project) = self
                    .projects
                    .iter_mut()
                    .find(|project| project.path.to_string_lossy() == selected.project)
                {
                    project
                        .conversations
                        .retain(|item| item["id"] != selected.id);
                }
                self.notice = Some(format!(
                    "Session {} ({}) and its history permanently deleted.",
                    selected.id, selected.project
                ));
            }
            Err(error) => {
                self.notice = Some(format!(
                    "Could not delete session {} ({}): {error}",
                    selected.id, selected.project
                ));
            }
        }
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
        let is_slash = self
            .picker
            .as_ref()
            .is_some_and(|p| p.kind == Kind::SlashCommands);
        let fallback_query = self.picker.as_ref().map(|p| p.query.clone());
        let action = self.picker.as_ref().and_then(|picker| {
            picker
                .items(&self.view)
                .into_iter()
                .nth(picker.selected)
                .map(|(_, action)| action)
        });
        self.picker = None;
        if action.is_none()
            && is_slash
            && let Some(query) = fallback_query
        {
            self.input.insert_str(&format!("/{query}"));
            return true;
        }
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
            Some(Action::SwitchConversation { id, project }) => {
                self.switch_conversation(id, Some(&project));
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
        self.set_focus(Focus::Graph);
        self.select_work_stage(key);
    }

    fn show_picker_entry(&mut self, entry: usize) {
        self.close_picker_views();
        self.open_details("conversation".into());
        self.navigation.update_presentation(|presentation| {
            presentation.tab = crate::lean::tui::WorkbenchTab::Conversation
        });
        if let Some(details) = &mut self.details {
            details.prepare_workbench_details(None, crate::lean::tui::WorkbenchTab::Conversation);
        }
        self.set_focus(Focus::Chat);
        self.found = Some(entry);
        self.scroll = usize::MAX;
    }

    fn close_picker_views(&mut self) {
        self.remember_workbench();
        self.navigation.screen = crate::lean::tui::Screen::Graph;
        self.details = None;
        self.diff = None;
        self.plan_view = None;
    }

    fn switch_conversation(&mut self, id: String, project_path: Option<&str>) {
        if id == self.conversation {
            return;
        }
        let selected_path = project_path
            .map(std::path::PathBuf::from)
            .or_else(|| self.conversation_projects.get(&id).cloned());
        if self.projects.is_empty() {
            self.open(id);
            return;
        }
        let Some(selected_path) = selected_path else {
            self.notice = Some("✗ Session project is unknown; session was not opened".into());
            return;
        };
        let Some(index) = self
            .projects
            .iter()
            .position(|project| project.path == selected_path)
        else {
            self.notice = Some(format!(
                "✗ Project connection for {} is unavailable; session was not opened",
                selected_path.display()
            ));
            return;
        };
        self.conversation_projects.insert(id.clone(), selected_path);
        if index != self.active_project {
            self.switch_project(index, Some(id));
        } else {
            self.open(id);
        }
    }

    fn address_recipient(&mut self, recipient: &str) {
        if self.details.as_ref().is_some_and(|details| details.editing) {
            self.open_details(recipient.to_owned());
            if let Some(details) = &mut self.details {
                details.editing = true;
            }
            self.navigation.focus = NavigationFocus::Composer;
            return;
        }
        if self.navigation.focus == NavigationFocus::Composer {
            let text = self
                .input
                .text
                .strip_prefix('@')
                .and_then(|rest| rest.split_once(char::is_whitespace).map(|(_, text)| text))
                .unwrap_or(&self.input.text)
                .to_owned();
            self.input = InputBuffer::with_text(if recipient == "orchestrator" {
                text
            } else {
                format!("@{recipient} {text}")
            });
            return;
        }
        self.close_picker_views();
        self.set_focus(Focus::Chat);
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
