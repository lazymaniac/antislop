use super::*;

impl Core {
    /// Records a result checked in full by its tool; the gateway ends the
    /// turn with exactly this result.
    pub(super) fn record_result(&self, turn: &str, result: TurnResult) -> Result<Value> {
        self.store.save_turn_result(turn, &result)?;
        Ok(json!({"recorded":true,"result":result}))
    }

    /// Checks a task result against the task's contract and records it; the
    /// gateway ends the turn with exactly this result.
    pub(super) fn record_task_result(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        result: TurnResult,
    ) -> Result<Value> {
        self.record_task_result_with_evidence(conversation, role, turn, result, &[])
    }

    pub(super) fn record_task_result_with_evidence(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        result: TurnResult,
        evidence: &[store::EvidenceReference],
    ) -> Result<Value> {
        self.validate_agent_result(conversation, role, turn, &result)?;
        self.store
            .save_turn_result_with_evidence(turn, &result, evidence)?;
        // A turn that ends without waiting leaves no one to take an answer.
        if !matches!(result, TurnResult::NeedsInput { .. }) {
            self.store.invalidate_open_questions(turn)?;
        }
        Ok(json!({"recorded":true,"result":result}))
    }

    /// Refuses within the turn what `propose` would refuse after it: a graph
    /// that does not order, missing acceptance criteria, or a
    /// task the runtime could not run as written.
    pub(super) fn check_submitted_plan(
        &self,
        conversation: &str,
        tasks: Vec<PlannedTask>,
        keep: &[String],
        team: crate::lean::tasks::Team,
    ) -> Result<()> {
        let specs = self.submitted_graph(conversation, tasks, keep)?;
        let specs = self.checked_candidate_plan(conversation, &specs, team, &[])?;
        self.store.validate_result_inputs(conversation, &specs)
    }

    pub(super) fn await_agent_answer(
        &self,
        conversation: &str,
        turn: &str,
        request: &str,
    ) -> Result<Value> {
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

    pub(super) fn validate_agent_result(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        result: &TurnResult,
    ) -> Result<()> {
        if let Some((id, reviewer)) = task_role(role) {
            let task = self
                .store
                .task_by_id(id)?
                .ok_or_else(|| Error::Invalid("task no longer exists".into()))?;
            self.store.assert_task_inputs(&task)?;
            let spec: TaskContract = serde_json::from_value(task.spec.clone())?;
            result.validate_task(&spec, reviewer)?;
            if reviewer && let TurnResult::Review { verdict, .. } = result {
                self.check_review_evidence(conversation, turn, &task, verdict)?;
            }
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
