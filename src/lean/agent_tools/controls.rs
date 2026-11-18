use super::*;

impl Core {
    pub(super) fn orchestrator_tool(
        self: &Arc<Self>,
        conversation: &str,
        role: &str,
        turn: &str,
        tool: &str,
        args: &Value,
    ) -> Result<Value> {
        require(
            matches!(role, ORCHESTRATOR | COMMENT_ORCHESTRATOR),
            "only the orchestrator may use this control tool",
        )?;
        match tool {
            "answer_agent" => self.answer_agent_tool(conversation, turn, args),
            "defer_to_user" => self.defer_question_tool(conversation, args),
            "decide_approval" => self.decide_approval_tool(conversation, args),
            "retry_task" => self.retry_failed_task(
                conversation,
                role,
                turn,
                &required_text(args, "task")?,
                &required_text(args, "guidance")?,
            ),
            "resume_task" => {
                let expected: (u32, i64) =
                    serde_json::from_value(json!([args["round"], args["input_generation"]]))
                        .map_err(|error| {
                            Error::Invalid(format!("round/input_generation: {error}"))
                        })?;
                self.resume_coordinating_task(
                    conversation,
                    role,
                    turn,
                    &required_text(args, "task")?,
                    expected,
                    &required_text(args, "resolution")?,
                )
            }
            _ => Err(Error::Invalid(format!("unknown agent tool {tool}"))),
        }
    }

    pub(super) fn answer_agent_tool(
        self: &Arc<Self>,
        conversation: &str,
        turn: &str,
        args: &Value,
    ) -> Result<Value> {
        if args["disposition"] == "escalate_to_user" {
            return self.defer_question_tool(conversation, &json!({"request":required_text(args,"request")?,"explanation":required_text(args,"answer")?}));
        }
        let request = required_text(args, "request")?;
        let answer = required_text(args, "answer")?;
        let disposition = match args.get("disposition").filter(|value| !value.is_null()) {
            None => "continue",
            Some(value) => value.as_str().ok_or_else(|| Error::Invalid("disposition: expected continue, wait_for_revision, wait_for_dependency or escalate_to_user".into()))?,
        };
        if disposition == "escalate_to_user" {
            return self
                .store
                .defer_question_to_user(conversation, &request, &answer);
        }
        let reply = self.store.answer_agent_from_turn(
            conversation,
            &request,
            &answer,
            disposition,
            args["target"].as_str().unwrap_or_default(),
            Some(turn),
        )?;
        self.resume_delivered_answer(conversation);

        Ok(reply)
    }

    pub(in crate::lean) fn replay_delivered_answers(self: &Arc<Self>) -> Result<()> {
        let conversations = {
            let db = self.store.db();
            db.prepare(
                "SELECT DISTINCT conversation FROM agent_answer_receipts WHERE resume_pending=1",
            )?
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for conversation in conversations {
            self.resume_delivered_answer(&conversation);
        }
        Ok(())
    }

    pub(in crate::lean) fn resume_delivered_answer(self: &Arc<Self>, conversation: &str) {
        let core = self.clone();
        let conversation = conversation.to_owned();
        std::thread::spawn(move || {
            let result = (|| -> Result<()> {
                let upto: i64=core.store.db().query_row("SELECT coalesce(max(rowid),0) FROM agent_answer_receipts WHERE conversation=?1",[&conversation],|r|r.get(0))?;
                if let Some(plan) = core.store.accepted_plan(&conversation)? {
                    core.maybe_continue_plan(&conversation, &plan)?;
                }
                core.store.db().execute(
                    "UPDATE agent_answer_receipts SET resume_pending=0 WHERE conversation=?1 AND rowid<=?2",
                    rusqlite::params![conversation,upto],
                )?;
                Ok(())
            })();
            if let Err(error) = result {
                core.notice(
                    &conversation,
                    &format!("answer delivered; scheduler resume pending: {error}"),
                );
            }
        });
    }

    pub(super) fn defer_question_tool(&self, conversation: &str, args: &Value) -> Result<Value> {
        let _graph = self.locks.graph();
        let _integration = self.locks.integration();
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
        let _graph = self.locks.graph();
        let selected_by_user = self.store.user_selected_team(conversation)?;
        let saved_origin = self.store.team_origin(conversation)?;
        let (submitted_team, explicit_team) = match args["team"].as_str() {
            Some(name) => (Some(crate::lean::tasks::Team::parse(name)?), true),
            None => (None, false),
        };
        if let (Some(selected), Some(submitted)) = (selected_by_user, submitted_team) {
            require(
                selected == submitted,
                &format!(
                    "the user selected rigor `{}`; keep that team in submit_plan or ask the user to change it",
                    selected.name()
                ),
            )?;
        }
        // A recommendation is selected once per conversation. Redrafting a
        // plan cannot silently turn it into a different rigor team.
        let team = if saved_origin.is_some() {
            self.store.team(conversation)?
        } else {
            submitted_team.unwrap_or_default()
        };
        let reason = args["team_reason"].as_str().unwrap_or_default().trim();
        if explicit_team && saved_origin.is_none() {
            require(
                team == crate::lean::tasks::Team::default() || !reason.is_empty(),
                "team_reason is required with a team other than `feature`: say why this work needs no more gates",
            )?;
        }
        self.check_submitted_plan(conversation, tasks, &keep, team)?;
        if saved_origin.is_none() {
            self.store.select_planned_team(conversation, team, reason)?;
        }
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
                scope: if args["scope"].is_null() {
                    None
                } else {
                    Some(required_text(args, "scope")?)
                },
                plan,
                base_revision,
                reason: required_text(args, "reason")?,
                tasks,
                retire: serde_json::from_value(args["retire"].clone())
                    .map_err(|error| Error::Invalid(format!("retire: {error}")))?,
                replacements: serde_json::from_value(
                    args.get("replacements").cloned().unwrap_or(json!({})),
                )
                .map_err(|error| Error::Invalid(format!("replacements: {error}")))?,
                redo: serde_json::from_value(args.get("redo").cloned().unwrap_or(json!([])))
                    .map_err(|error| Error::Invalid(format!("redo: {error}")))?,
            },
        ))
    }

    fn revision_view(&self, conversation: &str) -> Result<(String, i64)> {
        self.revisions.view(conversation).ok_or_else(|| {
            Error::Invalid("there is no plan to revise; submit a new plan with submit_plan".into())
        })
    }
}
