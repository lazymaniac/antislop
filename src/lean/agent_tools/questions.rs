use super::*;

impl Core {
    pub(super) fn require_open_agent_turn(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
    ) -> Result<()> {
        let open: bool = self.store.db().query_row(
            "SELECT EXISTS(SELECT 1 FROM turns WHERE id=?1 AND conversation_id=?2 AND role=?3 AND status='running')",
            rusqlite::params![turn, conversation, role],
            |row| row.get(0),
        )?;
        require(open, "turn is closed or belongs to another agent")?;
        if let Some((task, _)) = task_role(role) {
            self.require_current_task_generation(turn, task)?;
        }
        Ok(())
    }

    pub(super) fn require_current_task_generation(&self, turn: &str, task: &str) -> Result<()> {
        let current: bool = self.store.db().query_row(
            "SELECT x.generation=coalesce(i.generation,0) FROM turn_inputs x LEFT JOIN task_inputs i ON i.task_id=?2 WHERE x.turn_id=?1",
            rusqlite::params![turn, task],
            |row| row.get(0),
        ).optional()?.ok_or_else(|| Error::Invalid(format!("turn {turn}: missing task input binding; start a fresh task turn")))?;
        require(current, "task inputs changed; this turn is superseded")
    }

    pub(super) fn question_tool(
        self: &Arc<Self>,
        conversation: &str,
        role: &str,
        turn: &str,
        tool: &str,
        args: &Value,
    ) -> Result<Value> {
        require(
            task_role(role).is_some(),
            "only task agents may ask or wait",
        )?;
        if tool == "ask" {
            return self.ask_agent_tool(conversation, turn, args);
        }
        self.wait_for_answer_tool(conversation, role, turn, args)
    }

    pub(super) fn ask_agent_tool(
        self: &Arc<Self>,
        conversation: &str,
        turn: &str,
        args: &Value,
    ) -> Result<Value> {
        let question = required_text(args, "question")?;
        if let Some(previous) = self
            .store
            .local_questions(turn)?
            .into_iter()
            .find(|q| q["question"] == question && q["state"] == "answered")
        {
            return self.store.agent_question(
                conversation,
                previous["request"].as_str().unwrap_or_default(),
                Some(turn),
            );
        }
        let key = crate::hash(question.as_bytes())[..32].to_owned();
        let request = self
            .store
            .ask_agent_question(conversation, turn, &key, &question)?;
        self.wake(conversation);
        self.await_agent_answer(conversation, turn, &request)
    }

    pub(super) fn wait_for_answer_tool(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        args: &Value,
    ) -> Result<Value> {
        let request = match self.store.open_question_of_turn(turn)? {
            Some(request) => request,
            None => {
                if let Some(answer) = self
                    .store
                    .local_questions(turn)?
                    .into_iter()
                    .find(|q| q["state"] == "answered")
                {
                    let request = answer["request"].as_str().unwrap_or_default();
                    let answer = self
                        .store
                        .agent_question(conversation, request, Some(turn))?;
                    if answer["released"] != false {
                        return Ok(answer);
                    }
                    let mut response = self.record_task_result(
                        conversation,
                        role,
                        turn,
                        TurnResult::NeedsInput {
                            request: request.into(),
                            checkpoint: args["note"]
                                .as_str()
                                .unwrap_or(
                                    "Waiting for the revision or dependency named in the answer.",
                                )
                                .into(),
                        },
                    )?;
                    for key in [
                        "request",
                        "state",
                        "answer",
                        "disposition",
                        "target",
                        "released",
                    ] {
                        response[key] = answer[key].clone();
                    }
                    return Ok(response);
                }
                return Err(Error::Invalid(
                    "no question of this turn is waiting for an answer; ask one with ask".into(),
                ));
            }
        };
        let checkpoint = args["note"]
            .as_str()
            .filter(|note| !note.trim().is_empty())
            .unwrap_or("Waiting for the answer to my question.")
            .to_owned();
        self.record_task_result(
            conversation,
            role,
            turn,
            TurnResult::NeedsInput {
                request,
                checkpoint,
            },
        )
    }

    pub(super) fn finish_tool(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        tool: &str,
        args: &Value,
    ) -> Result<Value> {
        if tool == "finish_turn" && matches!(role, ORCHESTRATOR | COMMENT_ORCHESTRATOR) {
            self.require_plan_correction(conversation, turn)?;
        }
        if tool == "finish_turn" && role == PLANNER {
            self.require_refused_plan(conversation, turn)?;
        }
        self.record_result(turn, turn_result::tool_result(tool, args)?)
    }

    pub(super) fn interview_tool(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        tool: &str,
        args: &Value,
    ) -> Result<Value> {
        require(
            matches!(role, ORCHESTRATOR | COMMENT_ORCHESTRATOR),
            if tool == "ask_user" {
                "only the orchestrator asks the user"
            } else {
                "only the orchestrator summarizes requirements"
            },
        )?;
        require(
            self.store.workflow(conversation)?.interviewing() || tool == "ask_user",
            "the requirements interview is closed; answer with finish_turn or submit a plan",
        )?;
        self.record_result(turn, turn_result::tool_result(tool, args)?)
    }
}
