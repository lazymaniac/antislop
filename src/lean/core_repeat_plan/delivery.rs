use super::*;

impl Core {
    pub(in crate::lean) fn deliver(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: Option<&str>,
        sender: &str,
        message: tasks::Outgoing,
    ) -> Result<()> {
        let plan = load_delivery_plan(self, conversation, plan_id)?;
        let targets = self.delivery_targets(conversation, &plan, sender, &message.to)?;
        if targets.is_empty() {
            reject_delivery(self, conversation, plan_id, sender, &message)?;
        }
        let context = DeliveryContext {
            conversation,
            plan_id,
            sender,
            plan: &plan,
            message: &message,
        };
        for (label, role) in targets {
            self.deliver_to_target(&context, &label, &role)?;
        }
        self.answer_in_background(conversation, plan_id.unwrap_or(""));
        Ok(())
    }

    fn delivery_targets(
        &self,
        conversation: &str,
        plan: &[Task],
        sender: &str,
        recipient: &str,
    ) -> Result<Vec<(String, String)>> {
        let own_task = sender.split('/').next().unwrap_or(sender);
        match recipient {
            ORCHESTRATOR | COMMENT_ORCHESTRATOR => Ok(vec![(recipient.into(), recipient.into())]),
            PLAN_VERIFIER => Ok(vec![(PLAN_VERIFIER.into(), PLAN_VERIFIER.into())]),
            AUDITOR => Ok(vec![(AUDITOR.into(), AUDITOR.into())]),
            "all" => Ok(all_worker_targets(plan, own_task)),
            to => Ok(self
                .mail_address_task(conversation, plan, to)?
                .map(|(_, role)| vec![(to.into(), role)])
                .unwrap_or_default()),
        }
    }

    fn deliver_to_target(
        &self,
        context: &DeliveryContext<'_>,
        label: &str,
        role: &str,
    ) -> Result<()> {
        let (origin, source_role) = self.delivery_identity(context, role)?;
        let source_turn = self
            .store
            .last_turn(context.conversation, &source_role)?
            .map(|turn| turn.id);
        self.store.add_mail_from(
            context.conversation,
            role,
            &queued_mail(origin, label, context.message),
            source_turn.as_deref(),
        )?;
        Ok(())
    }

    fn delivery_identity(
        &self,
        context: &DeliveryContext<'_>,
        role: &str,
    ) -> Result<(String, String)> {
        let own_task = context.sender.split('/').next().unwrap_or(context.sender);
        let source = context.plan.iter().find(|task| task.spec["id"] == own_task);
        let prefix = sender_prefix(context.sender);
        let mut origin = source_origin(context.sender, source);
        if let Some((target, _)) =
            self.mail_address_task(context.conversation, context.plan, role)?
            && Some(target.plan_id.as_str()) != context.plan_id
            && let Some(source) = source
        {
            origin = format!("{prefix}:{}", source.id);
        }
        let source_role = source.map_or_else(
            || context.sender.into(),
            |task| format!("{prefix}:{}", task.id),
        );
        Ok((origin, source_role))
    }
}

fn source_origin(sender: &str, source: Option<&Task>) -> String {
    if let Some(source) = source {
        if sender.ends_with("/verifier") {
            return format!("verifier:{}", source.id);
        }
        if sender.ends_with("/review") {
            return format!("code-review:{}", source.id);
        }
    }
    sender.into()
}

struct DeliveryContext<'a> {
    conversation: &'a str,
    plan_id: Option<&'a str>,
    sender: &'a str,
    plan: &'a [Task],
    message: &'a tasks::Outgoing,
}

fn load_delivery_plan(core: &Core, conversation: &str, plan_id: Option<&str>) -> Result<Vec<Task>> {
    match plan_id {
        Some(plan) => core.store.plan(plan),
        None => core.store.latest_plan(conversation),
    }
}

fn all_worker_targets(plan: &[Task], own_task: &str) -> Vec<(String, String)> {
    plan.iter()
        .filter(|task| task.spec["id"] != own_task)
        .map(|task| {
            (
                task.spec["id"].as_str().unwrap_or_default().to_owned(),
                format!("worker:{}", task.id),
            )
        })
        .collect()
}

fn sender_prefix(sender: &str) -> &'static str {
    if sender.ends_with("/verifier") {
        "verifier"
    } else if sender.ends_with("/review") {
        "code-review"
    } else {
        "worker"
    }
}

fn queued_mail(origin: String, label: &str, message: &tasks::Outgoing) -> Mail {
    Mail {
        id: 0,
        sender: origin,
        recipient: label.into(),
        kind: message.kind.clone(),
        text: message.text.clone(),
        created_ms: 0,
        delivered_turn: None,
        delivered_ms: None,
        state: "queued".into(),
    }
}

fn reject_delivery(
    core: &Core,
    conversation: &str,
    plan_id: Option<&str>,
    sender: &str,
    message: &tasks::Outgoing,
) -> Result<()> {
    core.store.append_event(
        conversation,
        None,
        plan_id,
        "mail_rejected",
        &json!({"sender": sender, "recipient": message.to, "text": message.text,
                "reason": "no such recipient in the current plan"}),
    )?;
    Ok(())
}
