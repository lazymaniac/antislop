use super::core_prompt_helpers::orchestrator_prompt;
use super::*;
mod context;
use context::comment_context;

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
        let Some(answer) = self.turn(setup, cancel, |first| {
            let mut prompt = orchestrator_prompt(&project, first || answered == 0, &asked, &open);
            if first && answered > 0 {
                prompt.push_str("\nPrevious conversation context (most recent messages):\n");
                for message in &history {
                    prompt.push_str(&format!("{}: {}\n", message.author, tasks::bound(&message.text, 2048)));
                }
            }
            prompt.push_str(&comment_context);
            if editable_graph && !interviewing {
                prompt.push_str(&format!(
                    "\nEditable graph snapshot, base_plan={}:\n{}\nTo insert tasks or change dependencies, return a fenced ```graph JSON block with base_plan and tasks (complete specs of only added or updated tasks). Omitted nodes are preserved. Keep existing IDs. For example, to insert implementation between A and security/performance gates, add B depending on A and update those gates to depend on B. The runtime merges and validates the entire DAG. Before execution it stores a new immutable plan revision for review and approval. During execution it independently reviews the additive edit and applies it atomically within the approved requirements, preserving existing agents and results. Return either graph or plan, never both. During execution, existing task updates may change only depends_on, and only before that task has started. Add a new task for other changes. Never rewrite running or completed tasks. Historical graphs remain accessible through /plans.\n",
                    graph[0].plan_id,
                    graph_json
                ));
            }
            if interviewing {
                prompt.insert_str(0, "\nSESSION PHASE: INTERVIEW. This overrides any instruction to propose a plan. Ask concise concrete questions in the user's language to establish the goal, scope, constraints, exclusions and observable acceptance criteria. Use answers already given; do not repeat questions. Even a self-contained request needs a requirements checkpoint. Ask one question at a time using a fenced ```question JSON block: {\"id\":\"scope\",\"context\":\"Why this decision matters\",\"question\":\"What should we build?\",\"options\":[{\"label\":\"Option A\",\"description\":\"Tradeoff\"},{\"label\":\"Option B\",\"description\":\"Tradeoff\"}]}. Offer 2–6 concrete options, put the recommended option first, mark its label as recommended in the user's language and explain its tradeoff. Do not include an Other option: the UI always supplies a custom answer field. Do not combine a question with the requirements checkpoint in the same answer. When no more questions are needed, end with a fenced ```requirements Markdown summary containing those agreed requirements and any explicit assumptions. This completes the interview. In supervised mode, wait for explicit user approval before planning; in autonomous mode planning starts automatically. Do not emit a plan block, schedule work or implement anything. The UI offers approval of the requirements summary; the user can comment before proceeding.\n");
            } else if let Some(workflow) = &workflow {
                prompt.insert_str(0, &format!("\nSESSION PHASE: {}. Collected requirements:\n{}\nPrepare or revise the complete plan for review and comments. Never start implementation yourself; separate user plan approval is required. During implementation, follow progress and respond to results or questions; propose revisions for approval if necessary.\n", workflow.phase, workflow.summary));
            }
            if let Ok(Some(run)) = self.store.planning_run(conversation) {
                prompt.push_str(&self.ledger_context(conversation, &run.id).unwrap_or_default());
                prompt.push_str("\nThe runtime binds this plan to the active requirements run. Each task must include requirement_ids and criterion_ids. Every task serves at least one active requirement. Cover every mandatory criterion across the tasks; supporting tasks need not cover every criterion themselves. Use the reviewed implementation comparison and resolve its findings in the plan. Do not ask the user to copy run or option identifiers.\n");
            }
            prompt
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
