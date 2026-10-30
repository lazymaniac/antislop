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
        self.validate_agent_result(conversation, role, turn, &result)?;
        self.store.save_turn_result(turn, &result)?;
        // A turn that ends without waiting leaves no one to take an answer.
        if !matches!(result, TurnResult::NeedsInput { .. }) {
            self.store.invalidate_open_questions(turn)?;
        }
        Ok(json!({"recorded":true,"result":result}))
    }

    /// Refuses within the turn what `propose` would refuse after it: a graph
    /// that does not order, links to criteria the ledger does not have, or a
    /// task the runtime could not run as written.
    pub(super) fn check_submitted_plan(
        &self,
        conversation: &str,
        tasks: Vec<PlannedTask>,
        keep: &[String],
    ) -> Result<()> {
        let mut specs = self.submitted_graph(conversation, tasks, keep)?;
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
    pub(super) fn revised_tasks(
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
            .map(|task| Self::link_revised_task(task, &baseline, &entries))
            .collect()
    }

    pub(super) fn link_revised_task(
        task: PlannedTask,
        baseline: &[Task],
        entries: &[super::ledger::LedgerEntry],
    ) -> Result<TaskContract> {
        let mut spec = task.into_contract();
        restore_requirement_links(&mut spec, baseline)?;
        super::ledger::resolve_covers(&mut spec, entries)?;
        Ok(spec)
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

fn restore_requirement_links(spec: &mut TaskContract, baseline: &[Task]) -> Result<()> {
    if !spec.covers.is_empty() {
        return Ok(());
    }
    if let Some(old) = baseline.iter().find(|old| old.id == spec.id) {
        let old: TaskContract = serde_json::from_value(old.spec.clone())?;
        spec.requirement_ids = old.requirement_ids;
        spec.criterion_ids = old.criterion_ids;
    }
    Ok(())
}
