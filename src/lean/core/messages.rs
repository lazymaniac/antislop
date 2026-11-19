use super::*;

impl Core {
    pub fn submit_addressed(
        self: &Arc<Self>,
        conversation: &str,
        text: &str,
        explicit_to: Option<&str>,
    ) -> Result<serde_json::Value> {
        self.store.conversation(conversation)?;
        if explicit_to.is_none()
            && let Some(direct_prompt) = text.trim().strip_prefix("/direct").and_then(|rest| {
                if rest.is_empty() || rest.starts_with(char::is_whitespace) {
                    Some(rest.trim())
                } else {
                    None
                }
            })
        {
            return self.direct_execute(conversation, direct_prompt);
        }
        let Some((recipient, body)) = address(text, explicit_to)? else {
            return Ok(json!(self.send(conversation, text)?.seq));
        };
        require(!body.is_empty(), "the message is empty")?;
        dispatch_addressed(self, conversation, &recipient, &body)
    }

    /// Zero-ceremony 1-task execution (`/direct <prompt>`): bypasses the
    /// orchestrator interview and plan review committee, creates a single
    /// implementation task in an isolated worktree, and starts it immediately.
    pub fn direct_execute(
        self: &Arc<Self>,
        conversation: &str,
        prompt: &str,
    ) -> Result<serde_json::Value> {
        let prompt = prompt.trim();
        require(!prompt.is_empty(), "usage: /direct <instruction>")?;
        let _operation = self.operation(conversation)?;
        let source = self.locks.source(conversation);
        let _source = source.lock().expect("conversation source scope");
        require(
            !self.activity.plan_running(conversation),
            "a plan is already running",
        )?;
        let _lifecycle = self
            .locks
            .session_lifecycle
            .lock()
            .expect("session lifecycle mutex");
        let message = self.store.add_message(conversation, "user", prompt)?;
        let turn =
            self.store
                .start_turn(conversation, ORCHESTRATOR, "direct", message.seq, None)?;
        self.store
            .finish_turn(&turn.id, store::TurnStatus::Done, None)?;
        self.store.add_message(
            conversation,
            ORCHESTRATOR,
            "Starting direct execution in an isolated worktree.",
        )?;
        self.store.db().execute(
            "UPDATE session_workflow SET phase='planning', summary=?2, upto_message=?3 WHERE conversation_id=?1",
            rusqlite::params![conversation, prompt, message.seq as i64],
        )?;
        self.store.append_event(
            conversation,
            None,
            None,
            "workflow",
            &json!({"phase": "planning", "summary": prompt, "upto_message": message.seq}),
        )?;

        let scope = self.store.create_scope(conversation, prompt)?;
        let first_line = prompt.lines().next().unwrap_or(prompt).trim();
        let title: String = unicode_segmentation::UnicodeSegmentation::graphemes(first_line, true)
            .take(64)
            .collect();
        let existing_count = self.store.latest_plan(conversation)?.len();
        let task_id = if existing_count == 0 {
            "direct-1".to_owned()
        } else {
            format!("direct-{}", existing_count + 1)
        };
        let spec = TaskContract {
            quality_gate: false,
            integration_batch: false,
            options_for: None,
            id: task_id.clone(),

            acceptance_criteria: vec![tasks::AcceptanceCriterion {
                id: "C1".into(),
                text: prompt.into(),
            }],
            title: if title.is_empty() {
                "Direct task".into()
            } else {
                title
            },
            goal: prompt.to_owned(),
            summary: None,
            section: None,
            deferred: false,
            change_kind: tasks::ChangeKind::Code,
            outputs: Vec::new(),
            delivery: None,
            depends_on: Vec::new(),
            kind: WorkKind::Implementation,

            reviewer: tasks::ReviewerSpec {
                summary: Some("Verify that the direct change satisfies the request.".into()),
            },
        };
        let specs = vec![spec];
        let values = specs
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let added = self.store.add_plan(conversation, &values)?;
        let plan_id = added[0].plan_id.clone();
        self.store
            .bind_plan_scope(conversation, &plan_id, &scope.id)?;
        self.store.append_event(
            conversation,
            None,
            Some(&plan_id),
            "plan_review",
            &json!({"plan": plan_id, "approved": true, "findings": []}),
        )?;
        let revision_token = self.plan_revision_token(conversation, &plan_id)?;
        self.store.append_event(
            conversation,
            None,
            Some(&plan_id),
            "plan_proposed",
            &json!({
                "plan": plan_id,
                "document": tasks::render_plan_document(prompt, &specs),
                "parallel": [],
                "tasks": added.iter().map(|t| t.spec.clone()).collect::<Vec<_>>(),
                "review": {"approved": true, "findings": []},
                "automatic": true,
                "waiting": serde_json::Value::Null,
                "revision_token": revision_token,
            }),
        )?;
        drop(_lifecycle);
        let _ = self.activate_reviewed_plan(conversation, &plan_id)?;
        Ok(json!({
            "seq": message.seq,
            "direct_plan": plan_id,
            "task": task_id,
        }))
    }
}

