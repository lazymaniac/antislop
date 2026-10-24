use super::core_comment_helpers::{comment_label, is_plan_target};
use super::*;

impl Core {
    pub fn comment_at(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: Option<&str>,
        target: &str,
        text: &str,
        round: Option<u32>,
        run: Option<&str>,
    ) -> Result<Comment> {
        self.store.conversation(conversation)?;
        let text = text.trim();
        require(!text.is_empty(), "the comment is empty")?;
        require(!target.trim().is_empty(), "the comment has no element")?;
        let (plan, resolved_plan_id) = self.comment_plan(conversation, plan_id)?;
        let label = contextual_comment_label(target, &plan, round, run);
        self.persist_comment(
            conversation,
            CommentDraft {
                target,
                text,
                label,
                plan_id: resolved_plan_id,
                round,
                run,
            },
        )
    }

    fn comment_plan(
        &self,
        conversation: &str,
        plan_id: Option<&str>,
    ) -> Result<(Vec<Task>, Option<String>)> {
        let plan = match plan_id {
            Some(id) => self.store.plan(id)?,
            None => self.store.latest_plan(conversation)?,
        };
        let resolved_id = plan_id
            .map(str::to_owned)
            .or_else(|| plan.first().map(|task| task.plan_id.clone()));
        Ok((plan, resolved_id))
    }

    fn persist_comment(
        self: &Arc<Self>,
        conversation: &str,
        draft: CommentDraft<'_>,
    ) -> Result<Comment> {
        let new = Comment {
            id: 0,
            plan_id: draft.plan_id,
            target: draft.target.into(),
            label: draft.label.clone(),
            text: draft.text.into(),
            status: "open".into(),
            message: 0,
        };
        if is_plan_target(draft.target) {
            return self.store.add_draft_comment(conversation, &new);
        }
        let comment =
            self.store
                .add_comment_at(conversation, &new, draft.round, draft.run, |id| {
                    format!("{COMMENT_PREFIX}{id} on {}] {}", draft.label, draft.text)
                })?;
        self.wake(conversation);
        Ok(comment)
    }

    /// The user finished commenting on the plan: the orchestrator now
    /// addresses all the draft comments together.
    pub fn review_comments(self: &Arc<Self>, conversation: &str, plan_id: &str) -> Result<usize> {
        self.store.conversation(conversation)?;
        let sent = self.store.send_draft_comments(conversation, plan_id)?;
        self.wake(conversation);
        Ok(sent.len())
    }

    /// Paginated read of all stored plans for a conversation.
    pub fn plans(&self, conversation: &str, offset: usize, limit: usize) -> Result<PlansPage> {
        self.store.plans_page(conversation, offset, limit)
    }

    /// Read detailed history of a single plan.
    pub fn plan_history(&self, conversation: &str, plan_id: &str) -> Result<PlanHistory> {
        self.store.plan_history(conversation, plan_id)
    }

    /// Resolves, reopens or deletes a comment; only the user does this.
    pub fn set_comment(
        self: &Arc<Self>,
        conversation: &str,
        id: u64,
        status: &str,
    ) -> Result<Comment> {
        require(
            matches!(status, "open" | "resolved" | "deleted"),
            "a comment is open, resolved or deleted",
        )?;
        let comment = self.store.set_comment_status(conversation, id, status)?;
        if status == "open" {
            self.wake(conversation);
        }
        Ok(comment)
    }

    /// Stops the orchestrator's turn and any running plan; queued messages stay.
    /// Pausing starts no new plan work (worker, verifier and audit turns):
    /// turns already running finish, the orchestrator still answers, and
    /// everything waiting goes on when the user continues. It survives a
    /// restart.
    /// Whether the conversation runs on by itself: the mode the user set for
    /// it, else the configured one.
    pub fn autonomous(&self, conversation: &str) -> Result<bool> {
        let config = self.config_snapshot();
        Ok(self
            .store
            .mode(conversation)?
            .unwrap_or_else(|| config.policy.automatic_progression()))
    }

