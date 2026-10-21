//! Field-based configuration draft. Only a successful daemon reply marks it saved.
use super::{InputBuffer, Theme, sanitize_single_line_paste};
use crate::config::Config;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    text::Line,
    widgets::{Clear, Paragraph},
};
use serde_json::Value;

#[derive(Debug)]
pub struct Field {
    pub path: Vec<String>,
    pub input: InputBuffer,
    pub error: Option<String>,
    choices: Vec<&'static str>,
}

#[derive(Debug)]
pub struct ConfigEditor {
    pub fields: Vec<Field>,
    pub selected: usize,
    pub path: String,
    pub status: String,
    pub pending: bool,
    pub visible: bool,
}

impl ConfigEditor {
    pub fn new(source: &str, path: String) -> crate::Result<Self> {
        let config = Config::from_toml(source)?;
        let mut value = serde_json::to_value(config)?;
        for section in ["providers", "profiles"] {
            for entry in value[section].as_object_mut().unwrap().values_mut() {
                let table = entry.as_object_mut().unwrap();
                table.entry("binary").or_insert(Value::Null);
                table.entry("args").or_insert(serde_json::json!([]));
                table.entry("command").or_insert(Value::Null);
            }
        }
        let mut editor = Self {
            fields: vec![],
            selected: 0,
            path,
            status: String::new(),
            pending: false,
            visible: true,
        };
        editor.flatten(vec![], &value);
        Ok(editor)
    }

    fn flatten(&mut self, path: Vec<String>, value: &Value) {
        if let Some(table) = value.as_object() {
            for (key, value) in table {
                let mut child = path.clone();
                child.push(key.clone());
                self.flatten(child, value);
            }
        } else if path != ["schema_version"] {
            let key = path.last().unwrap().as_str();
            let choices = match key {
                "mode" if path[0] == "workspace" => vec!["in-place", "task-worktrees"],
                "mode" => vec!["supervised", "autonomous"],
                "provider" => vec!["agy", "codex", "claude", "gemini", "opencode"],
                "effort" => vec!["low", "medium", "high", "xhigh"],
                _ if value.is_boolean() => vec!["false", "true"],
                _ => vec![],
            };
            let text = match value {
                Value::String(s) => s.clone(),
                Value::Null => String::new(),
                _ => value.to_string(),
            };
            self.fields.push(Field {
                path,
                input: InputBuffer::with_text(text),
                error: None,
                choices,
            });
        }
    }

