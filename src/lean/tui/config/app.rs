use super::super::App;
use super::super::Update;
use super::super::config;
use crate::lean::daemon::Request;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

impl App {
    pub(in crate::lean::tui) fn config_key(&mut self, key: KeyEvent) -> bool {
        if !self
            .config_editor
            .as_ref()
            .is_some_and(|editor| editor.visible)
        {
            return false;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('r') {
            self.reload_config();
        } else if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
            self.save_config();
        } else if let Some(editor) = self.config_editor.as_mut() {
            editor.key(key);
        }
        true
    }

    fn reload_config(&mut self) {
        if let Some(editor) = self.config_editor.as_mut().filter(|editor| !editor.pending) {
            editor.pending = true;
            editor.reload_requested = true;
            self.config_request(Request::GetConfig, false);
        }
    }

    fn save_config(&mut self) {
        let Some(editor) = self.config_editor.as_mut().filter(|editor| !editor.pending) else {
            return;
        };
        let Some(toml) = editor.source() else {
            if let Some(index) = editor.fields.iter().position(|field| field.error.is_some()) {
                editor.selected = index;
            }
            return;
        };
        editor.pending = true;
        editor.status = "Applying...".into();
        let expected_revision = crate::hash(&editor.base_source);
        self.config_request(
            Request::SetConfig {
                toml,
                expected_revision,
            },
            true,
        );
    }

    pub(in crate::lean::tui) fn open_config(&mut self) {
        if let Some(editor) = &mut self.config_editor {
            editor.visible = true;
            if editor.pending {
                return;
            }
            editor.pending = true;
        }
        self.config_request(Request::GetConfig, false);
    }

    fn config_request(&self, request: Request, save: bool) {
        let client = self.client.clone();
        let updates = self.updates.clone();
        let project = self.client.state_dir().to_string_lossy().into_owned();
        let expected = self.config_save_guard(save);
        std::thread::spawn(move || {
            let reply = config_reply(client, request, expected);
            let _ = updates.send(Update::Config {
                project,
                save,
                reply,
            });
        });
    }

    fn config_save_guard(&self, save: bool) -> Option<(String, String)> {
        save.then_some(self.config_editor.as_ref())
            .flatten()
            .map(|editor| (editor.base_source.clone(), editor.path.clone()))
    }
}

fn config_reply(
    client: crate::lean::daemon::Client,
    request: Request,
    expected: Option<(String, String)>,
) -> Result<serde_json::Value, String> {
    if let Some((source, path)) = expected {
        let current = config::ConfigEditor::from_reply(
            client
                .call(&Request::GetConfig)
                .map_err(|error| error.to_string())?,
        )?;
        if current.base_source != source || current.path != path {
            return Err("Configuration changed externally. Reopen /config or use Ctrl+R to reload; draft preserved.".into());
        }
    }
    client.call(&request).map_err(|error| error.to_string())
}
