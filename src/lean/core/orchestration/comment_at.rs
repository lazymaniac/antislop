use crate::lean::core::orchestration::comment_helpers::{comment_label, is_plan_target};
use crate::lean::*;

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
        self.comment_at_with_operation(conversation, None, plan_id, target, text, round, run)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn comment_at_with_operation(
        self: &Arc<Self>,
        conversation: &str,
        operation_id: Option<&str>,
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
        let diff_author = diff_author(&plan, target)?;
        let label = contextual_comment_label(target, &plan, round, run);
        let operation_id = operation_id.map(str::to_owned).unwrap_or_else(crate::id);
        let comment = self.persist_comment(
            conversation,
            &operation_id,
            CommentDraft {
                target,
                text,
                label,
                plan_id: resolved_plan_id,
                round,
                run,
            },
            diff_author.is_none(),
        )?;
        if let Some(task) = diff_author {
            let recipient = format!("worker:{}", task.id);
            let message = store::AgentMessage {
                conversation: conversation.into(),
                plan: Some(task.plan_id.clone()),
                recipient,
                operation_id,
                expected_round: Some(task.round),
                expected_run: None,
                delivery: Default::default(),
                text: format!(
                    "Diff comment #{} at {} (task round {}):\n{}\nAddress this location-specific feedback in your changes.",
                    comment.id, target, task.round, text
                ),
            };
            self.send_to_agent(&message)?;
        }
        Ok(comment)
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
        operation_id: &str,
        draft: CommentDraft<'_>,
        notify_orchestrator: bool,
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
        let comment = self.store.add_comment_at_with_operation(
            conversation,
            operation_id,
            &new,
            draft.round,
            draft.run,
            |id| format!("{COMMENT_PREFIX}{id} on {}] {}", draft.label, draft.text),
        )?;
        if notify_orchestrator {
            self.wake(conversation);
        }
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

    /// Pauses starting new work; running turns can finish and drafts remain intact.
    pub fn pause(&self, conversation: &str, paused: bool) -> Result<()> {
        self.store.conversation(conversation)?;
        self.store.set_paused(conversation, paused)
    }

    /// Cancels only orchestration; task workers keep their own cancellation flags.
    pub fn restart_orchestrator(self: &Arc<Self>, conversation: &str) -> Result<()> {
        self.store.conversation(conversation)?;
        let running = self.activity.running.lock().expect("running mutex");
        self.store.add_message(conversation, RUNTIME,
            "Restart the current orchestration round in a fresh session. Continue the user's request from the current accepted plan, task_states, full recorded results and durable project state. Preserve worktrees, input generations and retained evidence. The runtime imposes no access limits or execution budgets on agents. A restart alone does not require a plan revision. Use the current runtime tools: resume_task for a resolved blocker or remaining independent authorized work under the unchanged contract, retry_task for a transient failed CLI, and revise_plan only for actual contract/dependency changes. Keep steps with unavailable prerequisites and deferred work held; never fabricate missing historical data or ask again for an answered decision. Record new experiments as new evidence.")?;
        self.activity
            .orchestrator_restarts
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
            .activity
            .task_cancels
            .lock()
            .expect("task cancel mutex")
            .values()
        {
            if owner == conversation {
                flag.store(true, Ordering::SeqCst);
            }
        }
        for flags in [
            &self.activity.plans,
            &self.activity.running,
            &self.activity.reviews,
            &self.revisions.jobs,
            &self.activity.agent_reviews,
        ] {
            if let Some(flag) = flags.lock().expect("flags mutex").get(conversation) {
                flag.store(true, Ordering::SeqCst);
            }
        }
    }

    pub fn is_idle(&self, conversation: &str) -> bool {
        // A plan hands its report to the orchestrator while holding plans.
        // Read those two slots in the same order to observe that handoff atomically.
        let plans = self.activity.plans.lock().expect("plans mutex");
        self.is_idle_with_plans(conversation, &plans)
    }

    /// Delete only while no in-memory worker is active, excluding a concurrent wake.
    pub fn delete_session(&self, conversation: &str) -> Result<()> {
        // Keep the established plan-to-running lock order used by idle checks.
        let plans = self.activity.plans.lock().expect("plans mutex");
        let _lifecycle = self
            .locks
            .session_lifecycle
            .lock()
            .expect("session lifecycle mutex");
        require(
            self.is_idle_with_plans(conversation, &plans),
            "stop this session's work before deleting it",
        )?;
        self.store.delete_session(conversation)
    }

    fn is_idle_with_plans(
        &self,
        conversation: &str,
        plans: &HashMap<String, Arc<AtomicBool>>,
    ) -> bool {
        let running = self.activity.running.lock().expect("running mutex");
        !self.activity.agent_review_running(conversation)
            && !self
                .revisions
                .jobs
                .lock()
                .expect("revision jobs mutex")
                .contains_key(conversation)
            && !plans.contains_key(conversation)
            && !running.contains_key(conversation)
            && !self.activity.review_running(conversation)
    }

    /// Whether any conversation has an agent or a plan running.
    pub fn busy(&self) -> bool {
        !self
            .activity
            .agent_reviews
            .lock()
            .expect("agent reviews mutex")
            .is_empty()
            || !self
                .revisions
                .jobs
                .lock()
                .expect("revision jobs mutex")
                .is_empty()
            || !self
                .activity
                .running
                .lock()
                .expect("running mutex")
                .is_empty()
            || !self.activity.plans.lock().expect("plans mutex").is_empty()
            || !self
                .activity
                .reviews
                .lock()
                .expect("reviews mutex")
                .is_empty()
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

fn diff_author<'a>(plan: &'a [Task], target: &str) -> Result<Option<&'a Task>> {
    let Some(rest) = target.strip_prefix("diff:") else {
        return Ok(None);
    };
    let Some((task_ref, location)) = rest.split_once(':') else {
        return Err(Error::Invalid("diff comment has no file location".into()));
    };
    require(
        !location.trim().is_empty(),
        "diff comment has no file location",
    )?;
    plan.iter()
        .find(|task| task.id == task_ref || task.spec["id"].as_str() == Some(task_ref))
        .map(Some)
        .ok_or_else(|| Error::Invalid(format!("diff comment task {task_ref} is unavailable")))
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
