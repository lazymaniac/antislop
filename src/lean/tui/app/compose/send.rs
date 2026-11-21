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
    pub(super) updates: UpdateSender,
    pub(super) conversation: String,
    pub(super) operation_id: String,
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
            operation_id: self.operation_id,
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
        self.client.call(&Request::Conversation(
            crate::lean::daemon::ConversationRequest::Reject {
                conversation: self.conversation.clone(),
                operation_id: Some(self.operation_id.clone()),
                plan: plan.to_owned(),
                reason,
                revision_token: None,
            },
        ))
    }

    fn execute_comment(&self) -> Result<Value> {
        let target = self.editor.target.clone();
        if self.round.is_none() && self.run.is_none() {
            self.client.call(&Request::Conversation(
                crate::lean::daemon::ConversationRequest::Comment {
                    conversation: self.conversation.clone(),
                    operation_id: Some(self.operation_id.clone()),
                    plan: self.plan.clone(),
                    target,
                    text: self.text.clone(),
                    round: None,
                    run: None,
                },
            ))
        } else {
            self.client.call(&Request::Conversation(
                crate::lean::daemon::ConversationRequest::Comment {
                    conversation: self.conversation.clone(),
                    operation_id: Some(self.operation_id.clone()),
                    plan: self.plan.clone(),
                    target,
                    text: self.text.clone(),
                    round: self.round,
                    run: self.run.clone(),
                },
            ))
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
    pub(super) delivery: crate::lean::store::Delivery,
}

pub(super) struct AgentMessageSend {
    pub(super) client: Client,
    pub(super) updates: UpdateSender,
    pub(super) target: AgentMessageTarget,
    pub(super) operation: String,
    pub(super) text: String,
    pub(super) broadcast: Option<Vec<crate::lean::store::AgentMessage>>,
}

impl AgentMessageSend {
    pub(super) fn spawn(self) {
        std::thread::spawn(move || self.send());
    }

    fn send(self) {
        let result = if let Some(messages) = self.broadcast {
            send_addressed_deliveries(&self.client, messages)
        } else if self.target.recipient == "all"
            && self.target.delivery == crate::lean::store::Delivery::Queue
        {
            self.client
                .call(&Request::Conversation(
                    crate::lean::daemon::ConversationRequest::Send {
                        conversation: self.target.conversation.clone(),
                        operation_id: Some(self.operation.clone()),
                        text: self.text.clone(),
                        to: Some("all".into()),
                        images: Vec::new(),
                    },
                ))
                .map_err(|error| error.to_string())
        } else {
            self.client
                .call(&Request::Conversation(
                    crate::lean::daemon::ConversationRequest::SendToAgent {
                        message: crate::lean::store::AgentMessage {
                            conversation: self.target.conversation.clone(),
                            plan: self.target.plan,
                            recipient: self.target.addressed,
                            operation_id: self.operation.clone(),
                            expected_round: self.target.expected_round,
                            expected_run: self.target.expected_run,
                            delivery: self.target.delivery,
                            text: self.text.clone(),
                        },
                    },
                ))
                .map_err(|error| error.to_string())
        };
        let _ = self.updates.send(Update::AgentMessage {
            conversation: self.target.conversation,
            recipient: self.target.recipient,
            operation: self.operation,
            text: self.text,
            result,
        });
    }
}

pub(super) fn addressed_deliveries(
    view: &View,
    conversation: &str,
    operation: &str,
    text: &str,
    delivery: crate::lean::store::Delivery,
) -> std::result::Result<Vec<crate::lean::store::AgentMessage>, String> {
    let (recipient, body) = text
        .strip_prefix('@')
        .and_then(|rest| rest.split_once(char::is_whitespace))
        .unwrap_or((ORCHESTRATOR, text));
    let recipients = if recipient == "all" {
        view.tasks
            .iter()
            .map(|task| format!("worker:{}", task.id))
            .collect::<Vec<_>>()
    } else {
        vec![recipient.to_owned()]
    };
    if recipients.is_empty() {
        return Err("no valid recipients for broadcast in current plan".into());
    }
    recipients
        .into_iter()
        .map(|recipient| {
            let task_role = crate::lean::task_role(&recipient).or_else(|| {
                view.tasks
                    .iter()
                    .any(|task| task.id == recipient)
                    .then_some((recipient.as_str(), false))
            });
            let (address, round) = if let Some((id, reviewer)) = task_role {
                let task = view
                    .tasks
                    .iter()
                    .find(|task| task.id == id || task.uuid.as_deref() == Some(id))
                    .ok_or_else(|| format!("Task {id} is not in the current plan"))?;
                let uuid = task
                    .uuid
                    .as_deref()
                    .ok_or_else(|| format!("Task {id} has no runtime identity yet"))?;
                (
                    format!("{}:{uuid}", if reviewer { "reviewer" } else { "worker" }),
                    Some(task.round as u32),
                )
            } else {
                (recipient.clone(), None)
            };
            Ok(crate::lean::store::AgentMessage {
                conversation: conversation.into(),
                plan: view.plan.clone(),
                recipient: address,
                operation_id: format!("{operation}:{recipient}"),
                expected_round: round,
                expected_run: None,
                delivery,
                text: body.into(),
            })
        })
        .collect()
}

pub(super) fn send_addressed_deliveries(
    client: &Client,
    messages: Vec<crate::lean::store::AgentMessage>,
) -> std::result::Result<Value, String> {
    let mut sessions = None;
    let mut receipts = Vec::new();
    let mut steered = false;
    for mut message in messages {
        if !message.recipient.contains(':') {
            if sessions.is_none() {
                let reply = client
                    .call(&Request::Session(
                        crate::lean::daemon::SessionRequest::Sessions {
                            conversation: message.conversation.clone(),
                        },
                    ))
                    .map_err(|error| error.to_string())?;
                sessions = Some(
                    serde_json::from_value::<Vec<crate::lean::store::SessionInfo>>(reply)
                        .map_err(|error| error.to_string())?,
                );
            }
            message.expected_run = sessions
                .as_ref()
                .unwrap()
                .iter()
                .rfind(|session| session.role == message.recipient)
                .map(|session| session.turn_id.clone());
        }
        let receipt = client
            .call(&Request::Conversation(
                crate::lean::daemon::ConversationRequest::SendToAgent { message },
            ))
            .map_err(|error| error.to_string())?;
        steered |= receipt["state"] == "steered";
        receipts.extend(receipt["mail"].as_u64());
    }
    Ok(
        serde_json::json!({"mail_ids":receipts,"recipient":"all","state":if steered { "steered" } else { "queued" }}),
    )
}
