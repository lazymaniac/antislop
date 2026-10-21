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
use std::collections::BTreeMap;

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
    pub conflict: bool,
    pub reload_requested: bool,
    pub base_source: String,
    baseline: BTreeMap<Vec<String>, String>,
}

impl Field {
    fn value(&self) -> Result<Value, String> {
        let text = &self.input.text;
        let key = self.path.last().ok_or("Missing field path")?.as_str();
        match key {
            "args" | "command" if !text.is_empty() => serde_json::from_str::<Vec<String>>(text)
                .map(|v| serde_json::json!(v))
                .map_err(|e| e.to_string()),
            "args" => Ok(serde_json::json!([])),
            "max_parallel" => text
                .parse::<i64>()
                .map(|v| serde_json::json!(v))
                .map_err(|e| e.to_string()),
            "local_write" | "local_commits" | "required_gates" => text
                .parse::<bool>()
                .map(Value::Bool)
                .map_err(|e| e.to_string()),
            "command" | "model" if text.is_empty() => Ok(Value::Null),
            "binary" if text.is_empty() && self.path[0] == "profiles" => Ok(Value::Null),
            _ => Ok(Value::String(text.clone())),
        }
    }
}

fn toml_value(value: &Value) -> Result<toml_edit::Value, String> {
    match value {
        Value::String(s) => Ok(s.as_str().into()),
        Value::Bool(b) => Ok((*b).into()),
        Value::Number(n) => n
            .as_i64()
            .map(Into::into)
            .ok_or_else(|| "Integer exceeds TOML range".into()),
        Value::Array(values) => {
            let mut array = toml_edit::Array::new();
            for value in values {
                array.push(toml_value(value)?);
            }
            Ok(array.into())
        }
        _ => Err("Unsupported configuration value".into()),
    }
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
            conflict: false,
            reload_requested: false,
            base_source: source.into(),
            baseline: BTreeMap::new(),
        };
        editor.flatten(vec![], &value);
        editor.baseline = editor.inputs();
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

    fn inputs(&self) -> BTreeMap<Vec<String>, String> {
        self.fields
            .iter()
            .map(|f| (f.path.clone(), f.input.text.clone()))
            .collect()
    }

    pub fn is_dirty(&self) -> bool {
        self.inputs() != self.baseline
    }

    pub fn from_reply(value: Value) -> Result<Self, String> {
        #[derive(serde::Deserialize)]
        struct Reply {
            toml: String,
            path: String,
        }
        let reply: Reply = serde_json::from_value(value)
            .map_err(|e| format!("Invalid configuration reply: {e}"))?;
        if reply.path.is_empty() {
            return Err("Invalid configuration reply: empty path".into());
        }
        Self::new(&reply.toml, reply.path).map_err(|e| e.to_string())
    }

    /// Refresh clean drafts; preserve edits and block a save after an external change.
    pub fn refresh(&mut self, mut fresh: Self) {
        fresh.visible = self.visible;
        if self.reload_requested || !self.is_dirty() {
            *self = fresh;
        } else {
            self.pending = false;
            self.conflict = self.base_source != fresh.base_source || self.path != fresh.path;
            if self.conflict {
                self.status = "Changed externally. Ctrl+R: reload and discard this draft.".into();
            }
        }
    }

    pub fn applied(&mut self, source: &str) {
        self.base_source = source.into();
        self.baseline = self.inputs();
        self.conflict = false;
    }

    fn parse_draft(&mut self) -> Option<Value> {
        let mut root = serde_json::json!({"schema_version": 1, "profiles": {}, "providers": {}, "routing": {}});
        let mut invalid = false;
        for field in &mut self.fields {
            field.error = None;
            match field.value() {
                Ok(value) => {
                    let mut cursor = &mut root;
                    for component in &field.path[..field.path.len() - 1] {
                        if cursor.get(component).is_none() {
                            cursor[component] = serde_json::json!({});
                        }
                        cursor = &mut cursor[component];
                    }
                    cursor[field.path.last().unwrap()] = value;
                }
                Err(error) => {
                    field.error = Some(error);
                    invalid = true;
                }
            }
        }
        if invalid { None } else { Some(root) }
    }

    fn validate_draft(&mut self, value: Value) -> bool {
        let result = serde_json::from_value::<Config>(value)
            .map_err(|e| e.to_string())
            .and_then(|c| c.validate().map_err(|e| e.to_string()));
        if let Err(error) = result {
            // The Config validator owns semantic and cross-field rules. Show its
            // error beside edited fields, including edits that interact.
            let mut marked = false;
            for field in &mut self.fields {
                if self.baseline.get(&field.path) != Some(&field.input.text) {
                    field.error = Some(error.clone());
                    marked = true;
                }
            }
            if !marked && let Some(field) = self.fields.get_mut(self.selected) {
                field.error = Some(error);
            }
            false
        } else {
            true
        }
    }

    pub fn source(&mut self) -> Option<String> {
        if self.conflict {
            return None;
        }
        let value = self.parse_draft()?;
        if !self.validate_draft(value) {
            return None;
        }
        match self.emit_draft() {
            Ok(source) => Some(source),
            Err(error) => {
                self.status = error;
                None
            }
        }
    }

    /// Patch the loaded document so unrelated TOML comments and formatting survive.
    fn emit_draft(&self) -> Result<String, String> {
        let mut doc = self
            .base_source
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| e.to_string())?;
        if let Some(profiles) = doc.get_mut("profiles").and_then(|p| p.as_table_like_mut()) {
            let removed: Vec<String> = profiles
                .iter()
                .map(|(name, _)| name.to_owned())
                .filter(|name| {
                    !self
                        .fields
                        .iter()
                        .any(|f| f.path[0] == "profiles" && &f.path[1] == name)
                })
                .collect();
            for name in removed {
                profiles.remove(&name);
            }
        }
        for field in &self.fields {
            if self.baseline.get(&field.path) == Some(&field.input.text) {
                continue;
            }
            let value = field.value()?;
            let mut cursor = doc.as_item_mut();
            for component in &field.path[..field.path.len() - 1] {
                if cursor.get(component).is_none() {
                    cursor[component] = toml_edit::Item::Table(toml_edit::Table::new());
                }
                cursor = &mut cursor[component];
            }
            let key = field.path.last().unwrap();
            if value.is_null() {
                if let Some(table) = cursor.as_table_like_mut() {
                    table.remove(key);
                }
            } else {
                let mut replacement = toml_value(&value)?;
                if let Some(existing) = cursor.get(key).and_then(|item| item.as_value()) {
                    *replacement.decor_mut() = existing.decor().clone();
                }
                cursor[key] = toml_edit::Item::Value(replacement);
            }
        }
        Ok(doc.to_string())
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
            let Some(provider) = self
                .fields
                .iter()
                .find(|f| f.path.first().is_some_and(|p| p == "providers"))
                .and_then(|f| f.path.get(1))
                .cloned()
            else {
                self.status = "Cannot add a profile: no provider is configured.".into();
                return;
            };
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
        self.draw_with_theme(
            frame,
            &Theme::from_env(),
            super::trust::config_dir()
                .ok()
                .map(|p| p.join("antislop.toml"))
                .as_deref(),
        );
    }

    pub fn draw_with_theme(
        &self,
        frame: &mut Frame,
        theme: &Theme,
        global: Option<&std::path::Path>,
    ) {
        let area = frame.area();
        if area.width < 12 || area.height < 5 {
            return;
        }
        frame.render_widget(Clear, area);
        let origin = if global == Some(std::path::Path::new(&self.path)) {
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
            "Tab: field  Enter: cycle  Ctrl+N/D: add/delete profile  Ctrl+S: apply  Ctrl+R: reload  Esc: close",
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
    #[test]
    fn document_patching_preserves_comments_and_reports_integer_overflow_at_the_field() {
        let source = include_str!("../../../examples/antislop.toml")
            .replace("max_parallel = 4", "max_parallel = 4 # keep this comment");
        let mut editor = ConfigEditor::new(&source, "antislop.toml".into()).unwrap();
        let index = editor
            .fields
            .iter()
            .position(|f| f.path == ["policy", "max_parallel"])
            .unwrap();
        editor.fields[index].input = InputBuffer::with_text("2");
        assert!(
            editor
                .source()
                .unwrap()
                .contains("max_parallel = 2 # keep this comment")
        );
        editor.fields[index].input = InputBuffer::with_text("18446744073709551615");
        assert!(editor.source().is_none());
        assert!(editor.fields[index].error.is_some());
    }
}
