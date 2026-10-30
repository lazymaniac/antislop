use super::super::super::*;
use crate::lean::RecoveryAction;

impl App {
    /// Opens actions for the named or selected step, falling back to a
    /// stopped step or an active task that can be restarted.
    pub fn open_unblock(&mut self, named: Option<String>) {
        if self.targets_orchestrator(&named) {
            self.unblock = Some(unblock::Unblock::orchestrator());
            return;
        }
        let Some(plan) = self.view.plan.clone() else {
            return;
        };
        match self.unblock_task(named) {
            Some(task) => self.unblock = Some(unblock::Unblock::new(&plan, task)),
            None => self.notice = Some("✗ no step is stopped; nothing to unblock".into()),
        }
    }

    fn targets_orchestrator(&self, named: &Option<String>) -> bool {
        named.as_deref() == Some(ORCHESTRATOR)
            || (named.is_none() && self.selected == ORCHESTRATOR)
            || self.view.plan.is_none()
    }

    fn unblock_task(&self, named: Option<String>) -> Option<&view::TaskRow> {
        let selected = self.selected.split_once(':').map(|(_, id)| id.to_owned());
        let wanted = named.or(selected);
        self.named_unblock_task(wanted.as_deref())
            .or_else(|| {
                self.view
                    .tasks
                    .iter()
                    .find(|task| unblock::is_blocked(&task.status) && task.status != "skipped")
            })
            .or_else(|| {
                self.view
                    .tasks
                    .iter()
                    .find(|task| unblock::is_blocked(&task.status))
            })
            .or_else(|| {
                self.view
                    .tasks
                    .iter()
                    .find(|task| matches!(task.status.as_str(), "working" | "verifying" | "queued"))
            })
    }

    fn named_unblock_task(&self, id: Option<&str>) -> Option<&view::TaskRow> {
        id.and_then(|id| self.view.tasks.iter().find(|task| task.id == id))
            .filter(|task| {
                unblock::is_blocked(&task.status)
                    || matches!(task.status.as_str(), "queued" | "working" | "verifying")
            })
    }

    /// The user chose a way out: ask for a hint first where it takes one,
    /// else ask the daemon at once.
    pub(in crate::lean::tui) fn unblock_choice(
        &mut self,
        state: unblock::Unblock,
        choice: unblock::Choice,
    ) {
        if choice.asks_text {
            self.open_unblock_note(&state, &choice);
        } else {
            self.start_unblock_recovery(state, choice);
        }
    }

    fn open_unblock_note(&mut self, state: &unblock::Unblock, choice: &unblock::Choice) {
        let action = serde_json::to_value(&choice.action)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default();
        let target = format!("unblock:{action}:{}:{}", state.plan, state.task);
        let input = self.get_comment_draft(&target);
        self.editor = Some(Editor { target, input });
    }

    fn start_unblock_recovery(&self, state: unblock::Unblock, choice: unblock::Choice) {
        let client = self.client.clone();
        let updates = self.updates.clone();
        let conversation = self.conversation.clone();
        std::thread::spawn(move || {
            let notice = unblock_result(&client, &conversation, &state, choice.action);
            let _ = updates.send(Update::Notice(notice));
        });
    }
}

fn unblock_result(
    client: &Client,
    conversation: &str,
    state: &unblock::Unblock,
    action: RecoveryAction,
) -> String {
    match recover(client, conversation, &state.plan, &state.task, action, None) {
        Ok(notice) => format!("{}: {notice}", state.task),
        Err(error) => format!("✗ {error}"),
    }
}
