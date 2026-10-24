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
    if matches!(recipient, PLAN_VERIFIER | AUDITOR | COMMENT_ORCHESTRATOR) {
        return queue_directive(core, conversation, recipient, body);
    }
    if recipient == "all" {
        return broadcast(core, conversation, body);
    }
    deliver_to_task(core, conversation, recipient, body)
}

fn queue_directive(
    core: &Arc<Core>,
    conversation: &str,
    recipient: &str,
    body: &str,
) -> Result<serde_json::Value> {
    let mail = core.store.add_mail(
        conversation,
        recipient,
        &queued_mail("user", recipient, "user_instruction", body),
    )?;
    queue_orchestrator_copy(
        core,
        conversation,
        &format!("(the user wrote to {recipient}) {body}"),
    )?;
    core.answer_in_background(conversation, "");
    Ok(json!({"mail":mail.id,"state":"queued","recipient":recipient,"recipient_finished":false}))
}

fn broadcast(core: &Arc<Core>, conversation: &str, body: &str) -> Result<serde_json::Value> {
    let plan = core.store.latest_plan(conversation)?;
    if plan.is_empty() {
        return reject_broadcast(core, conversation, body, &plan);
    }
    let mail_ids = queue_broadcast_targets(core, conversation, &plan, body)?;
    queue_orchestrator_copy(
        core,
        conversation,
        &format!("(the user broadcast to all) {body}"),
    )?;
    if let Some(task) = plan.first() {
        core.answer_in_background(conversation, &task.plan_id);
    }
    Ok(
        json!({"mail":mail_ids.first().copied().unwrap_or(0),"mail_ids":mail_ids,"state":"queued","recipient":"all","recipient_finished":false}),
    )
}

fn reject_broadcast(
    core: &Core,
    conversation: &str,
    body: &str,
    plan: &[Task],
) -> Result<serde_json::Value> {
    core.store.append_event(conversation, None, plan.first().map(|t| t.plan_id.as_str()), "mail_rejected", &json!({"sender":"user","recipient":"all","text":body,"reason":"no valid recipients for broadcast in current plan"}))?;
    Err(crate::Error::Invalid(
        "no valid recipients for broadcast in current plan".into(),
    ))
}

fn queue_broadcast_targets(
    core: &Core,
    conversation: &str,
    plan: &[Task],
    body: &str,
) -> Result<Vec<u64>> {
    plan.iter()
        .map(|task| {
            let recipient = format!("worker:{}", task.id);
            let label = task.spec["id"].as_str().unwrap_or_default();
            Ok(core
                .store
                .add_mail(
                    conversation,
                    &recipient,
                    &queued_mail("user", label, "broadcast", body),
                )?
                .id)
        })
        .collect()
}

fn deliver_to_task(
    core: &Arc<Core>,
    conversation: &str,
    recipient: &str,
    body: &str,
) -> Result<serde_json::Value> {
    let plan = core.store.latest_plan(conversation)?;
    let Some((task, role)) = core.mail_address_task(conversation, &plan, recipient)? else {
        return reject_recipient(core, conversation, recipient, body);
    };
    let mail = core.store.add_mail(
        conversation,
        &role,
        &queued_mail("user", recipient, "user_instruction", body),
    )?;
    queue_orchestrator_copy(
        core,
        conversation,
        &format!("(the user wrote to {recipient}) {body}"),
    )?;
    let finished = status::finished(&task.status);
    if finished {
        core.answer_in_background(conversation, &task.plan_id);
    }
    Ok(json!({"mail":mail.id,"state":"queued","recipient":recipient,"recipient_finished":finished}))
}

fn queued_mail(sender: &str, recipient: &str, kind: &str, text: &str) -> Mail {
    Mail {
        id: 0,
        sender: sender.into(),
        recipient: recipient.into(),
        kind: kind.into(),
        text: text.into(),
        created_ms: 0,
        delivered_turn: None,
        delivered_ms: None,
        state: "queued".into(),
    }
}

fn queue_orchestrator_copy(core: &Core, conversation: &str, text: &str) -> Result<()> {
    core.store.add_mail(
        conversation,
        ORCHESTRATOR,
        &queued_mail("user", ORCHESTRATOR, "copy", text),
    )?;
    Ok(())
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
    Err(crate::Error::Invalid(reason))
}
