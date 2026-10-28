//! Session-bound MCP commands. Waiting never owns a store or scheduler lock.
use super::tasks::PlannedTask;
use super::turn_result::TurnResult;
use super::*;
use rusqlite::OptionalExtension;
use serde_json::Value;

/// The tasks of `submit_plan` or `revise_plan`, with an error that names the
/// task and field the agent must fix.
fn planned_tasks(value: &Value) -> Result<Vec<PlannedTask>> {
    let items = value
        .as_array()
        .ok_or_else(|| Error::Invalid("tasks: expected an array of tasks".into()))?;
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            serde_json::from_value(item.clone()).map_err(|error| {
                let id = item["id"].as_str().unwrap_or("?");
                Error::Invalid(format!("tasks[{index}] ({id}): {error}"))
            })
        })
        .collect()
}

impl Core {
    pub(super) fn agent_tool(
        self: &Arc<Self>,
        conversation: &str,
        role: &str,
        turn: &str,
        tool: &str,
        args: &Value,
    ) -> Result<Value> {
        let open: bool = self.store.db().query_row("SELECT EXISTS(SELECT 1 FROM turns WHERE id=?1 AND conversation_id=?2 AND role=?3 AND status='running')",rusqlite::params![turn,conversation,role],|r|r.get(0))?;
        require(open, "turn is closed or belongs to another agent")?;
        if let Some((task, _)) = task_role(role) {
            let current: bool = self.store.db().query_row("SELECT x.generation=coalesce(i.generation,0) FROM turn_inputs x LEFT JOIN task_inputs i ON i.task_id=?2 WHERE x.turn_id=?1",rusqlite::params![turn,task],|r|r.get(0)).optional()?.ok_or_else(|| Error::Invalid(format!("turn {turn}: missing task input binding; start a fresh task turn")))?;
            require(current, "task inputs changed; this turn is superseded")?;
        }
        let required = |field: &str| {
            args[field]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .map(str::to_owned)
                .ok_or_else(|| Error::Invalid(format!("{field}: required")))
        };
        match tool {
            "ask" => {
                require(
                    task_role(role).is_some(),
                    "only task workers and verifiers may ask",
                )?;
                let question = required("question")?;
                // The same question asked again in the turn is the same request.
                let key = crate::hash(question.as_bytes())[..32].to_owned();
                let request = self
                    .store
                    .ask_agent_question(conversation, turn, &key, &question)?;
                self.wake(conversation);
                self.await_agent_answer(conversation, turn, &request)
            }
            "wait_for_answer" => {
                require(
                    task_role(role).is_some(),
                    "only task agents wait for answers",
                )?;
                let request = self.store.open_question_of_turn(turn)?.ok_or_else(|| {
                    Error::Invalid(
                        "no question of this turn is waiting for an answer; ask one with ask, or continue with the answers you have"
                            .into(),
                    )
                })?;
                let result = TurnResult::NeedsInput {
                    request,
                    checkpoint: args["note"]
                        .as_str()
                        .filter(|note| !note.trim().is_empty())
                        .unwrap_or("Waiting for the answer to my question.")
                        .to_owned(),
                };
                self.record_task_result(conversation, role, turn, result)
            }
            "complete" | "select" | "review" if task_role(role).is_some() => {
                let result = turn_result::task_tool_result(tool, args)?;
                self.record_task_result(conversation, role, turn, result)
            }
            "review" | "finish_turn" => {
                let result = turn_result::tool_result(tool, args)?;
                self.record_result(turn, result)
            }
            "ask_user" => {
                require(
                    role == ORCHESTRATOR || role == COMMENT_ORCHESTRATOR,
                    "only the orchestrator asks the user",
                )?;
                let result = turn_result::tool_result(tool, args)?;
                self.record_result(turn, result)
            }
            "submit_requirements" => {
                require(
                    role == ORCHESTRATOR || role == COMMENT_ORCHESTRATOR,
                    "only the orchestrator summarizes requirements",
                )?;
                require(
                    self.store.workflow(conversation)?.interviewing(),
                    "the requirements interview is closed; answer with finish_turn or submit a plan",
                )?;
                let result = turn_result::tool_result(tool, args)?;
                self.record_result(turn, result)
            }
            "answer_agent" => {
                let _graph = self.graph_updates.lock().expect("graph update mutex");
                let _integration = self.integrating.lock().expect("integration mutex");
                require(
                    role == ORCHESTRATOR || role == COMMENT_ORCHESTRATOR,
                    "only the orchestrator may answer",
                )?;
                let reply = self.store.answer_agent_question(
                    conversation,
                    &required("request")?,
                    &required("answer")?,
                )?;
                if let Some(task) = self.store.latest_plan(conversation)?.first() {
                    self.maybe_continue_plan(conversation, &task.plan_id)?;
                }
                Ok(reply)
            }
            "defer_to_user" => {
                let _graph = self.graph_updates.lock().expect("graph update mutex");
                let _integration = self.integrating.lock().expect("integration mutex");
                require(
                    role == ORCHESTRATOR || role == COMMENT_ORCHESTRATOR,
                    "only the orchestrator may hand a question to the user",
                )?;
                let request = required("request")?;
                let explanation = required("explanation")?;
                let already = self.store.agent_question(conversation, &request, None)?["state"]
                    == "awaiting_user";
                let reply =
                    self.store
                        .defer_question_to_user(conversation, &request, &explanation)?;
                // Nothing else shows the user that a task waits for them; said
                // once, since the orchestrator may refresh its explanation.
                if !already {
                    self.notice(
                        conversation,
                        &format!(
                            "A task is waiting for your decision (request {request}): {} Answer in the chat; the orchestrator passes your answer on.",
                            tasks::bound(&explanation, 600)
                        ),
                    );
                }
                Ok(reply)
            }
            "submit_plan" => {
                require(
                    ORCHESTRATION_ROLES.contains(&role),
                    "only the orchestrator and the planner submit plans",
                )?;
                require(
                    !self.store.workflow(conversation)?.interviewing(),
                    "the requirements interview is still open; finish it with the requirements summary before submitting a plan",
                )?;
                // Each task's own error names it, before the plan as a whole
                // is decoded.
                let tasks = planned_tasks(&args["tasks"])?;
                let result = turn_result::tool_result(tool, args)?;
                self.check_submitted_plan(conversation, tasks)?;
                self.record_result(turn, result)
            }
            "submit_ledger" => {
                require(
                    role == "requirements-extraction-analyst",
                    "only the requirements analyst submits requirements",
                )?;
                let extraction = self.submitted_extraction(conversation, args)?;
                self.record_result(turn, TurnResult::Ledger { extraction })
            }
            "submit_audit" => {
                require(
                    role == "requirements-audit-analyst",
                    "only the requirements auditor submits an audit",
                )?;
                let (findings, evidence) = self.submitted_audit(conversation, args)?;
                self.record_result(turn, TurnResult::Audit { findings, evidence })
            }
            "submit_audit_review" => {
                require(
                    role == "requirements-audit-verifier",
                    "only the audit verifier reviews an audit",
                )?;
                let (note, assessments) = self.submitted_audit_review(conversation, args)?;
                self.record_result(turn, TurnResult::AuditReview { note, assessments })
            }
            "revise_plan" => {
                require(
                    role == ORCHESTRATOR || role == COMMENT_ORCHESTRATOR,
                    "only the orchestrator may revise the plan",
                )?;
                let (plan, base_revision) = self
                    .revision_views
                    .lock()
                    .expect("revision views mutex")
                    .get(conversation)
                    .cloned()
                    .flatten()
                    .ok_or_else(|| {
                        Error::Invalid(
                            "there is no plan to revise; submit a new plan with submit_plan".into(),
                        )
                    })?;
                let mut payload = args.clone();
                if let Some(fields) = payload.as_object_mut() {
                    fields.remove("turn");
                }
                // The same revision asked twice is one request.
                let request_key = crate::hash(serde_json::to_vec(&payload)?);
                let tasks =
                    self.revised_tasks(conversation, &plan, planned_tasks(&args["tasks"])?)?;
                let retire = serde_json::from_value(args["retire"].clone())?;
                self.submit_revision(
                    conversation,
                    &request_key,
                    super::store::PlanRevision {
                        plan,
                        base_revision,
                        reason: required("reason")?,
                        tasks,
                        retire,
                    },
                )
            }
            "request_replan" => {
                require(
                    role == ORCHESTRATOR || role == COMMENT_ORCHESTRATOR,
                    "only the orchestrator may ask for a new plan",
                )?;
                self.request_replan(conversation, &required("reason")?)
            }
            _ => Err(Error::Invalid(format!("unknown agent tool {tool}"))),
        }
    }

