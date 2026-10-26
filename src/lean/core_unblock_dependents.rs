use super::*;

impl Core {
    pub(in crate::lean) fn unblock_dependent_tasks(
        &self,
        conversation: &str,
        plan_id: &str,
        approved_task_id: &str,
    ) -> Result<()> {
        let tasks = self.store.plan(plan_id)?;
        for mut task in tasks_to_unblock(&tasks, approved_task_id) {
            task.status = status::QUEUED.to_string();
            // Whatever it showed while blocked no longer stands.
            task.feedback = None;
            self.store.update_task(&task)?;
            self.announce_task(conversation, &task, None);
        }
        Ok(())
    }

    pub(in crate::lean) fn check_comments(
        &self,
        conversation: &str,
        plan: &str,
        spec_id: &str,
    ) -> String {
        let prefix = format!("check:{spec_id}:");
        let comments: Vec<(String, String)> = self
            .store
            .comments(conversation, "open")
            .unwrap_or_default()
            .into_iter()
            .filter(|comment| comment.plan_id.as_deref().is_none_or(|id| id == plan))
            .filter_map(|comment| {
                let id = comment.target.strip_prefix(&prefix)?.to_owned();
                Some((id, comment.text))
            })
            .collect();
        tasks::check_comments_text(&comments)
    }

    pub(in crate::lean) fn notice(&self, conversation: &str, text: &str) {
        let _ = self
            .store
            .append_event(conversation, None, None, "notice", &json!({"text": text}));
    }

    pub(in crate::lean) fn maybe_continue_plan(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
    ) -> Result<()> {
        let tasks = self.store.plan(plan_id)?;
        let has_queued = tasks.iter().any(|t| t.status == status::QUEUED);
        let all_approved = !tasks.is_empty() && tasks.iter().all(|t| t.status == status::APPROVED);
        if has_queued || all_approved {
            let plans = self.plans.lock().expect("plans mutex");
            if !plans.contains_key(conversation) {
                drop(plans);
                self.start_plan_with_scope_lock(conversation, plan_id, |_| Ok(()))?;
            }
        }
        Ok(())
    }

    pub(in crate::lean) fn start_plan(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        queue: impl FnOnce(&Store) -> Result<()>,
    ) -> Result<Vec<Task>> {
        let _integration = self.integrating.lock().expect("integration mutex");
        self.start_plan_with_scope_lock(conversation, plan_id, queue)
    }
}

fn tasks_to_unblock(tasks: &[Task], approved_task_id: &str) -> Vec<Task> {
    let target_spec_id = tasks
        .iter()
        .find(|task| task.id == approved_task_id || task.spec["id"] == approved_task_id)
        .and_then(|task| task.spec["id"].as_str())
        .unwrap_or_default();
    tasks
        .iter()
        .filter(|task| skipped_task_ready(tasks, task, approved_task_id, target_spec_id))
        .cloned()
        .collect()
}

fn skipped_task_ready(tasks: &[Task], task: &Task, approved: &str, target_spec: &str) -> bool {
    if !matches!(task.status.as_str(), status::SKIPPED | status::CANCELLED) {
        return false;
    }
    let dependencies: Vec<String> =
        serde_json::from_value(task.spec["depends_on"].clone()).unwrap_or_default();
    dependencies
        .iter()
        .any(|id| id == target_spec || id == approved)
        && dependencies
            .iter()
            .all(|id| dependency_approved(tasks, id, approved))
}

fn dependency_approved(tasks: &[Task], id: &str, approved: &str) -> bool {
    tasks.iter().any(|task| {
        (task.id == id || task.spec["id"] == id)
            && (task.status == status::APPROVED
                || task.id == approved
                || task.spec["id"] == approved)
    })
}
