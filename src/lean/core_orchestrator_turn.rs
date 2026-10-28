use super::*;
mod context;
mod prompt;
use super::core_plan_answer::PlanAnswer;
use super::core_planning_context::planning_ledger_context;
use super::core_unanswered::includes_comment;
use context::comment_context;
use prompt::{PromptContext, orchestrator_prompt_text};

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
        let asked = self.asked_since(conversation, answered, upto)?;
        let is_comment = includes_comment(&asked);
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
        let interviewing = workflow.interviewing();
        if workflow.planning() {
            self.store.append_event(
                conversation,
                None,
                None,
                "planning_progress",
                &json!({"text": "Working on the plan draft"}),
            )?;
        }
        let graph = self.store.latest_plan(conversation)?;
        let editable_graph = !graph.is_empty();
        // `revise_plan` edits the graph this turn was shown, at the revision
        // it was shown at; a revision applied meanwhile makes it stale.
        let view = graph
            .first()
            .map(|task| {
                Ok::<_, Error>((
                    task.plan_id.clone(),
                    self.store.plan_revision(&task.plan_id)?,
                ))
            })
            .transpose()?;
        self.revision_views
            .lock()
            .expect("revision views mutex")
            .insert(conversation.to_owned(), view);
        let graph_json = serde_json::to_string(
            &json!({"base_revision":graph.first().map(|t|self.store.plan_revision(&t.plan_id)).transpose()?,"tasks":graph.iter().map(|task|&task.spec).collect::<Vec<_>>()}),
        )?;
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
                workflow: &workflow,
                ledger_context: ledger_context.as_deref(),
            })
        })?
        else {
            return Ok(());
        };

        // A regenerated plan can carry a fresh selective claim too, including
        // after a deferred original comment revision.
        let result = super::turn_result::TurnResult::decode(&answer);
        let addressed = result.addressed().map(<[u64]>::to_vec);
        let text = result.text();
        let mut plan = None;
        let mut awaiting_decision = false;
        match result {
            super::turn_result::TurnResult::Plan { tasks, .. } => {
                plan = Some(tasks::planned_graph(tasks));
            }
            super::turn_result::TurnResult::Question { question } if interviewing => {
                self.store.ask_interview_question(conversation, &question)?;
            }
            // A question never submits a plan or changes existing permissions.
            super::turn_result::TurnResult::Question { question } => {
                self.store.ask_user_decision(conversation, &question)?;
                awaiting_decision = true;
            }
            super::turn_result::TurnResult::Requirements { markdown } if interviewing => {
                self.store
                    .interview_summary(conversation, &markdown, upto)?;
            }
            _ => {}
        }
        if interviewing && plan.take().is_some() {
            self.notice(
                conversation,
                "plan withheld during interview; planning starts after the requirements summary is complete",
            );
        }
        if workflow.planning() && !awaiting_decision {
            self.store.append_event(conversation, None, None, "planning_progress", &json!({"text": if plan.as_ref().is_some_and(|p| p.is_ok()) { "Plan draft ready" } else { "Plan draft requires correction" }}))?;
        }
        self.store.add_message(conversation, ORCHESTRATOR, &text)?;
        if interviewing && !cancel.load(Ordering::SeqCst) && self.autonomous(conversation)? {
            self.store.finish_interview(conversation)?;
        }
        self.handle_plan_answer(
            PlanAnswer {
                conversation,
                project: &project,
                config: &config,
                text: &text,
                plan,
                addressed: addressed.as_deref(),
                is_comment,
                answered,
                upto,
            },
            cancel,
        )
    }
}
