use super::super::super::*;
use crate::lean::RecoveryAction;

impl App {
    /// Opens actions for the named or selected step, falling back to a
    /// stopped step or an active task that can be restarted.
    pub fn open_unblock(&mut self, named: Option<String>) {
        let named = named.map(|key| self.view.agent_key(&key)).map(|key| {
            crate::lean::task_role(&key).map_or_else(|| key.clone(), |(id, _)| id.to_owned())
        });
        if self.view.audit_retryable()
            && (named.is_none() || named.as_deref() == Some(view::AUDITOR))
        {
            self.unblock = Some(unblock::Unblock::auditor(
                self.view.plan.as_deref().unwrap(),
                self.view.audit_error.as_deref().unwrap(),
            ));
            return;
        }
        if named.as_deref() == Some(view::AUDITOR)
            || (named.is_none() && self.selected == view::AUDITOR)
        {
            self.notice =
                Some("The final audit is running or has no interrupted result to retry.".into());
            return;
        }
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
        matches!(
            named.as_deref(),
            Some(ORCHESTRATOR | crate::lean::PLANNER | crate::lean::COMMENT_ORCHESTRATOR)
        ) || (named.is_none() && self.selected == ORCHESTRATOR)
            || self.view.plan.is_none()
    }

    fn unblock_task(&self, named: Option<String>) -> Option<&view::TaskRow> {
        let selected = named
            .is_none()
            .then(|| {
                self.details
                    .as_ref()
                    .filter(|_| self.navigation.screen == crate::lean::tui::Screen::Workbench)
                    .and_then(|details| details.task_id().map(str::to_owned))
                    .or_else(|| self.selected.split_once(':').map(|(_, id)| id.to_owned()))
            })
            .flatten();
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
                    .find(|task| matches!(task.status.as_str(), "working" | "reviewing" | "queued"))
            })
    }

    fn named_unblock_task(&self, id: Option<&str>) -> Option<&view::TaskRow> {
        id.and_then(|id| self.view.tasks.iter().find(|task| task.id == id))
            .filter(|task| {
                unblock::is_blocked(&task.status)
                    || matches!(task.status.as_str(), "queued" | "working" | "reviewing")
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
        if choice.action == RecoveryAction::IntegrateWithConsent {
            self.prepare_recovery_consent(state.plan, state.task);
            return;
        }
        let client = self.client.clone();
        let updates = self.updates.clone();
        let conversation = self.conversation.clone();
        std::thread::spawn(move || {
            let notice = unblock_result(&client, &conversation, &state, choice.action);
            let _ = updates.send(Update::Notice(notice));
        });
    }

    fn prepare_recovery_consent(&self, plan: String, task: String) {
        let client = self.client.clone();
        let updates = self.updates.clone();
        let conversation = self.conversation.clone();
        let generation = self.generation;
        std::thread::spawn(move || {
            let result = load_recovery_consent(&client, &conversation, &plan, &task);
            let _ = updates.send(Update::RecoveryConsent {
                conversation,
                generation,
                plan,
                task,
                result,
            });
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

fn load_recovery_consent(
    client: &Client,
    conversation: &str,
    plan: &str,
    task: &str,
) -> std::result::Result<consent::RecoveryConsent, String> {
    use crate::lean::daemon::ConversationRequest;
    let task_data = client
        .call(&Request::Conversation(ConversationRequest::Task {
            conversation: Some(conversation.to_owned()),
            plan: plan.to_owned(),
            id: task.to_owned(),
        }))
        .map_err(|error| error.to_string())?;
    let status = task_data["status"]
        .as_str()
        .ok_or("task response has no status")?;
    let result = task_data["result"]
        .as_str()
        .ok_or("task has no retained result snapshot")?;
    let round = task_data["round"]
        .as_u64()
        .ok_or("task response has no review round")?;
    let verdict = &task_data["verdict"];
    if verdict["approved"] != true || verdict["result"] != result || verdict["round"] != round {
        return Err(format!(
            "integration unavailable: no positive independent review for current snapshot {result} at round {round}"
        ));
    }
    if status != "needs_user" {
        return Err(format!(
            "integration unavailable for current task status '{status}'"
        ));
    }
    let diff = client
        .call(&Request::Conversation(ConversationRequest::Diff {
            conversation: conversation.to_owned(),
            plan: plan.to_owned(),
            id: task.to_owned(),
        }))
        .map_err(|error| error.to_string())?;
    let files = diff["changed_files"]
        .as_array()
        .ok_or("diff response has no complete changed_files list")?
        .iter()
        .map(|file| {
            file.as_str()
                .map(str::to_owned)
                .ok_or("diff contains a non-text path")
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if files.is_empty() {
        return Err("integration unavailable: reviewed snapshot contains no changed files".into());
    }
    let manual = task_data["feedback"]["manual_snapshot"]["result"] == result
        && task_data["feedback"]["manual_snapshot"]["round"] == round;
    let target = if manual || task_data["spec"]["delivery"] == "untracked" {
        "project working tree"
    } else {
        "session result worktree"
    };
    Ok(consent::RecoveryConsent {
        plan: plan.to_owned(),
        task: task.to_owned(),
        status: status.to_owned(),
        target: target.into(),
        base: task_data["base"]
            .as_str()
            .ok_or("task has no result base")?
            .to_owned(),
        result: result.to_owned(),
        token: task_data["freshness_token"]
            .as_str()
            .ok_or("task response has no freshness token")?
            .to_owned(),
        require_all: task_data["spec"]["delivery"] == "untracked",
        files: files
            .into_iter()
            .map(|file| (file, task_data["spec"]["delivery"] == "untracked"))
            .collect(),
        selected: 0,
        scroll: 0,
        confirming: false,
        error: None,
    })
}