    pub fn source(&mut self) -> Option<String> {
        let mut root = serde_json::json!({"schema_version": 1});
        let mut invalid = false;
        for field in &mut self.fields {
            field.error = None;
            let text = &field.input.text;
            let key = field.path.last().unwrap().as_str();
            if !field.choices.is_empty() && !field.choices.contains(&text.as_str()) {
                field.error = Some(format!("Choose one of: {}", field.choices.join(", ")));
                invalid = true;
                continue;
            }
            if field.path == ["policy", "max_parallel"] && text == "0" {
                field.error = Some("Must be at least 1".into());
                invalid = true;
                continue;
            }

            let parsed = match key {
                "args" | "command" if !text.is_empty() => serde_json::from_str::<Vec<String>>(text)
                    .map(|v| serde_json::json!(v))
                    .map_err(|e| e.to_string()),
                "max_parallel" => text
                    .parse::<usize>()
                    .map(|n| serde_json::json!(n))
                    .map_err(|e| e.to_string()),
                "local_write" | "local_commits" | "required_gates" => text
                    .parse::<bool>()
                    .map(Value::Bool)
                    .map_err(|e| e.to_string()),
                "args" if text.is_empty() => Ok(serde_json::json!([])),
                "command" | "model" if text.is_empty() => Ok(Value::Null),
                "binary" if text.is_empty() && field.path[0] == "profiles" => Ok(Value::Null),
                _ => Ok(Value::String(text.clone())),
            };
            match parsed {
                Ok(value) => {
                    let mut cursor = &mut root;
                    for component in &field.path[..field.path.len() - 1] {
                        if cursor.get(component).is_none() {
                            cursor[component] = serde_json::json!({});
                        }
                        cursor = &mut cursor[component];
                    }
                    cursor[key] = value;
                }
                Err(error) => {
                    field.error = Some(error);
                    invalid = true;
                }
            }
        }
        if invalid {
            return None;
        }
        for field in &mut self.fields {
            let key = field.path.last().unwrap().as_str();
            let target = if field.path[0] == "routing" || field.path[0] == "orchestrator" {
                Some("profiles")
            } else if key == "provider" {
                Some("providers")
            } else {
                None
            };
            let value = field.path.iter().fold(&root, |value, key| &value[key]);
            let nonempty =
                |text: &str| !text.trim().is_empty() && !text.chars().any(char::is_control);
            if matches!(key, "args" | "command")
                && let Some(tokens) = value.as_array()
                && ((key == "command" && tokens.is_empty())
                    || tokens
                        .iter()
                        .any(|v| !nonempty(v.as_str().unwrap_or_default())))
            {
                field.error =
                    Some("Tokens must be nonempty and contain no control characters".into());
                invalid = true;
            }
            if matches!(key, "binary" | "model") && !value.is_null() {
                let command = &root[&field.path[0]][&field.path[1]]["command"];
                if !(key == "binary" && field.path[0] == "providers" && !command.is_null())
                    && !nonempty(value.as_str().unwrap_or_default())
                {
                    field.error = Some("Must be nonempty and contain no control characters".into());
                    invalid = true;
                }
            }
            if field.path == ["routing", "orchestration", "executor"]
                && value != &root["orchestrator"]["profile"]
            {
                field.error = Some("Must match orchestrator.profile".into());
                invalid = true;
            }
            if let Some(target) = target
                && root[target].get(&field.input.text).is_none()
            {
                field.error = Some(format!("Unknown {} entry", target));
                invalid = true;
            }
        }
        if invalid {
            return None;
        }
        let result = serde_json::from_value::<Config>(root.clone())
            .map_err(|e| e.to_string())
            .and_then(|c| c.validate().map_err(|e| e.to_string()));
        if let Err(error) = result {
            self.fields[self.selected].error = Some(error);
            return None;
        }
        fn emit(value: &Value, path: &[String], out: &mut String) {
            let table = value.as_object().unwrap();
            if !path.is_empty() {
                out.push_str(&format!(
                    "\n[{}]\n",
                    path.iter()
                        .map(|p| serde_json::to_string(p).unwrap())
                        .collect::<Vec<_>>()
                        .join(".")
                ));
            }
            for (key, value) in table {
                if !value.is_object() && !value.is_null() {
                    out.push_str(&format!(
                        "{} = {}\n",
                        serde_json::to_string(key).unwrap(),
                        value
                    ));
                }
            }
            for (key, value) in table {
                if value.is_object() {
                    let mut child = path.to_vec();
                    child.push(key.clone());
                    emit(value, &child, out);
                }
            }
        }
        let mut source = String::new();
        emit(&root, &[], &mut source);
        Some(source)
    }

    pub fn paste(&mut self, text: &str) {
        if self.pending {
            return;
        }
        self.fields[self.selected]
            .input
            .insert_str(&sanitize_single_line_paste(text));
        self.fields[self.selected].error = None;
        self.status = "Unsaved changes".into();
    }

