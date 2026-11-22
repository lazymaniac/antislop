use super::super::*;

fn conversation_title(value: &Value, id: &str, current: bool) -> String {
    let status = value["status"].as_str().unwrap_or("idle");
    let status = match value["status_error"].as_str() {
        Some(error) => format!("{status}: {}", view::clean_line(error)),
        None => status.to_owned(),
    };
    let mark = if current { " (current)" } else { "" };
    let total = value["tasks_total"].as_u64().unwrap_or(0);
    let goal = value["goal"]
        .as_str()
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let subject = if goal.is_empty() {
        format!("New session · {id}")
    } else {
        goal
    };
    if total == 0 {
        format!("{subject}{mark} · {status}")
    } else {
        let done = value["tasks_done"].as_u64().unwrap_or(0);
        format!("{subject}{mark} · {status} · {done}/{total} tasks")
    }
}

impl App {
    pub fn open_conversation_picker(&mut self) {
        let mut items = Vec::new();
        self.append_project_conversations(&mut items, true);
        self.append_saved_conversations(&mut items);
        self.ensure_active_conversation(&mut items);
        let mut picker = Picker::new(Kind::Conversations);
        picker.conversations = items;
        picker.status = Some("Loading sessions...".into());
        self.picker = Some(picker);
        self.conversations_request_id = self.conversations_request_id.wrapping_add(1);
        let request_id = self.conversations_request_id;
        let client = self.client.clone();
        let updates = self.updates.clone();
        let state_dir = client.state_dir().to_string_lossy().into_owned();
        std::thread::spawn(move || {
            let result = client.call(&Request::Control(
                crate::lean::daemon::ControlRequest::Conversations,
            ));
            let _ = updates.send(Update::Conversations {
                state_dir,
                request_id,
                result: result.map_err(|error| error.to_string()),
            });
        });
    }

    pub(in crate::lean::tui) fn apply_conversations(
        &mut self,
        state_dir: String,
        request_id: u64,
        result: std::result::Result<Value, String>,
    ) {
        if state_dir != self.client.state_dir().to_string_lossy()
            || request_id != self.conversations_request_id
            || !self
                .picker
                .as_ref()
                .is_some_and(|picker| picker.kind == Kind::Conversations)
        {
            return;
        }
        let values = match result {
            Ok(Value::Array(values)) => values,
            Ok(_) => {
                self.picker.as_mut().unwrap().status = Some("Invalid session list response".into());
                return;
            }
            Err(error) => {
                self.picker.as_mut().unwrap().status = Some(error.clone());
                self.notice = Some(error);
                return;
            }
        };
        let mut items = values
            .iter()
            .filter_map(|value| self.daemon_conversation(value))
            .collect::<Vec<_>>();
        self.append_project_conversations(&mut items, false);
        self.append_saved_conversations(&mut items);
        self.ensure_active_conversation(&mut items);
        let picker = self.picker.as_mut().unwrap();
        picker.set_conversations(items, &self.view);
        if !picker.delete_target.as_ref().is_some_and(|target| {
            self.conversation_deletions
                .contains(&(target.project.clone(), target.id.clone()))
        }) {
            picker.status = None;
        }
    }

    fn daemon_conversation(&self, value: &Value) -> Option<palette::ConversationItem> {
        let project = value["project"].as_str().unwrap_or_default();
        let active_path = self
            .projects
            .get(self.active_project)
            .map(|project| project.path.to_string_lossy().into_owned());
        if self.projects.is_empty() && !project.is_empty() {
            let basename = std::path::Path::new(project)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(project);
            if project != self.project && basename != self.project {
                return None;
            }
        }
        if !project.is_empty()
            && active_path
                .as_deref()
                .is_some_and(|active| active != project)
            && self
                .projects
                .iter()
                .all(|candidate| candidate.path.to_string_lossy() != project)
        {
            return None;
        }
        let id = value["id"].as_str()?.to_owned();
        if id.is_empty() {
            return None;
        }
        let project = if project.is_empty() {
            active_path.unwrap_or_default()
        } else {
            project.to_owned()
        };
        if self
            .deleted_conversations
            .contains(&(std::path::PathBuf::from(&project), id.clone()))
        {
            return None;
        }
        Some(palette::ConversationItem {
            title: conversation_title(value, &id, id == self.conversation),
            project,
            id,
        })
    }