    /// Records a result checked in full by its tool; the gateway ends the
    /// turn with exactly this result.
    fn record_result(&self, turn: &str, result: TurnResult) -> Result<Value> {
        self.store.save_turn_result(turn, &result)?;
        Ok(json!({"recorded":true,"result":result}))
    }

    /// Checks a task result against the task's contract and records it; the
    /// gateway ends the turn with exactly this result.
    fn record_task_result(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        result: TurnResult,
    ) -> Result<Value> {
        self.validate_agent_result(conversation, role, turn, &result)?;
        self.store.save_turn_result(turn, &result)?;
        Ok(json!({"recorded":true,"result":result}))
    }

    /// Refuses within the turn what `propose` would refuse after it: a graph
    /// that does not order, links to criteria the ledger does not have, or a
    /// task the runtime could not run as written.
    fn check_submitted_plan(&self, conversation: &str, tasks: Vec<PlannedTask>) -> Result<()> {
        let mut specs = tasks::planned_graph(tasks)?;
        if let Some(run) = self.store.planning_run(conversation)? {
            let entries = self.store.ledger_entries(conversation, &run.id, None)?;
            if !entries.is_empty() {
                for spec in &mut specs {
                    super::ledger::resolve_covers(spec, &entries)?;
                }
            }
        }
        super::core_plan_proposal::checked_plan(&specs, &self.config_snapshot())
            .map(|_| ())
            .map_err(Error::Invalid)
    }

