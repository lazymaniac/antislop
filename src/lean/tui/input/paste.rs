use super::*;

impl App {
    pub fn paste(&mut self, text: &str) {
        if self.paste_native_or_overlay(text) {
            return;
        }
        self.sync_interview();
        if self.paste_form(text) || self.paste_details(text) {
            return;
        }
        if self.chat_paste_enabled() && self.attach_dropped_images(text) {
            return;
        }
        self.paste_picker_editor_or_chat(text);
    }

    fn attach_dropped_images(&mut self, text: &str) -> bool {
        let lines = text
            .lines()
            .map(|line| line.trim().trim_matches(['\'', '"']))
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        if lines.is_empty() {
            return false;
        }
        let candidates = lines
            .iter()
            .map(|line| (decode_dropped_path(line), line.starts_with("file://")))
            .collect::<Vec<_>>();
        if !candidates
            .iter()
            .all(|(path, file_uri)| is_confirmed_image_drop(path, *file_uri))
        {
            return false;
        }
        for (path, _) in candidates {
            self.add_image_attachment(path.into());
        }
        true
    }

    pub(super) fn add_image_attachment(&mut self, path: std::path::PathBuf) {
        let result = (|| -> std::io::Result<()> {
            let metadata = std::fs::metadata(&path)?;
            if !metadata.is_file() {
                return Err(std::io::Error::other("path is not a file"));
            }
            if metadata.len() > crate::lean::content::MAX_IMAGE_BYTES as u64 {
                return Err(std::io::Error::other(
                    "image attachment exceeds the 20 MiB limit",
                ));
            }
            let bytes = std::fs::read(&path)?;
            crate::lean::content::image_type(&bytes)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            Ok(())
        })();
        match result {
            Ok(()) => {
                let pending = self
                    .image_drafts
                    .entry(self.conversation.clone())
                    .or_default();
                if !pending.contains(&path) {
                    pending.push(path.clone());
                }
                self.notice = Some(format!(
                    "Attached {} · {} image(s); Alt+Backspace removes the last",
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    pending.len()
                ));
            }
            Err(error) => {
                self.notice = Some(format!(
                    "Image attachment failed for {}: {error}",
                    path.display()
                ))
            }
        }
    }

    fn paste_native_or_overlay(&mut self, text: &str) -> bool {
        if self.native_focused() {
            self.native_send(crate::lean::terminal::Input::Paste { text: text.into() });
            return true;
        }
        if self.plan_history.is_some()
            || self.help.is_some()
            || self.decision_inbox.is_some()
            || self.unblock.is_some()
            || self.recovery_consent.is_some()
            || self.graph_filters.is_some()
        {
            return true;
        }

        if let Some(editor) = self.config_editor.as_mut().filter(|editor| editor.visible) {
            editor.paste(text);
            return true;
        }
        false
    }

    fn paste_form(&mut self, text: &str) -> bool {
        if self.picker.is_some() || self.editor.is_some() || !self.interview_visible() {
            return false;
        }
        if let Some(form) = self.interview_forms.get_mut(&self.conversation) {
            form.paste(text);
        }
        true
    }

    fn paste_details(&mut self, text: &str) -> bool {
        if self.picker.is_some()
            || self.editor.is_some()
            || self.plan_view.is_some()
            || (self.diff.is_some()
                && !self.details.as_ref().is_some_and(|details| details.editing))
        {
            return false;
        }
        if self.navigation.focus == NavigationFocus::Composer
            && !self.details.as_ref().is_some_and(|details| details.editing)
        {
            return false;
        }
        let Some(details) = &mut self.details else {
            return false;
        };
        if details.editing && details.selected_round.is_none() {
            let key = details.composer_key();
            details
                .drafts
                .borrow_mut()
                .entry(key)
                .or_default()
                .input
                .insert_str(&sanitize_paste(text));
        }
        true
    }

    fn paste_picker_editor_or_chat(&mut self, text: &str) {
        if let Some(picker) = &mut self.picker {
            picker.query.push_str(&sanitize_single_line_paste(text));
            picker.selected = 0;
        } else if let Some(editor) = &mut self.editor {
            editor.input.insert_str(&sanitize_paste(text));
        } else if self.chat_paste_enabled() {
            self.input.insert_str(&sanitize_paste(text));
        }
    }

    fn chat_paste_enabled(&self) -> bool {
        self.navigation.background_focus == Focus::Chat
            && self.plan_view.is_none()
            && self.diff.is_none()
            && (self.details.is_none()
                || (self.navigation.focus == NavigationFocus::Composer
                    && !self.details.as_ref().is_some_and(|details| details.editing)))
    }
}

fn is_image_path(path: &str) -> bool {
    [".png", ".jpg", ".jpeg", ".gif", ".webp", ".tif", ".tiff"]
        .iter()
        .any(|extension| path.to_ascii_lowercase().ends_with(extension))
}

fn is_confirmed_image_drop(path: &str, file_uri: bool) -> bool {
    if !is_image_path(path) {
        return false;
    }

    if file_uri {
        return true;
    }

    match std::fs::metadata(path) {
        Ok(metadata) => metadata.is_file(),
        // A missing filename is common in ordinary pasted prose (for example,
        // "the mockup is in screenshot.png"). An absolute path that exists
        // but cannot be inspected is still likely a real drop, so let the
        // attachment loader surface its concrete filesystem error.
        Err(error) => {
            error.kind() != std::io::ErrorKind::NotFound && std::path::Path::new(path).is_absolute()
        }
    }
}

fn decode_dropped_path(path: &str) -> String {
    let path = path.strip_prefix("file://").unwrap_or(path);
    let localhost_path;
    let path = if let Some(path) = path.strip_prefix("localhost/") {
        localhost_path = format!("/{path}");
        localhost_path.as_str()
    } else {
        path
    };
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let (Some(high), Some(low)) = (hex(bytes[index + 1]), hex(bytes[index + 2]))
        {
            decoded.push(high * 16 + low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn hex(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}