fn address(text: &str, explicit: Option<&str>) -> Result<Option<(String, String)>> {
    let Some(explicit) = explicit else {
        if let Some((recipient, body)) = text
            .strip_prefix('@')
            .and_then(|r| r.split_once(char::is_whitespace))
        {
            return Ok(Some((recipient.trim().into(), body.trim().into())));
        }
        require(!text.starts_with('@'), "the message is empty")?;
        return Ok(None);
    };
    Ok(Some((
        explicit.trim().trim_start_matches('@').into(),
        text.trim().into(),
    )))
}

fn dispatch_addressed(
    core: &Arc<Core>,
    conversation: &str,
    recipient: &str,
    body: &str,
) -> Result<serde_json::Value> {
    if recipient == ORCHESTRATOR {
        return Ok(json!(core.send(conversation, body)?.seq));
    }
    let plan = core.store.latest_plan(conversation)?;
    dispatch_to_plan(core, conversation, recipient, body, &plan)
}

fn dispatch_to_plan(
    core: &Arc<Core>,
    conversation: &str,
    recipient: &str,
    body: &str,
    plan: &[Task],
) -> Result<serde_json::Value> {
    if recipient == "all" {
        return dispatch_broadcast(core, conversation, body, plan);
    }
    if matches!(recipient, PLAN_REVIEWER | AUDITOR) {
        return dispatch_reviewer(core, conversation, recipient, body, plan);
    }
    let Some((task, role)) = core.mail_address_task(conversation, plan, recipient)? else {
        return reject_recipient(core, conversation, recipient, body);
    };
    direct_task(core, conversation, &task, &role, body)
}

fn dispatch_broadcast(
    core: &Arc<Core>,
    conversation: &str,
    body: &str,
    plan: &[Task],
) -> Result<serde_json::Value> {
    if plan.is_empty() {
        core.store.append_event(conversation,None,None,"mail_rejected",&json!({"sender":"user","recipient":"all","text":body,"reason":"no valid recipients for broadcast in current plan"}))?;
        return Err(Error::Invalid(
            "no valid recipients for broadcast in current plan".into(),
        ));
    }
    let receipts = plan
        .iter()
        .map(|task| {
            direct_task(
                core,
                conversation,
                task,
                &format!("worker:{}", task.id),
                body,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(
        json!({"mail_ids":receipts.iter().map(|receipt|&receipt["mail"]).collect::<Vec<_>>(),"state":"queued","recipient":"all","recipient_finished":false}),
    )
}

fn dispatch_reviewer(
    core: &Arc<Core>,
    conversation: &str,
    recipient: &str,
    body: &str,
    plan: &[Task],
) -> Result<serde_json::Value> {
    let run = core
        .store
        .last_turn(conversation, recipient)?
        .map(|turn| turn.id);
    core.send_to_agent(&store::AgentMessage {
        conversation: conversation.into(),
        plan: plan.first().map(|task| task.plan_id.clone()),
        recipient: recipient.into(),
        operation_id: crate::id(),
        expected_round: None,
        expected_run: run,
        delivery: Default::default(),
        text: body.into(),
    })
}

fn direct_task(
    core: &Arc<Core>,
    conversation: &str,
    task: &Task,
    role: &str,
    body: &str,
) -> Result<serde_json::Value> {
    let mut receipt = core.send_to_agent(&store::AgentMessage {
        conversation: conversation.into(),
        plan: Some(task.plan_id.clone()),
        recipient: role.into(),
        operation_id: crate::id(),
        expected_round: Some(task.round),
        expected_run: None,
        delivery: Default::default(),
        text: body.into(),
    })?;
    receipt["recipient_finished"] = json!(status::finished(&task.status));
    Ok(receipt)
}

fn reject_recipient(
    core: &Core,
    conversation: &str,
    recipient: &str,
    body: &str,
) -> Result<serde_json::Value> {
    let reason = format!("unknown recipient or role: {recipient}");
    core.store.append_event(
        conversation,
        None,
        None,
        "mail_rejected",
        &json!({"sender":"user","recipient":recipient,"text":body,"reason":reason}),
    )?;
    Err(Error::Invalid(reason))
}