    pub fn key(&mut self, key: KeyEvent) {
        if self.pending {
            return;
        }
        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        if control && key.code == KeyCode::Char('n') {
            let mut index = 1;
            while self
                .fields
                .iter()
                .any(|f| f.path.get(1) == Some(&format!("new-profile-{index}")))
            {
                index += 1;
            }
            let name = format!("new-profile-{index}");
            let provider = self
                .fields
                .iter()
                .find(|f| f.path[0] == "providers")
                .unwrap()
                .path[1]
                .clone();
            self.selected = self.fields.len();
            self.flatten(vec!["profiles".into(), name], &serde_json::json!({"provider":provider,"model":null,"effort":"high","binary":null,"args":[],"command":null}));
            self.status = "Unsaved changes".into();
            return;
        }
        if control && key.code == KeyCode::Char('d') {
            let path = self.fields[self.selected].path.clone();
            if path[0] == "profiles" {
                self.fields.retain(|f| f.path[..2] != path[..2]);
                self.selected = self.selected.min(self.fields.len() - 1);
                self.status = "Unsaved changes".into();
            }
            return;
        }
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
            code => {
                let field = &mut self.fields[self.selected];
                if code == KeyCode::Enter && !field.choices.is_empty() {
                    let next = field
                        .choices
                        .iter()
                        .position(|v| *v == field.input.text)
                        .map_or(0, |i| (i + 1) % field.choices.len());
                    field.input = InputBuffer::with_text(field.choices[next]);
                } else {
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
        }
    }

    pub fn draw(&self, frame: &mut Frame) {
        let area = frame.area();
        if area.width < 12 || area.height < 5 {
            return;
        }
        let theme = Theme::from_env();
        frame.render_widget(Clear, area);
        let global = super::trust::config_dir()
            .ok()
            .map(|p| p.join("antislop.toml"));
        let origin = if global.as_deref() == Some(std::path::Path::new(&self.path)) {
            "global"
        } else {
            "project"
        };
        let rows = usize::from(area.height.saturating_sub(5));
        let start = self.selected.saturating_sub(rows.saturating_sub(1));
        let mut lines = vec![
            Line::styled(
                super::sanitize_line(&format!("Configuration ({origin}): {}", self.path)),
                theme.title(),
            ),
            Line::raw("Running turns finish with their old profile."),
        ];
        for (index, field) in self.fields.iter().enumerate().skip(start).take(rows) {
            let label = format!(
                "{} = {}{}",
                field.path.join("."),
                field.input.text,
                field
                    .error
                    .as_ref()
                    .map(|e| format!("  ERROR: {e}"))
                    .unwrap_or_default()
            );
            lines.push(Line::styled(
                super::sanitize_line(&label),
                if index == self.selected {
                    theme.selected()
                } else {
                    theme.text()
                },
            ));
        }
        lines.resize_with(2 + rows, || Line::raw(""));
        let selected = &self.fields[self.selected];
        let width = usize::from(area.width.saturating_sub(1).max(1));
        let wrapped = super::visual_text_lines(&selected.input.text, width);
        let (col, row) = selected.input.visual_cursor(width);
        lines.push(Line::styled(
            wrapped.get(row).cloned().unwrap_or_default(),
            theme.selected(),
        ));
        lines.push(Line::raw(
            "Tab: field  Enter: cycle  Ctrl+N/D: add/delete profile  Ctrl+S: apply  Esc: close",
        ));
        lines.push(Line::raw(super::sanitize_line(
            selected.error.as_deref().unwrap_or(&self.status),
        )));
        frame.render_widget(Paragraph::new(lines), area);
        frame.set_cursor_position((
            area.x + col.min(area.width.saturating_sub(1)),
            area.bottom().saturating_sub(3),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_fields_round_trip_and_invalid_values_stay_in_draft() {
        let source = include_str!("../../../examples/antislop.toml");
        let mut editor = ConfigEditor::new(source, "antislop.toml".into()).unwrap();
        assert_eq!(
            Config::from_toml(&editor.source().unwrap()).unwrap(),
            Config::from_toml(source).unwrap()
        );
        let index = editor
            .fields
            .iter()
            .position(|f| f.path.last().unwrap() == "max_parallel")
            .unwrap();
        editor.fields[index].input = InputBuffer::with_text("wrong");
        assert!(editor.source().is_none());
        assert!(editor.fields[index].error.is_some());
        assert_eq!(editor.fields[index].input.text, "wrong");
    }
    #[test]
    fn optional_overrides_commands_and_field_errors_round_trip() {
        let mut editor = ConfigEditor::new(
            include_str!("../../../examples/antislop.toml"),
            "antislop.toml".into(),
        )
        .unwrap();
        let binary = editor
            .fields
            .iter()
            .position(|f| f.path == ["providers", "codex", "binary"])
            .unwrap();
        let command = editor
            .fields
            .iter()
            .position(|f| f.path == ["providers", "codex", "command"])
            .unwrap();
        editor.fields[binary].input.clear();
        editor.fields[command].input = InputBuffer::with_text(r#"["wrapper", "path with spaces"]"#);
        let source = editor.source().unwrap();
        let parsed = Config::from_toml(&source).unwrap();
        assert_eq!(
            parsed.providers[&crate::config::ProviderKind::Codex]
                .command
                .as_ref()
                .unwrap()
                .0,
            ["wrapper", "path with spaces"]
        );
        editor.fields[command].input = InputBuffer::with_text("[]");
        assert!(editor.source().is_none());
        assert!(editor.fields[command].error.is_some());
        let route = editor
            .fields
            .iter()
            .position(|f| f.path == ["orchestrator", "profile"])
            .unwrap();
        editor.fields[command].input = InputBuffer::with_text(r#"["wrapper"]"#);
        editor.fields[route].input = InputBuffer::with_text("unknown-profile");
        assert!(editor.source().is_none());
        assert!(editor.fields[route].error.is_some());
    }
}
