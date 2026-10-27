use super::*;

impl Core {
    /// Pin configuration for one decision or operation. Existing snapshots survive updates.
    pub fn config_snapshot(&self) -> Arc<Config> {
        self.config
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn replace_config(&self, config: Config) {
        *self
            .config
            .write()
            .unwrap_or_else(|error| error.into_inner()) = Arc::new(config);
    }

    pub fn start(&self, project: &Path) -> Result<Conversation> {
        let project = project.canonicalize()?;
        let conversation = self.store.create_conversation(&project.to_string_lossy())?;
        if let Ok(branch) = git::session_branch(&project) {
            self.store.append_event(
                &conversation.id,
                None,
                None,
                "session_started",
                &json!({"branch": branch}),
            )?;
        }
        Ok(conversation)
    }

    /// Records the message and returns at once. The orchestrator answers in
    /// the background; messages sent meanwhile go into its next turn.
    pub fn send(self: &Arc<Self>, conversation: &str, text: &str) -> Result<Message> {
        self.store.conversation(conversation)?;
        let message = if text.trim() == "/planning" {
            require(
                self.is_idle(conversation),
                "wait for the orchestrator to finish before approving requirements",
            )?;
            self.store.approve_requirements(conversation)?
        } else {
            self.store.add_message(conversation, "user", text)?
        };
        self.wake(conversation);
        Ok(message)
    }

    /// A message from the user: with explicit recipient or `@recipient text` prefix,
    /// validates recipient role, plan, and broadcast targets, reporting real delivery states.
    pub fn submit(self: &Arc<Self>, conversation: &str, text: &str) -> Result<serde_json::Value> {
        self.submit_addressed(conversation, text, None)
    }

    pub fn send_to_agent(
        self: &Arc<Self>,
        message: &store::AgentMessage,
    ) -> Result<serde_json::Value> {
        let graph = self.graph_updates.lock().expect("graph update mutex");
        let _scope = self.integrating.lock().expect("integration mutex");
        let before = message
            .plan
            .as_ref()
            .map(|plan| self.store.plan(plan))
            .transpose()?
            .unwrap_or_default();
        let reply = self.store.enqueue_agent_message(message)?;
        if reply["state"] == "delivered" {
            return Ok(reply);
        }
        if message.recipient.contains(':')
            && let Some(plan) = &message.plan
        {
            let tasks = self.store.plan(plan)?;
            self.cancel_changed_task_inputs(&before, &tasks);
            for task in tasks {
                self.announce_task(&message.conversation, &task, None);
            }
            self.maybe_continue_plan(&message.conversation, plan)?;
        }
        drop(_scope);
        drop(graph);
        if matches!(message.recipient.as_str(), PLAN_VERIFIER | AUDITOR) {
            self.resume_agent_reviews(&message.conversation);
        }
        if message.recipient == ORCHESTRATOR {
            self.wake(&message.conversation);
        }
        Ok(reply)
    }

    pub fn conversation_scope_hash(&self, conversation: &str) -> Result<String> {
        let latest_seq = self.store.latest_event_seq(conversation)?;
        let paused = self.store.paused(conversation)?;
        let idle = self.is_idle(conversation);
        let active_plan = self
            .store
            .latest_plan(conversation)?
            .first()
            .map(|t| t.plan_id.clone());
        Ok(crate::hash(
            format!(
                "{conversation}:{latest_seq}:{paused}:{idle}:{:?}",
                active_plan
            )
            .as_bytes(),
        ))
    }

    pub fn plan_scope_hash(&self, _conversation: &str, plan_id: &str) -> Result<String> {
        let tasks = self.store.plan(plan_id)?;
        let revision = match self.store.plan_run(plan_id)? {
            Some(id) => Some(self.store.run(_conversation, &id)?.spec_revision),
            None => None,
        };
        Ok(crate::hash(
            format!("{}:{revision:?}", plan_scope_hash(&tasks)).as_bytes(),
        ))
    }

    pub fn diff_scope_hash(&self, conversation: &str, plan: &str, id: &str) -> Result<String> {
        let task = self.task_unscoped(plan, id)?;
        let (_, base, result) = self.change(conversation, &task)?;
        Ok(diff_scope_hash(
            conversation,
            plan,
            id,
            &base,
            result.as_deref(),
        ))
    }

    pub fn conversations(&self) -> Result<Vec<ConversationSummary>> {
        self.store
            .conversations()?
            .iter()
            .map(|conversation| self.conversation_summary(conversation))
            .collect()
    }

    fn conversation_summary(&self, conversation: &Conversation) -> Result<ConversationSummary> {
        let tasks = self.store.latest_plan(&conversation.id)?;
        let active_plan = tasks.first().map(|task| task.plan_id.clone());
        let tasks_done = tasks
            .iter()
            .filter(|task| task.status == status::APPROVED)
            .count();
        let paused = self.store.paused(&conversation.id).unwrap_or(false);
        let status = self.conversation_status(&conversation.id, paused, &tasks, &active_plan)?;
        let scope_hash = self
            .conversation_scope_hash(&conversation.id)
            .unwrap_or_default();
        Ok(ConversationSummary {
            id: conversation.id.clone(),
            project: conversation.project.clone(),
            created_ms: conversation.created_ms,
            status,
            active_plan,
            tasks_total: tasks.len(),
            tasks_done,
            scope_hash,
        })
    }

    fn conversation_status(
        &self,
        conversation: &str,
        paused: bool,
        tasks: &[Task],
        active_plan: &Option<String>,
    ) -> Result<String> {
        if let Some(status) = live_status(paused, self.is_idle(conversation)) {
            return Ok(status.into());
        }
        if tasks.iter().all(|task| task.status == status::APPROVED) && !tasks.is_empty() {
            return Ok(self.approved_plan_status(conversation, active_plan)?.into());
        }
        Ok(plan_status(tasks).into())
    }

    fn approved_plan_status(
        &self,
        conversation: &str,
        active_plan: &Option<String>,
    ) -> Result<&'static str> {
        let Some(run) = active_plan
            .as_deref()
            .map(|plan| self.store.plan_run(plan))
            .transpose()?
            .flatten()
        else {
            return Ok("requirements_pending");
        };
        let fingerprint = self.ledger_fingerprint(conversation);
        let complete = fingerprint
            .and_then(|fingerprint| {
                self.store
                    .requirements_gaps(conversation, &run, &fingerprint)
            })
            .is_ok_and(|gaps| gaps.is_empty());
        Ok(if complete {
            "done"
        } else {
            "requirements_pending"
        })
    }

