use super::super::*;

fn conversation_title(value: &Value, id: &str, current: bool) -> String {
    let status = value["status"].as_str().unwrap_or("idle");
    let mark = if current { " (current)" } else { "" };
    let total = value["tasks_total"].as_u64().unwrap_or(0);
    if total == 0 {
        format!("{id}{mark} · {status}")
    } else {
        let done = value["tasks_done"].as_u64().unwrap_or(0);
        format!("{id}{mark} · {status} · {done}/{total} tasks")
    }
}

impl App {
    pub fn open_conversation_picker(&mut self) {
        let mut items = self.daemon_conversations();
        self.append_project_conversations(&mut items);
        self.append_saved_conversations(&mut items);
        self.ensure_active_conversation(&mut items);
        let mut picker = Picker::new(Kind::Conversations);
        picker.conversations = items;
        self.picker = Some(picker);
    }

    fn daemon_conversations(&self) -> Vec<palette::ConversationItem> {
        let Ok(Value::Array(list)) = self.client.call(&Request::Conversations) else {
            return Vec::new();
        };
        list.iter()
            .filter_map(|value| self.daemon_conversation(value))
            .collect()
    }

    fn daemon_conversation(&self, value: &Value) -> Option<palette::ConversationItem> {
        let project = value["project"].as_str().unwrap_or_default();
        let basename = std::path::Path::new(project)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(project);
        if !project.is_empty() && project != self.project && basename != self.project {
            return None;
        }
        let id = value["id"].as_str()?.to_owned();
        if id.is_empty() {
            return None;
        }
        Some(palette::ConversationItem {
            title: conversation_title(value, &id, id == self.conversation),
            project: project.to_owned(),
            id,
        })
    }

    fn append_project_conversations(&self, items: &mut Vec<palette::ConversationItem>) {
        for (index, project) in self.projects.iter().enumerate() {
            if index == self.active_project {
                continue;
            }
            for conversation in &project.conversations {
                let Some(id) = conversation["id"].as_str().filter(|id| !id.is_empty()) else {
                    continue;
                };
                let status = conversation["status"].as_str().unwrap_or("idle");
                items.push(palette::ConversationItem {
                    title: format!("{} · {id} · {status}", project.name),
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
                    project: self.project.clone(),
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
                    project: self.project.clone(),
                    title: format!("{} (current) · active", self.conversation),
                },
            );
        }
    }

    pub(in crate::lean::tui) fn set_comment(&self, id: u64, status: &str) {
        self.request(
            Request::CommentStatus {
                conversation: self.conversation.clone(),
                id,
                status: status.into(),
            },
            None,
        );
    }

    pub(in crate::lean::tui) fn selection_context(&self) -> String {
        use std::hash::{Hash, Hasher};
        let details = self.details.as_ref().map(|details| {
            let mut query = std::collections::hash_map::DefaultHasher::new();
            details.query.hash(&mut query);
            (&details.key, details.tab, query.finish())
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
        if self.ledger_view.as_ref().is_some_and(|v| v.visible) {
            return true;
        }
        self.help.is_some()
            || self.picker.is_some()
            || self.editor.is_some()
            || self.diff.is_some()
            || self.unblock.is_some()
            || self.trust_prompt.is_some()
            || self.project_trust.is_some()
            || self
                .confirmation
                .as_ref()
                .is_some_and(|c| c.status == ConfirmationStatus::Pending)
            || self.config_editor.as_ref().is_some_and(|e| e.visible)
    }
}
