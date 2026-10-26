use super::core_feedback_helpers::review_json;
use super::*;

impl Core {
    /// Reviews a writer's change on the user's request, with the task's
    /// verifier profile in a session of its own. The findings go to the
    /// orchestrator; the review never changes the task's status.
    pub fn review_change(self: &Arc<Self>, conversation: &str, plan: &str, id: &str) -> Result<()> {
        let work = self.prepare_code_review(conversation, plan, id)?;
        self.store.append_event(
            conversation,
            None,
            Some(plan),
            "code_review",
            &json!({"id": id, "status": "running"}),
        )?;
        let core = self.clone();
        std::thread::spawn(move || core.run_code_review(work));
        Ok(())
    }

    fn prepare_code_review(&self, conversation: &str, plan: &str, id: &str) -> Result<CodeReview> {
        let task = self.task_unscoped(plan, id)?;
        let spec = serde_json::from_value(task.spec.clone())?;
        let (dir, base, result) = self.change(conversation, &task)?;
        let cancel = self.reserve_review(conversation)?;
        let comments = review_comments(&self.store, conversation, plan, id)?;
        Ok(CodeReview {
            cancel,
            conversation: conversation.to_owned(),
            plan: plan.to_owned(),
            task_id: task.id,
            spec,
            dir,
            base,
            result,
            comments,
        })
    }

    fn reserve_review(&self, conversation: &str) -> Result<Arc<AtomicBool>> {
        let mut reviews = self.reviews.lock().expect("reviews mutex");
        require(
            !reviews.contains_key(conversation),
            "a code review is already running",
        )?;
        let cancel = Arc::new(AtomicBool::new(false));
        reviews.insert(conversation.to_owned(), cancel.clone());
        Ok(cancel)
    }

    fn run_code_review(self: Arc<Self>, work: CodeReview) {
        let review = execute_code_review(&self, &work);
        publish_code_review(&self, &work, &review);
        self.reviews
            .lock()
            .expect("reviews mutex")
            .remove(&work.conversation);
        self.wake(&work.conversation);
    }

    pub fn review_change_scoped(
        self: &Arc<Self>,
        conversation: &str,
        plan: &str,
        id: &str,
        scope_hash: Option<&str>,
    ) -> Result<()> {
        if let Some(expected) = scope_hash {
            let current = self.diff_scope_hash(conversation, plan, id)?;
            require(expected == current, "stale_scope")?;
        }
        self.review_change(conversation, plan, id)
    }

    /// Open comments targeting the plan document or whole plan `plan_id`.
    pub fn open_plan_comments(&self, conversation: &str, plan_id: &str) -> Result<Vec<Comment>> {
        let prefix = format!("plan:{plan_id}:");
        let exact = format!("plan:{plan_id}");
        // Drafts count: they have not been answered either.
        let mut comments = self.store.comments(conversation, "open")?;
        comments.extend(self.store.comments(conversation, "draft")?);
        Ok(comments
            .into_iter()
            // The same rule as the screen's (`View::open_plan_comments`).
            .filter(|c| {
                (c.plan_id.is_none() || c.plan_id.as_deref() == Some(plan_id))
                    && (c.target == "plan"
                        || c.target == exact
                        || c.target.starts_with(&prefix)
                        || c.target.starts_with("plan:line:"))
            })
            .collect())
    }

    /// Starts the proposed plan `plan_id`: exactly the one the user saw, even
    /// if the orchestrator has proposed another since. Requires all open plan
    /// comments on `plan_id` to be resolved first.
    pub fn accept_plan(self: &Arc<Self>, conversation: &str, plan_id: &str) -> Result<Vec<Task>> {
        let open = self.open_plan_comments(conversation, plan_id)?;
        require(
            open.is_empty(),
            "send the draft plan comments with Review and resolve all open ones before accepting the plan",
        )?;
        self.start_plan(conversation, plan_id, |store| {
            store.queue_plan(conversation, plan_id).map(|_| ())
        })
    }

    /// Rejects the proposed plan `plan_id` and asks the orchestrator to revise it.
    pub fn reject_plan(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        reason: Option<&str>,
    ) -> Result<()> {
        self.store.conversation(conversation)?;
        let changed = self
            .store
            .set_plan_status(plan_id, status::PROPOSED, status::CANCELLED)?;
        require(changed > 0, "this plan is not waiting for approval")?;
        let tasks = self.store.plan(plan_id)?;
        let reason = rejection_reason(reason);
        self.announce_rejected_tasks(conversation, &tasks);
        self.record_plan_rejection(conversation, plan_id, &reason)?;
        self.wake(conversation);
        Ok(())
    }

