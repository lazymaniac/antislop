//! Session-bound MCP commands. Waiting never owns a store or scheduler lock.
use super::turn_result::TurnResult;
use super::*;
use rusqlite::OptionalExtension;
use serde_json::Value;

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
            "task_risks" => {
                require(
                    ORCHESTRATION_ROLES.contains(&role),
                    "only the orchestrator and the planner select task risk coverage",
                )?;
                let kind: WorkKind = serde_json::from_value(args["kind"].clone())?;
                Ok(
                    json!({"kind":kind,"risks":super::risks::for_kind(kind).map(|(id,text)|json!({"id":id,"text":text})).collect::<Vec<_>>() }),
                )
            }
            "ask_orchestrator" => {
                require(
                    task_role(role).is_some(),
                    "only task workers and verifiers may ask",
                )?;
                let request = self.store.ask_agent_question(
                    conversation,
                    turn,
                    &required("request_key")?,
                    &required("question")?,
                )?;
                self.wake(conversation);
                Ok(json!({"request":request,"state":"pending"}))
            }
            "await_orchestrator" => {
                require(
                    task_role(role).is_some(),
                    "only task workers and verifiers may await answers",
                )?;
                self.await_agent_answer(conversation, turn, &required("request")?)
            }
            "reply_to_agent" => {
                let _graph = self.graph_updates.lock().expect("graph update mutex");
                let _integration = self.integrating.lock().expect("integration mutex");
                require(
                    role == ORCHESTRATOR || role == COMMENT_ORCHESTRATOR,
                    "only the orchestrator may answer",
                )?;
                require(
                    args["state"].is_null()
                        || args["state"] == "answered"
                        || args["state"] == "awaiting_user",
                    "state: expected answered or awaiting_user",
                )?;
                if args["state"] == "awaiting_user" {
                    return self.store.defer_question_to_user(
                        conversation,
                        &required("request")?,
                        &required("answer")?,
                    );
                }
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
            "submit_plan_revision" => {
                require(
                    role == ORCHESTRATOR || role == COMMENT_ORCHESTRATOR,
                    "only the orchestrator may revise the plan",
                )?;
                self.submit_revision(
                    conversation,
                    &required("request_key")?,
                    serde_json::from_value(args["revision"].clone())?,
                )
            }
            "request_replan" => {
                require(
                    role == ORCHESTRATOR || role == COMMENT_ORCHESTRATOR,
                    "only the orchestrator may ask for a new plan",
                )?;
                self.request_replan(conversation, &required("reason")?)
            }
            "finish_turn" => {
                let result: TurnResult = serde_json::from_value(args["result"].clone())?;
                self.validate_agent_result(conversation, role, turn, &result)?;
                self.store.save_turn_result(turn, &result)?;
                Ok(json!({"recorded":true}))
            }
            _ => Err(Error::Invalid(format!("unknown agent tool {tool}"))),
        }
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
        } else {
            require(
                matches!(result, TurnResult::Report { .. }),
                "result.type: use report for this role",
            )?;
        }
        if matches!(role, ORCHESTRATOR | COMMENT_ORCHESTRATOR)
            && let TurnResult::Report { text } = result
            && let Some(question) = super::interview::parse(text)
        {
            question?;
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
