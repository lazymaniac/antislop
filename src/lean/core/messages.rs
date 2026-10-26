use super::*;

impl Core {
    pub fn submit_addressed(
        self: &Arc<Self>,
        conversation: &str,
        text: &str,
        explicit_to: Option<&str>,
    ) -> Result<serde_json::Value> {
        self.store.conversation(conversation)?;
        let Some((recipient, body)) = address(text, explicit_to)? else {
            return Ok(json!(self.send(conversation, text)?.seq));
        };
        require(!body.is_empty(), "the message is empty")?;
        dispatch_addressed(self, conversation, &recipient, &body)
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
    if recipient == "all" {
        if plan.is_empty() {
            core.store.append_event(conversation,None,None,"mail_rejected",&json!({"sender":"user","recipient":recipient,"text":body,"reason":"no valid recipients for broadcast in current plan"}))?;
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
        return Ok(
            json!({"mail_ids":receipts.iter().map(|r|&r["mail"]).collect::<Vec<_>>(),"state":"queued","recipient":"all","recipient_finished":false}),
        );
    }
    if matches!(recipient, PLAN_VERIFIER | AUDITOR) {
        let run = core.store.last_turn(conversation, recipient)?.map(|t| t.id);
        return core.send_to_agent(&store::AgentMessage {
            conversation: conversation.into(),
            plan: plan.first().map(|t| t.plan_id.clone()),
            recipient: recipient.into(),
            operation_id: crate::id(),
            expected_round: None,
            expected_run: run,
            text: body.into(),
        });
    }
    let Some((task, role)) = core.mail_address_task(conversation, &plan, recipient)? else {
        return reject_recipient(core, conversation, recipient, body);
    };
    direct_task(core, conversation, &task, &role, body)
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
