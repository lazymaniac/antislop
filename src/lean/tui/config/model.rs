use super::*;
#[derive(Debug)]
pub struct Field {
    pub path: Vec<String>,
    pub input: InputBuffer,
    pub error: Option<String>,
    pub(super) choices: Vec<String>,
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
    pub(super) baseline: BTreeMap<Vec<String>, String>,
    pub(super) adding: Option<(String, Vec<String>, usize)>,
}

impl Field {
    pub(super) fn belongs_to_entry(&self, section: &str, name: &str) -> bool {
        self.path[0] == section && self.path[1] == name
    }

    pub(super) fn value(&self) -> Result<Value, String> {
        let text = &self.input.text;
        let key = self.path.last().ok_or("Missing field path")?.as_str();
        match key {
            "args" | "command" if !text.is_empty() => serde_json::from_str::<Vec<String>>(text)
                .map(|v| serde_json::json!(v))
                .map_err(|e| e.to_string()),
            "args" => Ok(serde_json::json!([])),
            "max_parallel" | "max_tasks" => text
                .parse::<i64>()
                .map(|v| serde_json::json!(v))
                .map_err(|e| e.to_string()),
            "local_write" | "local_commits" | "required_gates" | "local_options" => text
                .parse::<bool>()
                .map(Value::Bool)
                .map_err(|e| e.to_string()),
            "command" | "model" if text.is_empty() => Ok(Value::Null),
            "binary" if text.is_empty() && self.path[0] == "profiles" => Ok(Value::Null),
            _ => Ok(Value::String(text.clone())),
        }
    }
}

pub(super) fn toml_value(value: &Value) -> Result<toml_edit::Value, String> {
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

#[derive(Debug, thiserror::Error)]
enum KindNameError {
    #[error("unknown configuration kind")]
    Unknown(&'static [&'static str]),
    #[error("{0}")]
    Other(String),
}

impl serde::de::Error for KindNameError {
    fn custom<T: std::fmt::Display>(message: T) -> Self {
        Self::Other(message.to_string())
    }

    fn unknown_variant(_variant: &str, expected: &'static [&'static str]) -> Self {
        Self::Unknown(expected)
    }
}

pub(super) fn kind_names<T: serde::de::DeserializeOwned>() -> Result<Vec<String>, String> {
    // An empty name is invalid for configuration kinds. Serde's documented
    // unknown_variant error supplies the accepted names as structured data.
    match T::deserialize(serde::de::value::StrDeserializer::<KindNameError>::new("")) {
        Err(KindNameError::Unknown(names)) => Ok(names.iter().map(|name| (*name).into()).collect()),
        Err(error) => Err(format!("Cannot list configuration kinds: {error}")),
        Ok(_) => {
            Err("Cannot list configuration kinds: empty variant names are unsupported.".into())
        }
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
            adding: None,
        };
        editor
            .flatten(vec![], &value)
            .map_err(crate::Error::Invalid)?;
        editor.baseline = editor.inputs();
        Ok(editor)
    }

    pub(super) fn flatten(&mut self, path: Vec<String>, value: &Value) -> Result<(), String> {
        if let Some(table) = value.as_object() {
            self.flatten_table(&path, table)?;
        } else if path != ["schema_version"] {
            self.push_field(path, value)?;
        }
        Ok(())
    }

    fn flatten_table(
        &mut self,
        path: &[String],
        table: &serde_json::Map<String, Value>,
    ) -> Result<(), String> {
        for (key, value) in table {
            let mut child = path.to_vec();
            child.push(key.clone());
            self.flatten(child, value)?;
        }
        Ok(())
    }

    fn push_field(&mut self, path: Vec<String>, value: &Value) -> Result<(), String> {
        let key = path.last().unwrap().as_str();
        let choices = match key {
            "provider" => kind_names::<ProviderKind>()?,
            "mode" => strings(&["supervised", "autonomous"]),
            "effort" => strings(&["low", "medium", "high", "xhigh"]),
            _ if value.is_boolean() => strings(&["false", "true"]),
            _ => vec![],
        };
        self.fields.push(Field {
            path,
            input: InputBuffer::with_text(field_text(value)),
            error: None,
            choices,
        });
        Ok(())
    }

    pub(super) fn inputs(&self) -> BTreeMap<Vec<String>, String> {
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

    pub(super) fn parse_draft(&mut self) -> Option<Value> {
        let mut root = serde_json::json!({"schema_version": 2, "profiles": {}, "providers": {}, "routing": {}});
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

    pub(super) fn validate_draft(&mut self, value: Value) -> bool {
        match self.validation_error(value) {
            Ok(()) => true,
            Err(error) => {
                self.mark_validation_error(error);
                false
            }
        }
    }

    fn validation_error(&self, value: Value) -> Result<(), String> {
        let config = serde_json::from_value::<Config>(value).map_err(|e| e.to_string())?;
        config.validate().map_err(|e| e.to_string())?;
        if self.removes_orchestration() {
            Err("Cannot remove routing.orchestration.".into())
        } else {
            Ok(())
        }
    }

    fn removes_orchestration(&self) -> bool {
        let existed = self
            .baseline
            .keys()
            .any(|path| path[0] == "routing" && path[1] == "orchestration");
        existed
            && !self
                .fields
                .iter()
                .any(|field| field.path[0] == "routing" && field.path[1] == "orchestration")
    }

    fn mark_validation_error(&mut self, error: String) {
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
    }
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).into()).collect()
}

fn field_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        _ => value.to_string(),
    }
}
