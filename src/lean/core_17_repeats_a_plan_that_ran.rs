use super::*;

impl Core {
    /// Whether `specs` asks for the same work as the last plan of the
    /// conversation that ran (ids aside).
    pub(in crate::lean) fn repeats_a_plan_that_ran(
        &self,
        conversation: &str,
        plan_id: &str,
        specs: &[TaskSpec],
    ) -> Result<bool> {
        let work = |spec: &TaskSpec| {
            (
                spec.title.clone(),
                spec.goal.clone(),
                spec.kind,
                spec.writes,
            )
        };
        let Some(earlier) = self.store.last_plan_that_ran(conversation, plan_id)? else {
            return Ok(false);
        };
        let mut before: Vec<_> = earlier
            .iter()
            .filter_map(|task| serde_json::from_value::<TaskSpec>(task.spec.clone()).ok())
            .map(|spec| work(&spec))
            .collect();
        let mut now: Vec<_> = specs.iter().map(work).collect();
        before.sort();
        now.sort();
        Ok(before == now)
    }

    /// Sends the ```message blocks of `answer` and returns the answer
    /// without them.
    pub(in crate::lean) fn post(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: Option<&str>,
        sender: &str,
        answer: &str,
    ) -> Result<String> {
        let (text, outgoing, unreadable) = tasks::parse_messages_checked(answer);
        for body in unreadable {
            self.store.append_event(
                conversation,
                None,
                plan_id,
                "mail_rejected",
                &json!({"sender": sender, "recipient": "?", "text": body,
                        "reason": "unreadable message block: invalid JSON or an unescaped ``` inside it"}),
            )?;
        }
        for message in outgoing {
            self.deliver(conversation, plan_id, sender, message)?;
        }
        Ok(text)
    }

    /// Queues a message for its recipients: the orchestrator, one task's
    /// worker, or (`all`) the workers of every task but the
    /// sender's.
    pub(in crate::lean) fn deliver(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: Option<&str>,
        sender: &str,
        message: tasks::Outgoing,
    ) -> Result<()> {
        // Mail turns of the orchestrator and auditors carry no plan; their
        // task labels refer to the conversation's current plan.
        let plan = match plan_id {
            Some(plan_id) => self.store.plan(plan_id)?,
            None => self.store.latest_plan(conversation)?,
        };
        let own_task = sender.split('/').next().unwrap_or(sender);
        let targets: Vec<(String, String)> = match message.to.as_str() {
            ORCHESTRATOR | COMMENT_ORCHESTRATOR => vec![(message.to.clone(), message.to.clone())],
            PLAN_VERIFIER => vec![(PLAN_VERIFIER.into(), PLAN_VERIFIER.into())],
            AUDITOR => vec![(AUDITOR.into(), AUDITOR.into())],
            "all" => plan
                .iter()
                .filter(|task| task.spec["id"] != own_task)
                .map(|task| {
                    (
                        task.spec["id"].as_str().unwrap_or_default().to_owned(),
                        format!("worker:{}", task.id),
                    )
                })
                .collect(),
            to => self
                .mail_address_task(conversation, &plan, to)?
                .map(|(_, role)| vec![(to.to_owned(), role)])
                .unwrap_or_default(),
        };
        if targets.is_empty() {
            self.store.append_event(
                conversation,
                None,
                plan_id,
                "mail_rejected",
                &json!({"sender": sender, "recipient": message.to, "text": message.text,
                        "reason": "no such recipient in the current plan"}),
            )?;
        }
        for (label, role) in targets {
            let mut origin = sender.to_owned();
            if let Some(source) = plan.iter().find(|task| task.spec["id"] == own_task) {
                if sender.ends_with("/verifier") {
                    origin = format!("verifier:{}", source.id);
                } else if sender.ends_with("/review") {
                    origin = format!("code-review:{}", source.id);
                }
            }
            if let Some((target, _)) = self.mail_address_task(conversation, &plan, &role)?
                && Some(target.plan_id.as_str()) != plan_id
                && let Some(source) = plan.iter().find(|task| task.spec["id"] == own_task)
            {
                let prefix = if sender.ends_with("/verifier") {
                    "verifier"
                } else if sender.ends_with("/review") {
                    "code-review"
                } else {
                    "worker"
                };
                origin = format!("{prefix}:{}", source.id);
            }
            let source_role = plan
                .iter()
                .find(|task| task.spec["id"] == own_task)
                .map(|task| {
                    format!(
                        "{}:{}",
                        if sender.ends_with("/verifier") {
                            "verifier"
                        } else if sender.ends_with("/review") {
                            "code-review"
                        } else {
                            "worker"
                        },
                        task.id
                    )
                })
                .unwrap_or_else(|| sender.into());
            let source_turn = self
                .store
                .last_turn(conversation, &source_role)?
                .map(|turn| turn.id);
            self.store.add_mail_from(
                conversation,
                &role,
                &Mail {
                    id: 0,
                    sender: origin,
                    recipient: label,
                    kind: message.kind.clone(),
                    text: message.text.clone(),
                    created_ms: 0,
                    delivered_turn: None,
                    delivered_ms: None,
                    state: "queued".into(),
                },
                source_turn.as_deref(),
            )?;
        }
        self.answer_in_background(conversation, plan_id.unwrap_or(""));
        Ok(())
    }

    /// The approved reports of the tasks `spec` depends on, as prompt text.
    pub(in crate::lean) fn upstream(&self, plan_id: &str, spec: &TaskSpec) -> Result<String> {
        let reports = self.store.dependency_reports(plan_id, &spec.depends_on)?;
        Ok(tasks::dependency_reports_note(&reports))
    }

    /// Where a writer starts: from the result of the one task it depends on,
    /// from the user's HEAD when none of them wrote, and from HEAD with all
    /// their results laid on it when several did.
    pub(in crate::lean) fn dependency_tasks(
        &self,
        plan_id: &str,
        spec: &TaskSpec,
    ) -> Result<Vec<Task>> {
        let plan = self.store.plan(plan_id)?;
        let mut seen = std::collections::HashSet::new();
        let mut ordered = Vec::new();
        for dep in &spec.depends_on {
            collect_dependencies(dep, &plan, &mut seen, &mut ordered)?;
        }
        Ok(ordered)
    }
}

fn collect_dependencies(
    id: &str,
    plan: &[Task],
    seen: &mut std::collections::HashSet<String>,
    ordered: &mut Vec<Task>,
) -> Result<()> {
    if !seen.insert(id.to_owned()) {
        return Ok(());
    }
    let task = plan
        .iter()
        .find(|task| task.spec["id"] == id)
        .ok_or_else(|| Error::Invalid(format!("missing dependency {id}")))?;
    let dependencies: Vec<String> = serde_json::from_value(task.spec["depends_on"].clone())?;
    for dependency in dependencies {
        collect_dependencies(&dependency, plan, seen, ordered)?;
    }
    ordered.push(task.clone());
    Ok(())
}
