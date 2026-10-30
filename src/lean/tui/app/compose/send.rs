use super::*;

pub(super) fn is_rejection_target(target: &str) -> bool {
    target.starts_with("reject:") || target.starts_with("unblock:")
}

pub(super) fn comment_text(editor: &Editor, view: &view::View) -> String {
    let quote = editor
        .target
        .rsplit_once(":line:")
        .and_then(|(_, line)| line.parse::<usize>().ok())
        .and_then(|line| {
            PlanView::new(String::new())
                .build_lines(view)
                .into_iter()
                .find(|row| row.number == line)
                .map(|row| row.text)
        })
        .map(|text| text.trim().chars().take(200).collect::<String>())
        .filter(|text| !text.is_empty());
    match quote {
        Some(quote) => format!("> {quote}\n{}", editor.input.text),
        None => editor.input.text.clone(),
    }
}

pub(super) struct CommentSend {
    pub(super) client: Client,
    pub(super) updates: std::sync::mpsc::Sender<Update>,
    pub(super) conversation: String,
    pub(super) plan: Option<String>,
    pub(super) generation: u64,
    pub(super) request_id: u64,
    pub(super) editor: Editor,
    pub(super) round: Option<u32>,
    pub(super) run: Option<String>,
    pub(super) text: String,
}

impl CommentSend {
    pub(super) fn spawn(self) {
        std::thread::spawn(move || self.complete());
    }

    fn complete(self) {
        let result = self
            .execute()
            .map(|_| ())
            .map_err(|error| error.to_string());
        let _ = self.updates.send(Update::CommentResult {
            conversation: self.conversation,
            plan: self.plan,
            generation: self.generation,
            request_id: self.request_id,
            editor: self.editor,
            result,
        });
    }

    fn execute(&self) -> Result<Value> {
        if let Some(rest) = self.editor.target.strip_prefix("unblock:") {
            self.execute_unblock(rest)
        } else if let Some(plan) = self.editor.target.strip_prefix("reject:") {
            self.execute_rejection(plan)
        } else {
            self.execute_comment()
        }
    }

    fn execute_unblock(&self, rest: &str) -> Result<Value> {
        let mut parts = rest.splitn(3, ':');
        let action = parts.next().unwrap_or_default();
        let plan = parts.next().unwrap_or_default();
        let task = parts.next().unwrap_or_default();
        let message = Some(self.text.trim().to_owned()).filter(|text| !text.is_empty());
        serde_json::from_value::<RecoveryAction>(serde_json::json!(action))
            .map_err(|error| crate::Error::Invalid(error.to_string()))
            .and_then(|action| {
                recover(
                    &self.client,
                    &self.conversation,
                    plan,
                    task,
                    action,
                    message,
                )
                .map(|notice| serde_json::json!(notice))
                .map_err(crate::Error::Invalid)
            })
    }

    fn execute_rejection(&self, plan: &str) -> Result<Value> {
        let reason = Some(self.text.trim().to_owned()).filter(|text| !text.is_empty());
        self.client.call(&Request::Reject {
            conversation: self.conversation.clone(),
            plan: plan.to_owned(),
            reason,
        })
    }

    fn execute_comment(&self) -> Result<Value> {
        let target = self.editor.target.clone();
        if self.round.is_none() && self.run.is_none() {
            self.client.call(&Request::CommentScoped {
                conversation: self.conversation.clone(),
                plan: self.plan.clone(),
                target,
                text: self.text.clone(),
            })
        } else {
            self.client.call(&Request::CommentAt {
                conversation: self.conversation.clone(),
                plan: self.plan.clone(),
                target,
                text: self.text.clone(),
                round: self.round,
                run: self.run.clone(),
            })
        }
    }
}

pub(super) struct AgentMessageTarget {
    pub(super) conversation: String,
    pub(super) recipient: String,
    pub(super) addressed: String,
    pub(super) plan: Option<String>,
    pub(super) expected_run: Option<String>,
    pub(super) expected_round: Option<u32>,
}

pub(super) struct AgentMessageSend {
    pub(super) client: Client,
    pub(super) updates: std::sync::mpsc::Sender<Update>,
    pub(super) target: AgentMessageTarget,
    pub(super) operation: String,
    pub(super) text: String,
}

impl AgentMessageSend {
    pub(super) fn spawn(self) {
        std::thread::spawn(move || self.send());
    }

    fn send(self) {
        let result = self
            .client
            .call(&Request::SendToAgent {
                message: crate::lean::store::AgentMessage {
                    conversation: self.target.conversation.clone(),
                    plan: self.target.plan,
                    recipient: self.target.addressed,
                    operation_id: self.operation.clone(),
                    expected_round: self.target.expected_round,
                    expected_run: self.target.expected_run,
                    text: self.text.clone(),
                },
            })
            .map_err(|error| error.to_string());
        let _ = self.updates.send(Update::AgentMessage {
            conversation: self.target.conversation,
            recipient: self.target.recipient,
            operation: self.operation,
            text: self.text,
            result,
        });
    }
}