    /// The revised tasks as contracts linked to the plan's requirements. A
    /// task that names no `covers` keeps the links it had.
    fn revised_tasks(
        &self,
        conversation: &str,
        plan: &str,
        tasks: Vec<PlannedTask>,
    ) -> Result<Vec<TaskContract>> {
        let run = self
            .store
            .plan_run(plan)?
            .ok_or_else(|| Error::Invalid("plan has no requirements".into()))?;
        let entries = self.store.ledger_entries(conversation, &run, None)?;
        let baseline = self.store.plan(plan)?;
        tasks
            .into_iter()
            .map(|task| {
                let mut spec = task.into_contract();
                if spec.covers.is_empty()
                    && let Some(old) = baseline.iter().find(|old| old.id == spec.id)
                {
                    let old: TaskContract = serde_json::from_value(old.spec.clone())?;
                    spec.requirement_ids = old.requirement_ids;
                    spec.criterion_ids = old.criterion_ids;
                }
                super::ledger::resolve_covers(&mut spec, &entries)?;
                Ok(spec)
            })
            .collect()
    }

    fn await_agent_answer(&self, conversation: &str, turn: &str, request: &str) -> Result<Value> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(25);
        loop {
            let answer = self
                .store
                .agent_question(conversation, request, Some(turn))?;
            if answer["state"] != "pending" || std::time::Instant::now() >= deadline {
                return Ok(answer);
            }
            let open: bool = self
                .store
                .db()
                .query_row(
                    "SELECT status='running' FROM turns WHERE id=?1",
                    [turn],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or_else(|| Error::Invalid(format!("turn {turn}: no longer exists")))?;
            require(open, "turn is no longer running")?;
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    fn validate_agent_result(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        result: &TurnResult,
    ) -> Result<()> {
        if let Some((id, verifier)) = task_role(role) {
            let task = self
                .store
                .task_by_id(id)?
                .ok_or_else(|| Error::Invalid("task no longer exists".into()))?;
            self.store.assert_task_inputs(&task)?;
            let spec: TaskContract = serde_json::from_value(task.spec)?;
            result.validate_task(&spec, verifier)?;
        }
        if let TurnResult::NeedsInput { request, .. } = result {
            let question = self
                .store
                .agent_question(conversation, request, Some(turn))?;
            require(
                question["state"] != "invalidated",
                "request: question was invalidated; await a fresh task turn",
            )?;
        }
        Ok(())
    }
}
