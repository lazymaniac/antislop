use super::super::*;

pub(super) struct Repair<'a> {
    core: &'a Arc<Core>,
    conversation: &'a str,
    task: &'a mut Task,
    spec: &'a TaskContract,
    worktree: &'a Path,
    onto: &'a str,
    change: &'a str,
    files: &'a [String],
    outcome: Outcome,
    cancel: &'a AtomicBool,
    base: String,
    attempts: u64,
}

pub(super) struct RepairParts<'a> {
    pub(super) core: &'a Arc<Core>,
    pub(super) conversation: &'a str,
    pub(super) task: &'a mut Task,
    pub(super) spec: &'a TaskContract,
    pub(super) worktree: &'a Path,
    pub(super) onto: &'a str,
    pub(super) change: &'a str,
    pub(super) files: &'a [String],
    pub(super) outcome: Outcome,
    pub(super) cancel: &'a AtomicBool,
}

impl<'a> Repair<'a> {
    pub(super) fn new(parts: RepairParts<'a>) -> Self {
        let attempts = repair_count(parts.task) + 1;
        let base = parts.task.base.as_deref().unwrap_or_default().to_owned();
        Self {
            core: parts.core,
            conversation: parts.conversation,
            task: parts.task,
            spec: parts.spec,
            worktree: parts.worktree,
            onto: parts.onto,
            change: parts.change,
            files: parts.files,
            outcome: parts.outcome,
            cancel: parts.cancel,
            base,
            attempts,
        }
    }

    pub(super) fn run(&mut self) -> Result<()> {
        let prompt = self.prompt();
        self.record_attempt();
        self.core
            .set_status(self.conversation, self.task, status::WORKING)?;
        let Some(report) = self.worker_report(&prompt)? else {
            let worker = format!("worker:{}", self.task.id);
            return self
                .core
                .stop_task(self.conversation, self.task, &worker, self.cancel);
        };
        let result: turn_result::TurnResult = serde_json::from_str(&report)?;
        let report = match result {
            turn_result::TurnResult::Completed { report, .. } => report,
            turn_result::TurnResult::NeedsInput {
                request,
                checkpoint,
            } => {
                self.task.result = Some(git::commit_all(
                    self.worktree,
                    &format!("{} checkpoint", self.spec.title),
                )?);
                let branch =
                    super::super::core_path_helpers::worktree_branch(&self.core.state, self.task)?;
                super::super::core_feedback_helpers::land(
                    self.task,
                    self.outcome,
                    "run",
                    &checkpoint,
                    None,
                    Some(&branch),
                    self.files,
                );
                self.task.report = Some(checkpoint);
                self.task.verdict = None;
                return self
                    .core
                    .wait_for_question(self.conversation, self.task, &request, false);
            }
            _ => {
                return Err(Error::Invalid(
                    "integration repair requires a worker completion".into(),
                ));
            }
        };
        self.persist_repair(report)?;
        self.queue_verification()
    }

    fn record_attempt(&mut self) {
        self.task
            .feedback
            .get_or_insert_with(|| json!({"round": self.task.round}))["integration_repairs"] =
            json!(self.attempts);
    }

    fn prompt(&self) -> String {
        format!(
            "Automatic integration conflict in {}. Resolve it in your allocated worktree. \
             Original base: {}; your result: {}; current project snapshot: {}.\n\
             Inspect git diff {} {} and git diff {} {}, plus the relevant files and tests. \
             Combine both intentions, preserving current local user edits and the task's behavior. \
             Edit only task paths in this worktree; never modify the primary working tree or another \
             worktree. The runtime will use {} as the new base and independently verify your \
             repaired result before landing. Do not merge unrelated snapshot paths into your result.\n\
             Resolve mechanical conflicts yourself. If the versions represent an unclear product or \
             behavior choice, do not guess or silently pick ours/theirs. Use ask with the conflicting intentions and alternatives, and end with wait_for_answer when the decision remains pending. Otherwise resolve the \
             files and report what you preserved and how you tested it.",
            self.files.join(", "),
            self.base,
            self.change,
            self.onto,
            self.base,
            self.change,
            self.base,
            self.onto,
            self.onto,
        )
    }

    fn worker_report(&self, prompt: &str) -> Result<Option<String>> {
        let worker = format!("worker:{}", self.task.id);
        let setup = self.worker_setup(worker.clone());
        let handoff = self
            .core
            .round_handoff(&self.task.id, self.task.round, self.worktree)?;
        let Some(answer) = self.core.turn(setup, self.cancel, |fresh| {
            self.worker_prompt(prompt, &handoff, fresh)
        })?
        else {
            return Ok(None);
        };
        Ok(Some(answer))
    }

    fn worker_setup(&self, worker: String) -> TurnSetup<'_> {
        TurnSetup {
            session_policy: SessionPolicy::TaskRound {
                task: self.task.id.clone(),
                round: self.task.round,
            },
            conversation: self.conversation,
            plan_id: Some(self.task.plan_id.clone()),
            role: worker,
            profile: self
                .core
                .config_snapshot()
                .resolve(self.spec.kind, RoleKind::Executor),
            cwd: self.worktree.to_path_buf(),
            add_dirs: vec![],
            access: agent::Access::Write,
            upto_message: 0,
        }
    }

    fn worker_prompt(&self, prompt: &str, handoff: &rounds::RoundHandoff, fresh: bool) -> String {
        let instructions = tasks::worker_turn_prompt(
            &self.worktree.to_string_lossy(),
            self.spec,
            Some(prompt),
            "",
        );
        if fresh {
            instructions
        } else {
            instructions + &tasks::round_handoff_prompt(handoff, false)
        }
    }

    fn persist_repair(&mut self, report: String) -> Result<()> {
        self.core.store.assert_task_inputs(self.task)?;
        self.task.report = Some(report);
        self.task.verdict = None;
        self.task.feedback =
            Some(json!({"round": self.task.round, "integration_repairs": self.attempts}));
        self.core.store.update_task(self.task)?;
        let result = git::transplant_repair(
            self.worktree,
            &self.base,
            self.onto,
            &format!("{} (conflict resolution)", self.spec.title),
        )?;
        self.task.base = Some(self.onto.to_owned());
        self.task.result = Some(result);
        self.core.store.update_task(self.task)
    }

    fn queue_verification(&mut self) -> Result<()> {
        let verifier = format!("verifier:{}", self.task.id);
        self.core.store.add_mail(
            self.conversation,
            &verifier,
            &Mail {
                id: 0,
                sender: RUNTIME.into(),
                recipient: verifier.clone(),
                kind: "agent_work".into(),
                text: self.verification_prompt(),
                created_ms: 0,
                delivered_turn: None,
                delivered_ms: None,
                state: "queued".into(),
            },
        )?;
        self.core
            .set_status(self.conversation, self.task, status::QUEUED)
    }

    fn verification_prompt(&self) -> String {
        format!(
            "Verify the conflict resolution independently. Compare original task delta: git diff {} {}; concurrent project edits: git diff {} {}. Confirm both intentions survive in {} and that no unrelated project edits are reverted. Reject silent ours/theirs choices and unresolved conflict markers. If the repair leaves a behavior choice unresolved, ask the orchestrator.",
            self.base,
            self.change,
            self.base,
            self.onto,
            self.files.join(", "),
        )
    }
}

fn repair_count(task: &Task) -> u64 {
    task.feedback
        .as_ref()
        .and_then(|feedback| feedback["integration_repairs"].as_u64())
        .unwrap_or(0)
}
