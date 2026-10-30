use super::model::toml_value;
use super::*;
impl ConfigEditor {
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
        self.remove_missing_entries(&mut doc);
        for field in &self.fields {
            self.patch_field(&mut doc, field)?;
        }
        Ok(doc.to_string())
    }

    fn remove_missing_entries(&self, doc: &mut toml_edit::DocumentMut) {
        for section in ["profiles", "providers", "routing"] {
            if let Some(entries) = doc
                .get_mut(section)
                .and_then(|item| item.as_table_like_mut())
            {
                let removed: Vec<String> = entries
                    .iter()
                    .map(|(name, _)| name.to_owned())
                    .filter(|name| {
                        !self
                            .fields
                            .iter()
                            .any(|field| field.belongs_to_entry(section, name))
                    })
                    .collect();
                for name in removed {
                    entries.remove(&name);
                }
            }
        }
    }

    fn patch_field(&self, doc: &mut toml_edit::DocumentMut, field: &Field) -> Result<(), String> {
        if self.baseline.get(&field.path) == Some(&field.input.text) {
            return Ok(());
        }
        let value = field.value()?;
        let cursor = locate_field(doc.as_item_mut(), &field.path);
        write_field(cursor, field.path.last().unwrap(), value)?;
        Ok(())
    }
}

fn locate_field<'a>(
    mut cursor: &'a mut toml_edit::Item,
    path: &[String],
) -> &'a mut toml_edit::Item {
    for component in &path[..path.len() - 1] {
        if cursor.get(component).is_none() {
            cursor[component] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        cursor = &mut cursor[component];
    }
    cursor
}

fn write_field(cursor: &mut toml_edit::Item, key: &str, value: Value) -> Result<(), String> {
    if value.is_null() {
        if let Some(table) = cursor.as_table_like_mut() {
            table.remove(key);
        }
        return Ok(());
    }
    let mut replacement = toml_value(&value)?;
    if let Some(existing) = cursor.get(key).and_then(|item| item.as_value()) {
        *replacement.decor_mut() = existing.decor().clone();
    }
    cursor[key] = toml_edit::Item::Value(replacement);
    Ok(())
}