    fn append_project_conversations(
        &self,
        items: &mut Vec<palette::ConversationItem>,
        include_active: bool,
    ) {
        for (index, project) in self.projects.iter().enumerate() {
            if index == self.active_project && !include_active {
                continue;
            }
            for conversation in &project.conversations {
                let Some(id) = conversation["id"].as_str().filter(|id| !id.is_empty()) else {
                    continue;
                };
                if self
                    .deleted_conversations
                    .contains(&(project.path.clone(), id.to_owned()))
                {
                    continue;
                }
                if items
                    .iter()
                    .any(|item| item.id == id && item.project == project.path.to_string_lossy())
                {
                    continue;
                }
                items.push(palette::ConversationItem {
                    title: conversation_title(conversation, id, id == self.conversation),
                    project: project.path.to_string_lossy().into_owned(),
                    id: id.to_owned(),
                });
            }
        }
    }

    fn append_saved_conversations(&self, items: &mut Vec<palette::ConversationItem>) {
        for id in self.conversation_states.keys() {
            if !items.iter().any(|item| &item.id == id) {
                let mark = if id == &self.conversation {
                    " (current)"
                } else {
                    ""
                };
                items.push(palette::ConversationItem {
                    id: id.clone(),
                    project: self
                        .conversation_projects
                        .get(id)
                        .map(|path| path.to_string_lossy().into_owned())
                        .or_else(|| {
                            self.projects
                                .get(self.active_project)
                                .map(|project| project.path.to_string_lossy().into_owned())
                        })
                        .unwrap_or_else(|| self.project.clone()),
                    title: format!("{id}{mark} · saved state"),
                });
            }
        }
    }

    fn ensure_active_conversation(&self, items: &mut Vec<palette::ConversationItem>) {
        if !self.conversation.is_empty() && !items.iter().any(|item| item.id == self.conversation) {
            items.insert(
                0,
                palette::ConversationItem {
                    id: self.conversation.clone(),
                    project: self
                        .projects
                        .get(self.active_project)
                        .map(|project| project.path.to_string_lossy().into_owned())
                        .unwrap_or_else(|| self.project.clone()),
                    title: format!("{} (current) · active", self.conversation),
                },
            );
        }
    }

    pub(in crate::lean::tui) fn set_comment(&self, id: u64, status: &str) {
        self.request(
            Request::Conversation(crate::lean::daemon::ConversationRequest::CommentStatus {
                conversation: self.conversation.clone(),
                id,
                status: status.into(),
            }),
            None,
        );
    }

    pub(in crate::lean::tui) fn selection_context(&self) -> String {
        use std::hash::{Hash, Hasher};
        let details = self.details.as_ref().map(|details| {
            let mut query = std::collections::hash_map::DefaultHasher::new();
            details.query.hash(&mut query);
            (&details.key, details.content_tab(), query.finish())
        });
        format!(
            "{}:{}:{}:{:?}:{:?}",
            self.conversation,
            self.generation,
            self.view.after,
            details,
            self.plan_view.as_ref().map(|p| &p.plan_id)
        )
    }

    pub(in crate::lean::tui) fn selection_blocked(&self) -> bool {
        if self.plan_history.is_some() {
            return true;
        }
        if self.interview_visible() {
            return true;
        }

        self.help.is_some()
            || self.picker.is_some()
            || self.editor.is_some()
            || self.flow_editor.is_some()
            || self.diff.is_some()
            || self.unblock.is_some()
            || self.config_editor.as_ref().is_some_and(|e| e.visible)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_picker_uses_goal_and_a_clear_empty_goal_fallback() {
        let named = conversation_title(
            &serde_json::json!({"acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"  Add   session deletion ","status":"done"}),
            "session-id",
            false,
        );
        assert_eq!(named, "Add session deletion · done");
        assert_eq!(
            conversation_title(&serde_json::json!({"status":"idle"}), "session-id", false),
            "New session · session-id · idle"
        );
        assert_eq!(
            conversation_title(
                &serde_json::json!({"goal":"Old session","status":"failed","status_error":"session branch is missing"}),
                "session-id",
                false,
            ),
            "Old session · failed: session branch is missing"
        );
    }
}