    fn announce_rejected_tasks(&self, conversation: &str, tasks: &[Task]) {
        for task in tasks {
            self.announce_task(conversation, task, Some("plan rejected by user"));
        }
    }

    fn record_plan_rejection(&self, conversation: &str, plan_id: &str, reason: &str) -> Result<()> {
        self.store.append_event(
            conversation,
            None,
            Some(plan_id),
            "plan_rejected",
            &json!({"plan": plan_id, "reason": reason}),
        )?;
        self.store.add_message(
            conversation,
            "user",
            &format!("[rejected plan {plan_id}] {reason}"),
        )?;
        Ok(())
    }

    pub fn reject_plan_scoped(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: &str,
        reason: Option<&str>,
        scope_hash: Option<&str>,
    ) -> Result<()> {
        if let Some(expected) = scope_hash {
            let current = self.plan_scope_hash(conversation, plan_id)?;
            require(expected == current, "stale_scope")?;
        }
        self.reject_plan(conversation, plan_id, reason)
    }
}

fn rejection_reason(reason: Option<&str>) -> String {
    reason
        .map(str::trim)
        .filter(|reason| !reason.is_empty())
        .unwrap_or("Please revise the plan or clarify requirements.")
        .to_owned()
}

struct CodeReview {
    cancel: Arc<AtomicBool>,
    conversation: String,
    plan: String,
    task_id: String,
    spec: TaskContract,
    dir: PathBuf,
    base: String,
    result: Option<String>,
    comments: Vec<Comment>,
}

fn review_comments(
    store: &Store,
    conversation: &str,
    plan: &str,
    id: &str,
) -> Result<Vec<Comment>> {
    let prefix = format!("diff:{id}");
    Ok(store
        .comments(conversation, "open")?
        .into_iter()
        .filter(|comment| {
            comment.plan_id.as_deref() == Some(plan) && comment.target.starts_with(&prefix)
        })
        .collect())
}

fn execute_code_review(core: &Core, work: &CodeReview) -> std::result::Result<Verdict, String> {
    let setup = review_turn_setup(core, work);
    let answer = core
        .turn(setup, &work.cancel, |_| review_prompt(work))
        .map_err(|error| error.to_string())?;
    match answer {
        Some(answer) => Ok(tasks::parse_verdict(&answer)),
        None => Err(review_failure(core, work)),
    }
}

fn review_turn_setup<'a>(core: &Core, work: &'a CodeReview) -> TurnSetup<'a> {
    TurnSetup {
        session_policy: SessionPolicy::Role,
        conversation: &work.conversation,
        plan_id: Some(work.plan.clone()),
        role: format!("{CODE_REVIEW}:{}", work.task_id),
        profile: core
            .config_snapshot()
            .resolve(work.spec.kind, RoleKind::Verifier),
        cwd: work.dir.clone(),
        add_dirs: vec![],
        access: agent::Access::ReadOnly,
        upto_message: 0,
    }
}

fn review_prompt(work: &CodeReview) -> String {
    tasks::code_review_prompt(
        &work.dir.to_string_lossy(),
        &work.spec,
        &work.base,
        work.result.as_deref(),
        &work.comments,
    )
}

fn review_failure(core: &Core, work: &CodeReview) -> String {
    let role = format!("{CODE_REVIEW}:{}", work.task_id);
    core.store
        .last_turn(&work.conversation, &role)
        .ok()
        .flatten()
        .and_then(|turn| turn.error)
        .unwrap_or_else(|| "no reason recorded".into())
}

fn publish_code_review(
    core: &Core,
    work: &CodeReview,
    review: &std::result::Result<Verdict, String>,
) {
    let mut payload = review_json(review);
    payload["id"] = json!(work.spec.id);
    payload["status"] = json!("done");
    let _ = core.store.append_event(
        &work.conversation,
        None,
        Some(&work.plan),
        "code_review",
        &payload,
    );
    if let Ok(verdict) = review {
        publish_review_message(core, work, verdict);
    }
}

fn publish_review_message(core: &Core, work: &CodeReview, verdict: &Verdict) {
    let findings = if verdict.findings.is_empty() {
        "No findings.".to_owned()
    } else {
        tasks::findings_text(&verdict.findings)
    };
    let text = format!(
        "{CODE_REVIEW_PREFIX} of task {} \"{}\", requested by the user]\n{findings}",
        work.spec.id, work.spec.title
    );
    let _ = core
        .store
        .add_message(&work.conversation, CODE_REVIEW, &text);
}