    pub fn sessions(&self, conversation: &str) -> Result<Vec<store::SessionInfo>> {
        self.store.sessions(conversation)
    }

    pub fn history(
        &self,
        conversation: &str,
        after: u64,
        limit: Option<usize>,
    ) -> Result<store::HistoryPage> {
        let lim = limit.unwrap_or(500).min(500);
        self.store.history_page(conversation, after, lim)
    }

    /// The user's comment on one element: it reaches the orchestrator as a
    /// message now (a comment on the plan waits for the review) and stays in
    /// its prompts until the user resolves it.
    pub fn comment(
        self: &Arc<Self>,
        conversation: &str,
        target: &str,
        text: &str,
    ) -> Result<Comment> {
        self.comment_scoped(conversation, None, target, text)
    }

    /// Comments on an element, scoped optionally to a specific plan (active or historical).
    pub fn comment_scoped(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: Option<&str>,
        target: &str,
        text: &str,
    ) -> Result<Comment> {
        self.comment_at(conversation, plan_id, target, text, None, None)
    }
}

fn live_status(paused: bool, idle: bool) -> Option<&'static str> {
    if paused {
        Some("paused")
    } else if !idle {
        Some("working")
    } else {
        None
    }
}

fn plan_status(tasks: &[Task]) -> &'static str {
    if tasks.is_empty() {
        "idle"
    } else if tasks.iter().all(|task| task.status == status::PROPOSED) {
        "proposed"
    } else if tasks.iter().any(|task| task.status == status::NEEDS_USER) {
        "needs_user"
    } else if tasks.iter().any(|task| task.status == status::WAITING) {
        "waiting"
    } else if tasks.iter().any(|task| task.status == status::FAILED) {
        "failed"
    } else if tasks.iter().any(|task| task.status == status::CANCELLED) {
        "cancelled"
    } else {
        "idle"
    }
}
