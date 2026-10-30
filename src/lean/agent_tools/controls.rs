use super::*;

impl Core {
    pub(super) fn orchestrator_tool(
        self: &Arc<Self>,
        conversation: &str,
        role: &str,
        tool: &str,
        args: &Value,
    ) -> Result<Value> {
        require(
            matches!(role, ORCHESTRATOR | COMMENT_ORCHESTRATOR),
            "only the orchestrator may use this control tool",
        )?;
        match tool {
            "answer_agent" => self.answer_agent_tool(conversation, args),
            "defer_to_user" => self.defer_question_tool(conversation, args),
            "retry_task" => self.retry_failed_task(
                conversation,
                &required_text(args, "task")?,
                &required_text(args, "guidance")?,
            ),
            _ => Err(Error::Invalid(format!("unknown agent tool {tool}"))),
        }
    }

    pub(super) fn answer_agent_tool(
        self: &Arc<Self>,
        conversation: &str,
        args: &Value,
    ) -> Result<Value> {
        let _graph = self.graph_updates.lock().expect("graph update mutex");
        let _integration = self.integrating.lock().expect("integration mutex");
        let reply = self.store.answer_agent_question(
            conversation,
            &required_text(args, "request")?,
            &required_text(args, "answer")?,
        )?;
        if let Some(task) = self.store.latest_plan(conversation)?.first() {
            self.maybe_continue_plan(conversation, &task.plan_id)?;
        }
        Ok(reply)
    }

    pub(super) fn defer_question_tool(&self, conversation: &str, args: &Value) -> Result<Value> {
        let _graph = self.graph_updates.lock().expect("graph update mutex");
        let _integration = self.integrating.lock().expect("integration mutex");
        let request = required_text(args, "request")?;
        let explanation = required_text(args, "explanation")?;
        let already =
            self.store.agent_question(conversation, &request, None)?["state"] == "awaiting_user";
        let reply = self
            .store
            .defer_question_to_user(conversation, &request, &explanation)?;
        if !already {
            self.notice(conversation, &format!(
                "A task is waiting for your decision (request {request}): {} Answer in the chat; the orchestrator passes your answer on.",
                tasks::bound(&explanation, 600)
            ));
        }
        Ok(reply)
    }

    pub(super) fn submit_plan_tool(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        args: &Value,
    ) -> Result<Value> {
        require(
            ORCHESTRATION_ROLES.contains(&role),
            "only the orchestrator and the planner submit plans",
        )?;
        require(
            !self.store.workflow(conversation)?.interviewing(),
            "the requirements interview is still open; finish it with the requirements summary before submitting a plan",
        )?;
        let tasks = planned_tasks(&args["tasks"])?;
        let result = turn_result::tool_result("submit_plan", args)?;
        let keep = match &result {
            TurnResult::Plan { keep, .. } => keep.clone(),
            _ => Vec::new(),
        };
        self.check_submitted_plan(conversation, tasks, &keep)?;
        self.record_result(turn, result)
    }

    pub(super) fn audit_tool(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        tool: &str,
        args: &Value,
    ) -> Result<Value> {
        let result = match tool {
            "submit_ledger" if role == "requirements-extraction-analyst" => TurnResult::Ledger {
                extraction: self.submitted_extraction(conversation, args)?,
            },
            "submit_audit" if role == "requirements-audit-analyst" => {
                let (findings, evidence) = self.submitted_audit(conversation, args)?;
                TurnResult::Audit { findings, evidence }
            }
            "submit_audit_review" if role == "requirements-audit-verifier" => {
                let (note, assessments) = self.submitted_audit_review(conversation, args)?;
                TurnResult::AuditReview { note, assessments }
            }
            _ => return Err(Error::Invalid("tool is unavailable for this role".into())),
        };
        self.record_result(turn, result)
    }

    pub(super) fn revision_tool(
        self: &Arc<Self>,
        conversation: &str,
        role: &str,
        tool: &str,
        args: &Value,
    ) -> Result<Value> {
        require(
            matches!(role, ORCHESTRATOR | COMMENT_ORCHESTRATOR),
            "only the orchestrator may revise the plan",
        )?;
        if tool == "request_replan" {
            return self.request_replan(conversation, &required_text(args, "reason")?);
        }
        self.revise_plan_tool(conversation, args)
    }

    pub(super) fn revise_plan_tool(
        self: &Arc<Self>,
        conversation: &str,
        args: &Value,
    ) -> Result<Value> {
        let (request_key, revision) = self.prepare_plan_revision(conversation, args)?;
        self.submit_revision(conversation, &request_key, revision)
    }

    fn prepare_plan_revision(
        &self,
        conversation: &str,
        args: &Value,
    ) -> Result<(String, super::store::PlanRevision)> {
        let (plan, base_revision) = self.revision_view(conversation)?;
        let request_key = revision_request_key(args)?;
        let tasks = self.revised_tasks(conversation, &plan, planned_tasks(&args["tasks"])?)?;
        Ok((
            request_key,
            super::store::PlanRevision {
                plan,
                base_revision,
                reason: required_text(args, "reason")?,
                tasks,
                retire: serde_json::from_value(args["retire"].clone())?,
                redo: serde_json::from_value(args.get("redo").cloned().unwrap_or(json!([])))?,
            },
        ))
    }

    fn revision_view(&self, conversation: &str) -> Result<(String, i64)> {
        self.revision_views
            .lock()
            .expect("revision views mutex")
            .get(conversation)
            .cloned()
            .flatten()
            .ok_or_else(|| {
                Error::Invalid(
                    "there is no plan to revise; submit a new plan with submit_plan".into(),
                )
            })
    }
}
