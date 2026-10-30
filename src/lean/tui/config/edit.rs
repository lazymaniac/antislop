use super::model::kind_names;
use super::*;
impl ConfigEditor {
    pub fn paste(&mut self, text: &str) {
        if self.pending || self.adding.is_some() {
            return;
        }
        self.fields[self.selected]
            .input
            .insert_str(&sanitize_single_line_paste(text));
        self.fields[self.selected].error = None;
        self.status = "Unsaved changes".into();
    }

    fn show_addition(&mut self, section: String, choices: Vec<String>, index: usize) {
        self.status = format!(
            "Add {}: {}  Tab/Up/Down: choose  Enter: add  Esc: cancel",
            section, choices[index]
        );
        self.adding = Some((section, choices, index));
    }

    fn addition_key(&mut self, key: KeyEvent) -> bool {
        let Some((section, choices, mut index)) = self.adding.take() else {
            return false;
        };
        match key.code {
            KeyCode::Esc => self.status = "Addition cancelled".into(),
            KeyCode::Enter => self.add_kind(section, &choices[index]),
            _ => {
                match key.code {
                    KeyCode::Up | KeyCode::BackTab => {
                        index = (index + choices.len() - 1) % choices.len()
                    }
                    KeyCode::Down | KeyCode::Tab => index = (index + 1) % choices.len(),
                    _ => {}
                }
                self.show_addition(section, choices, index);
            }
        }
        true
    }

    fn first_entry_name(&self, section: &str) -> Option<String> {
        self.fields
            .iter()
            .find(|field| field.path[0] == section)
            .map(|field| field.path[1].clone())
    }

    fn add_kind(&mut self, section: String, name: &str) {
        let value = if section == "providers" {
            serde_json::json!({"binary": name, "args": [], "command": null})
        } else {
            let Some(profile) = self.first_entry_name("profiles") else {
                self.status = "Cannot add a route: no profile is configured.".into();
                return;
            };
            let executor = if name == "orchestration" {
                self.fields
                    .iter()
                    .find(|f| f.path == ["orchestrator", "profile"])
                    .map(|f| f.input.text.clone())
                    .unwrap_or_else(|| profile.clone())
            } else {
                profile.clone()
            };
            serde_json::json!({"executor": executor, "verifier": profile})
        };
        self.append_entry(vec![section, name.into()], &value);
    }

    fn begin_addition(&mut self) {
        let section = self.addition_section();
        let candidates = match section.as_str() {
            "providers" => kind_names::<ProviderKind>(),
            "routing" => kind_names::<WorkKind>(),
            _ => Ok(vec![]),
        };
        let candidates = match candidates {
            Ok(candidates) => candidates,
            Err(error) => {
                self.status = error;
                return;
            }
        };
        if candidates.is_empty() {
            self.add_profile();
        } else {
            self.offer_addition(section, candidates);
        }
    }

    fn addition_section(&self) -> String {
        let mut section = self.fields[self.selected].path[0].clone();
        // Empty sections have no selectable field. Keep structural recovery
        // reachable after deleting their last entry.
        if !self.fields.iter().any(|f| f.path[0] == "providers") {
            section = "providers".into();
        } else if !self.fields.iter().any(|f| f.path[0] == "routing") && section != "providers" {
            section = "routing".into();
        }
        section
    }

    fn offer_addition(&mut self, section: String, candidates: Vec<String>) {
        let choices = candidates
            .into_iter()
            .filter(|name| {
                !self
                    .fields
                    .iter()
                    .any(|field| field.belongs_to_entry(&section, name))
            })
            .collect::<Vec<_>>();
        if choices.is_empty() {
            self.status = format!("Cannot add to {section}: all kinds are configured.");
        } else {
            self.show_addition(section, choices, 0);
        }
    }

    fn add_profile(&mut self) {
        let mut index = 1;
        while self
            .fields
            .iter()
            .any(|f| f.path.get(1) == Some(&format!("new-profile-{index}")))
        {
            index += 1;
        }
        let name = format!("new-profile-{index}");
        let Some(provider) = self.first_entry_name("providers") else {
            self.status = "Cannot add a profile: no provider is configured.".into();
            return;
        };
        self.append_entry(vec!["profiles".into(), name], &serde_json::json!({"provider":provider,"model":null,"effort":"high","binary":null,"args":[],"command":null}));
    }

    fn append_entry(&mut self, path: Vec<String>, value: &Value) {
        let previous_len = self.fields.len();
        match self.flatten(path, value) {
            Ok(()) => {
                self.selected = previous_len;
                self.status = "Unsaved changes".into();
            }
            Err(error) => {
                self.fields.truncate(previous_len);
                self.status = error;
            }
        }
    }

    pub fn key(&mut self, key: KeyEvent) {
        if self.pending || self.addition_key(key) {
            return;
        }
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        if control && self.structural_key(key.code) {
            return;
        }
        self.edit_selected(key, control);
    }

    fn structural_key(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Char('n') => self.begin_addition(),
            KeyCode::Char('d') => self.delete_selected_entry(),
            _ => return false,
        }
        true
    }

    fn delete_selected_entry(&mut self) {
        let path = self.fields[self.selected].path.clone();
        if matches!(path[0].as_str(), "profiles" | "providers" | "routing") {
            self.fields
                .retain(|field| !field.belongs_to_entry(&path[0], &path[1]));
            self.selected = self.selected.min(self.fields.len() - 1);
            self.status = "Unsaved changes".into();
        }
    }

    fn edit_selected(&mut self, key: KeyEvent, control: bool) {
        if matches!(
            key.code,
            KeyCode::Backspace | KeyCode::Delete | KeyCode::Enter
        ) || matches!(key.code, KeyCode::Char(_)) && !control
        {
            self.fields[self.selected].error = None;
            self.status = "Unsaved changes".into();
        }
        match key.code {
            KeyCode::Esc => self.visible = false,
            KeyCode::Up | KeyCode::BackTab => self.selected = self.selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => {
                self.selected = (self.selected + 1).min(self.fields.len() - 1)
            }
            code => self.edit_field(code, control),
        }
    }

    fn edit_field(&mut self, code: KeyCode, control: bool) {
        let field = &mut self.fields[self.selected];
        if code == KeyCode::Enter && !field.choices.is_empty() {
            let next = field
                .choices
                .iter()
                .position(|v| *v == field.input.text)
                .map_or(0, |i| (i + 1) % field.choices.len());
            field.input = InputBuffer::with_text(&field.choices[next]);
            return;
        }
        match code {
            KeyCode::Char(c) if !control => field.input.insert_char(c),
            KeyCode::Backspace => {
                field.input.backspace();
            }
            KeyCode::Delete => {
                field.input.delete();
            }
            KeyCode::Left => field.input.move_left(),
            KeyCode::Right => field.input.move_right(),
            KeyCode::Home => field.input.move_home(),
            KeyCode::End => field.input.move_end(),
            _ => {}
        }
    }
}
