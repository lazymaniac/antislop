use super::super::*;

impl App {
    pub(super) fn apply_plan_history(&mut self, update: Update) {
        match update {
            Update::PlanHistory {
                conversation,
                generation,
                offset,
                reply,
            } => {
                if conversation == self.conversation
                    && generation == self.generation
                    && let Some(browser) = self
                        .plan_history
                        .as_mut()
                        .filter(|browser| browser.offset == offset)
                {
                    match reply {
                        Ok(page) => browser.loaded(page),
                        Err(error) => {
                            browser.loading = false;
                            browser.error = Some(error);
                        }
                    }
                }
            }
            _ => unreachable!("update routed to wrong handler"),
        }
    }

    pub(super) fn apply_config(&mut self, update: Update) {
        let Update::Config {
            project,
            save,
            reply,
        } = update
        else {
            unreachable!("update routed to wrong handler");
        };
        let active = project == self.client.state_dir().to_string_lossy();
        if save {
            self.apply_config_save(&project, active, reply);
        } else {
            self.refresh_config(&project, active, reply);
        }
    }

    fn apply_config_save(
        &mut self,
        project: &str,
        active: bool,
        reply: std::result::Result<Value, String>,
    ) {
        let editor = if active {
            self.config_editor.as_mut()
        } else {
            self.config_drafts.get_mut(project)
        };
        if let Some(editor) = editor {
            editor.pending = false;
            editor.status = save_config(editor, reply);
        }
    }

    fn refresh_config(
        &mut self,
        project: &str,
        active: bool,
        reply: std::result::Result<Value, String>,
    ) {
        match reply.and_then(config::ConfigEditor::from_reply) {
            Ok(fresh) => self.install_fresh_config(project, active, fresh),
            Err(error) => self.record_config_error(project, active, error),
        }
    }

    fn install_fresh_config(
        &mut self,
        project: &str,
        active: bool,
        mut fresh: config::ConfigEditor,
    ) {
        if active {
            if let Some(editor) = &mut self.config_editor {
                editor.refresh(fresh);
            } else {
                self.config_editor = Some(fresh);
            }
            return;
        }
        fresh.visible = false;
        if let Some(editor) = self.config_drafts.get_mut(project) {
            editor.refresh(fresh);
        } else {
            self.config_drafts.insert(project.to_owned(), fresh);
        }
    }

    fn record_config_error(&mut self, project: &str, active: bool, error: String) {
        let editor = if active {
            self.config_editor.as_mut()
        } else {
            self.config_drafts.get_mut(project)
        };
        if let Some(editor) = editor {
            editor.pending = false;
            editor.reload_requested = false;
            editor.status = error.clone();
        }
        self.notice = Some(error);
    }

    pub(super) fn apply_conversation(&mut self, update: Update) {
        match update {
            Update::Conversation(id) => self.open(id),
            _ => unreachable!("update routed to wrong handler"),
        }
    }

    pub(super) fn apply_conversation_created(&mut self, update: Update) {
        let Update::ConversationCreated {
            project,
            state_dir,
            request_id,
            generation,
            result,
        } = update
        else {
            unreachable!("update routed to wrong handler");
        };
        let id = match result {
            Ok(id) if !id.is_empty() => id,
            Ok(_) => {
                self.notice = Some(format!(
                    "✗ {}: daemon returned an empty session id",
                    project.display()
                ));
                return;
            }
            Err(error) => {
                self.notice = Some(format!(
                    "✗ {}: {error} (session was not opened)",
                    project.display()
                ));
                return;
            }
        };
        if let Some(owner) = self.projects.iter_mut().find(|owner| owner.path == project)
            && !owner.conversations.iter().any(|item| item["id"] == id)
        {
            owner
                .conversations
                .push(serde_json::json!({"id": id, "project": project.to_string_lossy()}));
        }
        self.conversation_projects
            .insert(id.clone(), project.clone());
        let active_state_dir = self.client.state_dir();
        let active_project = self
            .projects
            .get(self.active_project)
            .map(|owner| owner.path.as_path());
        let project_is_active = active_project.map_or(project.as_os_str().is_empty(), |path| {
            path == project.as_path()
        });
        if request_id != self.new_conversation_request_id
            || generation != self.generation
            || !project_is_active
            || active_state_dir != state_dir
        {
            return;
        }
        if let Some(index) = self.projects.iter().position(|owner| owner.path == project) {
            self.switch_project(index, Some(id));
        } else {
            self.open(id);
        }
        self.navigation.screen = Screen::Workbench;
        self.navigation.target = WorkbenchTarget::Session;
        self.navigation.focus = NavigationFocus::Composer;
        self.navigation.background_focus = Focus::Chat;
    }

    pub(super) fn apply_project_ready(&mut self, update: Update) {
        match update {
            Update::ProjectReady { conn, conversation } => {
                let index = self.register_project(*conn);
                self.switch_project(index, conversation);
            }
            _ => unreachable!("update routed to wrong handler"),
        }
    }

    pub(super) fn apply_project_status(&mut self, update: Update) {
        match update {
            Update::ProjectStatus {
                path,
                mut conversations,
            } => {
                let present: std::collections::HashSet<String> = conversations
                    .iter()
                    .filter_map(|item| item["id"].as_str().map(str::to_owned))
                    .collect();
                self.deleted_conversations
                    .retain(|(deleted_path, id)| deleted_path != &path || present.contains(id));
                conversations.retain(|item| {
                    item["id"].as_str().is_none_or(|id| {
                        !self
                            .deleted_conversations
                            .contains(&(path.clone(), id.to_owned()))
                    })
                });
                if let Some(project) = self.projects.iter_mut().find(|p| p.path == path) {
                    project.conversations = conversations;
                }
            }
            _ => unreachable!("update routed to wrong handler"),
        }
    }

    pub(super) fn apply_notice(&mut self, update: Update) {
        let Update::Notice(text) = update else {
            unreachable!("update routed to wrong handler");
        };
        self.notice = Some(text);
    }
}

fn save_config(
    editor: &mut config::ConfigEditor,
    reply: std::result::Result<Value, String>,
) -> String {
    match reply.and_then(|value| apply_saved_config(editor, value.clone())) {
        Ok(value)
            if value["warnings"]
                .as_array()
                .is_some_and(|warnings| !warnings.is_empty()) =>
        {
            format!("Applied live with warnings: {}", value["warnings"])
        }
        Ok(_) => "Saved and applied live.".into(),
        Err(error) => error,
    }
}

fn apply_saved_config(
    editor: &mut config::ConfigEditor,
    value: Value,
) -> std::result::Result<Value, String> {
    let saved = config::ConfigEditor::from_reply(value.clone())?;
    if saved.path != editor.path {
        return Err("Configuration reply refers to a different file".into());
    }
    editor.applied(&saved.base_source);
    Ok(value)
}