    /// Changes the conversation's mode for what it does next (design §6.2);
    /// it approves nothing that already happened.
    pub fn set_mode(&self, conversation: &str, autonomous: bool) -> Result<()> {
        self.store.conversation(conversation)?;
        self.store.set_mode(conversation, autonomous)
    }

    pub fn pause(&self, conversation: &str, paused: bool) -> Result<()> {
        self.store.conversation(conversation)?;
        self.store.set_paused(conversation, paused)
    }

    /// Cancels only orchestration; task workers keep their own cancellation flags.
    pub fn restart_orchestrator(self: &Arc<Self>, conversation: &str) -> Result<()> {
        self.store.conversation(conversation)?;
        let running = self.running.lock().expect("running mutex");
        self.store.add_message(conversation, RUNTIME,
            "Restart the current orchestration round in a fresh session. Continue the user's request using the conversation history and durable project state.")?;
        self.orchestrator_restarts
            .lock()
            .expect("orchestrator restart mutex")
            .insert(conversation.into());
        if let Some(cancel) = running.get(conversation) {
            cancel.store(true, Ordering::SeqCst);
        }
        drop(running);
        self.wake(conversation);
        Ok(())
    }

    pub fn cancel(&self, conversation: &str) {
        for (owner, flag) in self
            .task_cancels
            .lock()
            .expect("task cancel mutex")
            .values()
        {
            if owner == conversation {
                flag.store(true, Ordering::SeqCst);
            }
        }
        for flags in [&self.running, &self.plans, &self.reviews, &self.mail_jobs] {
            if let Some(flag) = flags.lock().expect("flags mutex").get(conversation) {
                flag.store(true, Ordering::SeqCst);
            }
        }
    }

    pub fn cancel_scoped(&self, conversation: &str, scope_hash: Option<&str>) -> Result<()> {
        if let Some(expected) = scope_hash {
            let current = self.conversation_scope_hash(conversation)?;
            require(expected == current, "stale_scope")?;
        }
        self.cancel(conversation);
        Ok(())
    }

    pub fn is_idle(&self, conversation: &str) -> bool {
        // A plan hands its report to the orchestrator while holding plans.
        // Read those two slots in the same order to observe that handoff atomically.
        let plans = self.plans.lock().expect("plans mutex");
        let running = self.running.lock().expect("running mutex");
        !plans.contains_key(conversation)
            && !running.contains_key(conversation)
            && !self
                .reviews
                .lock()
                .expect("reviews mutex")
                .contains_key(conversation)
            && !self
                .mail_jobs
                .lock()
                .expect("mail jobs mutex")
                .contains_key(conversation)
    }

    /// Whether any conversation has an agent or a plan running.
    pub fn busy(&self) -> bool {
        !self.running.lock().expect("running mutex").is_empty()
            || !self.plans.lock().expect("plans mutex").is_empty()
            || !self.reviews.lock().expect("reviews mutex").is_empty()
            || !self.mail_jobs.lock().expect("mail jobs mutex").is_empty()
    }

    pub fn task_freshness_token(
        &self,
        conversation: &str,
        plan_id: &str,
        task_id: &str,
    ) -> Result<String> {
        let task = self.task_unscoped(plan_id, task_id)?;
        self.freshness_for_task(conversation, plan_id, &task)
    }
}

fn contextual_comment_label(
    target: &str,
    plan: &[Task],
    round: Option<u32>,
    run: Option<&str>,
) -> String {
    let mut label = comment_label(target, plan);
    if let Some(round) = round {
        label.push_str(&format!(" · round {round}"));
    }
    if let Some(run) = run {
        label.push_str(&format!(" · run {run}"));
    }
    label
}

struct CommentDraft<'a> {
    target: &'a str,
    text: &'a str,
    label: String,
    plan_id: Option<String>,
    round: Option<u32>,
    run: Option<&'a str>,
}
