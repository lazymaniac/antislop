use super::*;
mod context;
mod prompt;
use context::comment_context;
use prompt::{PromptContext, orchestrator_prompt_text, planning_ledger_context};

impl Core {
    pub(in crate::lean) fn orchestrator_turn(
        self: &Arc<Self>,
        conversation: &str,
        upto: u64,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let config = self.config_snapshot();
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        // A failed turn may never have reached the agent: repeat everything
        // asked since the last answer.
        let answered = self.store.last_orchestration_answered(conversation)?;
        let asked: Vec<Message> = self
            .store
            .messages_after(conversation, answered)?
            .into_iter()
            .filter(|message| message.author != ORCHESTRATOR && message.seq <= upto)
            .collect();
        let is_comment = asked
            .iter()
            .any(|message| message.text.starts_with(COMMENT_PREFIX));
        let setup = TurnSetup {
            session_policy: SessionPolicy::Role,
            conversation,
            plan_id: None,
            role: if is_comment {
                COMMENT_ORCHESTRATOR
            } else {
                ORCHESTRATOR
            }
            .into(),
            profile: config.resolve(WorkKind::Orchestration, RoleKind::Orchestrator),
            cwd: project.clone(),
            add_dirs: vec![],
            access: agent::Access::ReadOnly,
            upto_message: upto,
        };
        // Comments the user has not resolved stay in view; the ones asked in
        // this turn already are.
        let open: Vec<Comment> = self
            .store
            .comments(conversation, "open")?
            .into_iter()
            .filter(|comment| comment.message <= answered)
            .collect();
        let comment_context = comment_context(self, conversation, upto, answered, is_comment)?;
        let workflow = self.store.workflow(conversation)?;
        let interviewing = workflow.as_ref().is_some_and(|w| w.phase == "interview");
        if workflow.as_ref().is_some_and(|w| w.phase == "planning") {
            self.store.append_event(
                conversation,
                None,
                None,
                "planning_progress",
                &json!({"text": "Working on the plan draft"}),
            )?;
        }
        let graph = self.store.latest_plan(conversation)?;
        let editable_graph = !graph.is_empty()
            && (graph
                .iter()
                .all(|task| matches!(task.status.as_str(), status::PROPOSED | status::REVISED))
                || graph.iter().any(|task| {
                    matches!(
                        task.status.as_str(),
                        status::QUEUED | status::WORKING | status::VERIFYING
                    )
                }));
        let graph_json =
            serde_json::to_string(&graph.iter().map(|task| &task.spec).collect::<Vec<_>>())?;
        let history = self.store.recent_messages_through(conversation, answered)?;
        let ledger_context = planning_ledger_context(self, conversation)?;
        let Some(answer) = self.turn(setup, cancel, |first| {
            orchestrator_prompt_text(PromptContext {
                project: &project,
                first,
                answered,
                asked: &asked,
                open: &open,
                history: &history,
                comment_context: &comment_context,
                editable_graph,
                graph: &graph,
                graph_json: &graph_json,
                interviewing,
                workflow: workflow.as_ref(),
                ledger_context: ledger_context.as_deref(),
            })
        })?
        else {
            return Ok(());
        };
        let latest = self.store.latest_plan(conversation)?;
        let answer = self.post(
            conversation,
            latest.first().map(|task| task.plan_id.as_str()),
            ORCHESTRATOR,
            &answer,
        )?;
        if let Some(task) = latest.first() {
            self.answer_in_background(conversation, &task.plan_id);
        }
        // A regenerated plan can carry a fresh selective claim too, including
        // after a deferred original comment revision.
        let (answer, addressed) = feedback::split(&answer);
        let (answer, unblocking) = tasks::parse_recoveries(&answer);
        let (mut text, mut plan) = tasks::parse_plan(&answer);
        let graph_revision = answer.contains("```graph");
        let mut graph_baseline = None;
        if graph_revision {
            let graph = self.store.latest_plan(conversation)?;
            let current = graph
                .iter()
                .map(|task| serde_json::from_value(task.spec.clone()))
                .collect::<std::result::Result<Vec<TaskSpec>, _>>()?;
            let (graph_text, graph_edit) = tasks::parse_graph_edit(
                &answer,
                graph.first().map_or("", |task| task.plan_id.as_str()),
                &current,
            );
            if let Some(edit) = graph_edit {
                text = graph_text;
                plan = Some(if plan.is_some() {
                    Err(Error::Invalid(
                        "return either a graph edit or a full plan, never both".into(),
                    ))
                } else if graph.is_empty() {
                    Err(Error::Invalid("no graph to edit".into()))
                } else {
                    edit
                });
            }
            graph_baseline = Some(graph);
        }
        if interviewing {
            match interview::parse(&text) {
                Some(Ok(question)) => {
                    self.store.ask_interview_question(conversation, &question)?;
                    text = question.transcript();
                }
                Some(Err(error)) => self.notice(
                    conversation,
                    &format!("invalid interview question: {error}"),
                ),
                None => {
                    if let Some(summary) = tasks::requirements_summary(&text) {
                        self.store.interview_summary(conversation, &summary, upto)?;
                        text = summary;
                    }
                }
            }
            if plan.is_some() {
                self.notice(
                    conversation,
                    "plan withheld during interview; planning starts after the requirements summary is complete",
                );
            }
            plan = None;
        }
        if workflow.as_ref().is_some_and(|w| w.phase == "planning") {
            self.store.append_event(conversation, None, None, "planning_progress", &json!({"text": if plan.as_ref().is_some_and(|p| p.is_ok()) { "Plan draft ready" } else { "Working on the plan draft" }}))?;
        }
        self.store.add_message(conversation, ORCHESTRATOR, &text)?;
        if interviewing && !cancel.load(Ordering::SeqCst) && self.autonomous(conversation)? {
            self.store.finish_interview(conversation)?;
        }
        if !interviewing && !unblocking.is_empty() {
            // Supervised: only an answer of the user lets the orchestrator
            // unblock; the runtime's own report does not.
            let decided = asked.last().is_some_and(|message| message.author == "user");
            if decided || self.autonomous(conversation)? {
                self.unblock_by_orchestrator(conversation, &unblocking);
            } else {
                self.notice(
                    conversation,
                    "the orchestrator proposed to unblock steps before you decided; nothing was done",
                );
            }
        }
        let running_baseline = graph_baseline.as_ref().unwrap_or(&graph);
        let live_revision = graph_revision
            && running_baseline
                .iter()
                .any(|task| !matches!(task.status.as_str(), status::PROPOSED | status::REVISED))
            || workflow
                .as_ref()
                .is_some_and(|workflow| workflow.phase == "implementation")
                && running_baseline.iter().any(|task| {
                    matches!(
                        task.status.as_str(),
                        status::QUEUED | status::WORKING | status::VERIFYING
                    )
                });
        if !interviewing
            && live_revision
            && let Some(Ok(specs)) = plan.as_ref()
        {
            if let Err(error) = self.revise_running_graph(
                conversation,
                &project,
                &text,
                running_baseline,
                specs,
                cancel,
            ) {
                self.store.append_event(
                    conversation,
                    None,
                    running_baseline.first().map(|task| task.plan_id.as_str()),
                    "plan_invalid",
                    &json!({"error":error.to_string(),"retrying":false}),
                )?;
                self.notice(conversation, &format!("graph edit rejected: {error}"));
            }
            return Ok(());
        }
        match plan {
            Some(Ok(specs)) => self.propose(
                conversation,
                &project,
                &text,
                PlanProposal {
                    config: &config,
                    specs: &specs,
                },
                addressed.as_deref(),
                cancel,
            )?,
            Some(Err(error)) => {
                let retrying = if !graph_revision
                    && let Some(turn) = self.store.last_orchestration_turn(conversation)?
                {
                    self.store.queue_plan_format_repair(
                        conversation,
                        &turn.id,
                        &error.to_string(),
                    )?
                } else {
                    false
                };
                self.store.append_event(
                    conversation,
                    None,
                    None,
                    "plan_invalid",
                    &json!({"error":error.to_string(),"retrying":retrying,"retry":if retrying {"Correcting the plan format automatically."} else {"The plan could not be decoded after two correction attempts. Send a message to retry; your requirements are preserved."}}),
                )?;
            }
            None => {
                // Answered in words only: the comments stay open for the user.
                if is_comment {
                    let replied: Vec<u64> = self
                        .store
                        .comments(conversation, "open")?
                        .into_iter()
                        .filter(|c| c.message > answered && c.message <= upto)
                        .map(|c| c.id)
                        .collect();
                    self.store.append_event(
                        conversation,
                        None,
                        None,
                        "comment_replied",
                        &json!({"comments": replied}),
                    )?;
                }
            }
        }
        Ok(())
    }
}
