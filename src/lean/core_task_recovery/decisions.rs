//! Bound, user-selected recovery. No legacy/free-text decision becomes an override.
use super::*;

impl RecoveryScope {
    fn restriction(self) -> &'static str {
        match self {
            Self::EditsOnly => {
                "ONE continuation of source edits and analysis only. NO builds, tests, linters, controls, training or other project execution. Existing task inputs, permissions and cumulative execution budgets remain unchanged. Keep steps with missing prerequisites and all deferred work held."
            }
            Self::ExistingAuthorizedExecution => {
                "ONE continuation limited to the concrete request below AND the currently registered execution authorizations for this exact task, contract and input generation. Existing cumulative budgets, consumed invocations, stop conditions, monitoring and missing prerequisites remain binding. No new or renewed execution grant, budget, retry or deferred work. This separately approved continuation can perform the requested validation within those existing bounds; earlier turn-specific edit-only guidance is not its mode. Keep unavailable controls held."
            }
        }
    }

    pub(in crate::lean) fn event_scope(self) -> &'static str {
        match self {
            Self::EditsOnly => "no_project_execution",
            Self::ExistingAuthorizedExecution => "existing_authorized_execution",
        }
    }

    fn authorization_hash(self, core: &Core, task: &Task) -> Result<Option<String>> {
        if self == Self::EditsOnly {
            return Ok(None);
        }
        let authority = core.store.task_execution_approval_context(task)?;
        require(
            authority["plan_accepted"] == true
                && authority["scoped_execution_authorizations"]
                    .as_array()
                    .is_some_and(|grants| !grants.is_empty()),
            "validation recovery requires an existing execution authorization bound to this exact task, contract and input generation; it cannot create a grant",
        )?;
        Ok(Some(crate::hash(serde_json::to_vec(&authority)?)))
    }
}

impl Core {
    pub(in crate::lean) fn request_recovery_approval(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        name: &str,
        continuation: RecoveryContinuation<'_>,
    ) -> Result<serde_json::Value> {
        let RecoveryContinuation {
            expected,
            guidance,
            scope,
        } = continuation;
        let (_, task) = self.recovery_task(conversation, role, turn, name)?;
        require(
            task.spec["deferred"] != true
                && task.status
                    == if expected.is_some() {
                        status::COORDINATING
                    } else {
                        status::FAILED
                    },
            "recovery action must match a stopped non-deferred task; resume_task is coordinating-only and retry_task is failed-only",
        )?;
        if let Some(expected) = expected {
            require(
                (task.round, task.input_generation) == expected,
                "stale recovery request; refresh task_states",
            )?;
        }
        let prompt = match scope {
            RecoveryScope::EditsOnly => {
                "Allow one additional source-edit/analysis continuation without running the project?"
            }
            RecoveryScope::ExistingAuthorizedExecution => {
                "Allow one continuation of the requested validation using only the existing registered execution authorizations and remaining cumulative budget?"
            }
        };
        let args = json!({"context":format!("A single continuation of task {} requires your explicit choice.\n\nRequested continuation: {guidance}",task.spec["title"].as_str().unwrap_or(name)),"question":prompt,"options":["Resume once","Keep stopped"]});
        let turn_result::TurnResult::Question { mut question } =
            turn_result::tool_result("ask_user", &args)?
        else {
            unreachable!()
        };
        let binding = json!({"task":task.id,"round":task.round,"input_generation":task.input_generation,"guidance":guidance,"scope":scope});
        self.bind_recovery_question(conversation, role, turn, &binding, &mut question)?;
        Ok(json!({"approval_required":true,"question":question}))
    }

    fn bind_recovery_question(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        binding: &serde_json::Value,
        question: &mut interview::Question,
    ) -> Result<()> {
        let _graph = self.graph_updates.lock().expect("graph update mutex");
        let _scope = self.integrating.lock().expect("integration mutex");
        require(
            !self.store.workflow(conversation)?.interviewing(),
            "recovery decisions require an accepted plan",
        )?;
        let name = turn_result::required_text(binding, "task")?;
        let expected: (u32, i64) =
            serde_json::from_value(json!([binding["round"], binding["input_generation"]]))?;
        let continuation = turn_result::required_text(binding, "guidance")?;
        require(
            continuation.len() <= 8000,
            "recovery guidance exceeds 8000 bytes",
        )?;
        let (plan, task) = self.recovery_task(conversation, role, turn, &name)?;
        require(
            (task.round, task.input_generation) == expected,
            "stale recovery question; refresh task_states",
        )?;
        require(
            matches!(task.status.as_str(), status::COORDINATING | status::FAILED)
                && task.spec["deferred"] != true,
            "recovery decision requires a stopped non-deferred task",
        )?;
        self.store.assert_task_inputs(&task)?;
        require(
            question.options.len() == 2,
            "recovery decisions need exactly two options: approve once, then keep stopped",
        )?;
        require(
            question.options[0].label.trim() != question.options[1].label.trim(),
            "recovery approval and rejection labels must differ",
        )?;
        let token = self.task_freshness_token(conversation, &plan, &task.id)?;
        let scope: RecoveryScope = serde_json::from_value(binding["scope"].clone())?;
        let authorization_hash = scope.authorization_hash(self, &task)?;
        let restriction = scope.restriction();
        if authorization_hash.is_some() {
            let authority = self.store.task_execution_approval_context(&task)?;
            question.context.push_str(&format!(
                "\n\nExisting authority (not renewed): {}",
                authority
            ));
        }
        let guidance = format!("User-approved recovery. {restriction}\n\n{continuation}");
        question.context.push_str(&format!("\n\nRuntime recovery for task: {}. First option approves: {restriction} Second option keeps the task stopped.",task.spec["title"].as_str().unwrap_or(&name)));
        question.options[0]
            .description
            .push_str(&format!(" {restriction}"));
        question.options[1]
            .description
            .push_str(" No recovery will run.");
        question.id = crate::hash(serde_json::to_vec(&(
            question.clone(),
            &task.id,
            task.round,
            task.input_generation,
            &token,
            &guidance,
            scope,
            &authorization_hash,
        ))?)[..12]
            .to_owned();
        if let Some(Some(open)) = self.store.open_user_decision(conversation)? {
            require(
                open.id == question.id,
                "answer the existing open decision before requesting another recovery",
            )?;
        }
        self.store.bind_recovery_decision(
            conversation,
            &task,
            &token,
            &guidance,
            question,
            &RecoveryScopeBinding {
                scope,
                authorization_hash,
            },
        )
    }

    /// Called after trusted user input and on restart. Only previously bound
    /// requests with a captured answer participate; unrelated answers do not.
    pub(in crate::lean) fn apply_recovery_answers(
        self: &Arc<Self>,
        conversation: &str,
    ) -> Result<()> {
        for decision in self.store.answered_recovery_decisions(conversation)? {
            let _graph = self.graph_updates.lock().expect("graph update mutex");
            let _scope = self.integrating.lock().expect("integration mutex");
            let rejection = self.recovery_answer_rejection(&decision);
            let (applied, rejection) = self
                .store
                .settle_recovery_decision(&decision, rejection.as_deref())?;
            if applied {
                let task = self
                    .store
                    .task_by_id(&decision.task)?
                    .ok_or_else(|| Error::Invalid("recovery task disappeared".into()))?;
                self.announce_task(conversation, &task, None);
                // Queueing is already durable even if launching encounters a
                // runtime failure; pending guidance is recovered at startup.
                if let Err(error) = self.maybe_continue_plan(conversation, &decision.plan) {
                    self.notice(
                        conversation,
                        &format!("Approved recovery queued but could not start: {error}"),
                    );
                }
            } else if let Some(reason) = rejection {
                self.notice(conversation, &format!("Recovery not applied: {reason}"));
            }
        }
        Ok(())
    }

    fn recovery_answer_rejection(&self, decision: &store::RecoveryDecision) -> Option<String> {
        let check = || -> Result<()> {
            let approve = decision.question.options.first().ok_or_else(|| {
                Error::Invalid("bound recovery question has no approval option".into())
            })?;
            let expected = format!(
                "Interview answer ({})\nQuestion: {}\nAnswer: {}",
                decision.id, decision.question.question, approve.label
            );
            require(
                decision.answer.author == "user" && decision.answer.text == expected,
                "user did not select the bound one-time recovery approval; task left stopped",
            )?;
            require(
                self.store.accepted_plan(&decision.conversation)?.as_deref()
                    == Some(&decision.plan),
                "the approved recovery plan was replaced",
            )?;
            let task = self
                .store
                .task_by_id(&decision.task)?
                .ok_or_else(|| Error::Invalid("recovery task no longer exists".into()))?;
            require(
                task.plan_id == decision.plan
                    && task.input_generation == decision.generation
                    && task.round == decision.round,
                "task inputs or round changed after the recovery question",
            )?;
            require(
                matches!(task.status.as_str(), status::COORDINATING | status::FAILED)
                    && task.spec["deferred"] != true,
                "recovery task is no longer stopped or was deferred",
            )?;
            self.store.assert_task_inputs(&task)?;
            require(
                decision.scope.authorization_hash(self, &task)? == decision.authorization_hash,
                "execution authorization changed after the recovery preview; task left stopped",
            )?;
            require(
                self.task_freshness_token(&decision.conversation, &decision.plan, &task.id)?
                    == decision.token,
                "recovery preview is stale; task left stopped",
            )
        };
        check().err().map(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::recovery_fixture;
    use super::*;
    use crate::lean::turn_result::TurnResult;

    fn question(
        core: &Arc<Core>,
        conversation: &str,
        task: &Task,
        turn: &store::Turn,
        bound: bool,
    ) -> interview::Question {
        question_with_scope(
            core,
            conversation,
            task,
            turn,
            bound,
            RecoveryScope::EditsOnly,
        )
    }

    fn question_with_scope(
        core: &Arc<Core>,
        conversation: &str,
        task: &Task,
        turn: &store::Turn,
        bound: bool,
        scope: RecoveryScope,
    ) -> interview::Question {
        let question = if bound {
            let gateway = mcp::Gateway::start().unwrap();
            gateway.set_role(ORCHESTRATOR).unwrap();
            gateway.begin(&turn.id).unwrap();
            let handler_core = core.clone();
            let handler_conversation = conversation.to_owned();
            let handler_turn = turn.id.clone();
            gateway
                .set_handler(Arc::new(move |tool, args| {
                    handler_core.agent_tool(
                        &handler_conversation,
                        ORCHESTRATOR,
                        &handler_turn,
                        tool,
                        args,
                    )
                }))
                .unwrap();
            let guidance = if scope == RecoveryScope::EditsOnly {
                "Finish the remaining readonly graph projection; keep missing-reference validation held"
            } else {
                "Use one remaining registered build and controls 1-2 under the existing cumulative limits; hold control 3 for missing input, no retries"
            };
            let (name, mut args) = if task.status == status::FAILED {
                (
                    "retry_task",
                    json!({"turn":turn.id,"task":task.id,"guidance":guidance}),
                )
            } else {
                (
                    "resume_task",
                    json!({"turn":turn.id,"task":task.id,"round":task.round,"input_generation":task.input_generation,"resolution":guidance}),
                )
            };
            args["scope"] = serde_json::to_value(scope).unwrap();
            let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}});
            for _ in 0..2 {
                let reply: serde_json::Value = ureq::post(gateway.url())
                    .header("authorization", format!("Bearer {}", gateway.token()))
                    .send_json(&request)
                    .unwrap()
                    .body_mut()
                    .read_json()
                    .unwrap();
                assert_ne!(reply["result"]["isError"], true, "{reply}");
            }
            let finish = gateway
                .wait(
                    &turn.id,
                    &AtomicBool::new(false),
                    std::time::Duration::from_secs(2),
                    || Ok(()),
                )
                .unwrap();
            let TurnResult::Question { question } = serde_json::from_str(&finish.text).unwrap()
            else {
                panic!("the exhausted control must end its own turn with a bound question")
            };
            question
        } else {
            let args = json!({"context":"Continue the preserved implementation only","question":"Resume once without running the project?","options":["Approve once","Keep stopped"]});
            let reply = core
                .agent_tool(conversation, ORCHESTRATOR, &turn.id, "ask_user", &args)
                .unwrap();
            let TurnResult::Question { question } =
                serde_json::from_value(reply["result"].clone()).unwrap()
            else {
                panic!("expected a question")
            };
            question
        };
        core.store
            .finish_turn(&turn.id, store::TurnStatus::Done, None)
            .unwrap();
        // The normal orchestration finish persists generic questions; bound
        // questions must already be durable before the MCP call returns.
        if !bound {
            core.store
                .ask_user_decision(conversation, &question)
                .unwrap();
        }
        question
    }

    fn answer(question: &interview::Question, selected: &str) -> String {
        format!(
            "Interview answer ({})\nQuestion: {}\nAnswer: {selected}",
            question.id, question.question
        )
    }

    fn exhaust(core: &Core, conversation: &str, task: &Task) {
        for _ in 0..2 {
            core.store
                .append_event(
                    conversation,
                    None,
                    Some(&task.plan_id),
                    "task_recovery",
                    &json!({"actor":"orchestrator","task":task.id}),
                )
                .unwrap();
        }
        assert!(
            core.assert_orchestrator_recovery_budget(conversation, &task.id)
                .is_err()
        );
    }

    fn registered_grant(core: &Core, conversation: &str, task: &Task) {
        let authority = core.store.task_execution_approval_context(task).unwrap();
        let seq = core.store.messages_by(conversation, "user", 0).unwrap()[0].seq;
        core.store.db().execute("INSERT INTO execution_authorizations(decision_id,conversation_id,plan_id,task_id,local_task,generation,requirement_id,requirement_version,contract_hash,answer_seq,base_revision,created_ms) VALUES('registered-grant',?1,?2,?3,?4,?5,'R1',1,?6,?7,0,0)",
            rusqlite::params![conversation,task.plan_id,task.id,task.spec["id"].as_str().unwrap(),task.input_generation,authority["contract_hash"].as_str().unwrap(),seq as i64]).unwrap();
    }

    #[test]
    fn validation_recovery_is_explicit_bound_and_never_renews_authority() {
        for capped in [false, true] {
            let (_directory, core, conversation, mut task, turn) = recovery_fixture();
            task.status = status::COORDINATING.into();
            core.store.update_task(&task).unwrap();
            registered_grant(&core, &conversation, &task);
            if capped {
                exhaust(&core, &conversation, &task);
            }
            let authority = core.store.task_execution_approval_context(&task).unwrap();
            let question = question_with_scope(
                &core,
                &conversation,
                &task,
                &turn,
                true,
                RecoveryScope::ExistingAuthorizedExecution,
            );
            assert!(question.question.contains("requested validation"));
            assert!(question.context.contains("registered-grant"));
            assert!(!question.context.contains("NO builds"));
            assert_eq!(
                core.store.task_by_id(&task.id).unwrap().unwrap().status,
                status::COORDINATING,
                "the mode itself must never start work, even below the automatic limit"
            );
            let answer = answer(&question, &question.options[0].label);
            core.send(&conversation, &answer).unwrap();
            let queued = core.store.task_by_id(&task.id).unwrap().unwrap();
            assert_eq!(queued.status, status::QUEUED);
            assert_eq!(
                (queued.spec, queued.input_generation),
                (task.spec.clone(), task.input_generation)
            );
            assert_eq!(
                core.store.task_execution_approval_context(&task).unwrap(),
                authority
            );
            let guidance = core
                .store
                .pending_mail(&conversation, &format!("worker:{}", task.id))
                .unwrap();
            assert!(
                guidance
                    .iter()
                    .any(|m| m.text.contains("requested validation")
                        && m.text.contains("No new or renewed execution grant")
                        && !m.text.contains("NO builds"))
            );
            core.store.update_task(&task).unwrap();
            core.send(&conversation, &answer).unwrap();
            assert_eq!(
                core.store.task_by_id(&task.id).unwrap().unwrap().status,
                status::COORDINATING
            );
            assert_eq!(core.store.db().query_row("SELECT count(*) FROM events WHERE kind='task_recovery' AND json_extract(payload,'$.scope')='existing_authorized_execution'",[],|r| r.get::<_,i64>(0)).unwrap(),1);
            if capped {
                assert!(
                    core.assert_orchestrator_recovery_budget(&conversation, &task.id)
                        .is_err()
                );
            }
        }
    }

    #[test]
    fn validation_recovery_rejects_missing_or_changed_registered_grant() {
        for changed in [false, true] {
            let (_directory, core, conversation, mut task, turn) = recovery_fixture();
            task.status = status::COORDINATING.into();
            core.store.update_task(&task).unwrap();
            if !changed {
                let error = core.agent_tool(&conversation,ORCHESTRATOR,&turn.id,"resume_task",&json!({"task":task.id,"round":task.round,"input_generation":task.input_generation,"scope":"existing_authorized_execution","resolution":"Use existing validation only"})).unwrap_err().to_string();
                assert!(
                    error.contains("requires an existing execution authorization"),
                    "{error}"
                );
                assert!(
                    core.store
                        .open_user_decision(&conversation)
                        .unwrap()
                        .is_none()
                );
            } else {
                registered_grant(&core, &conversation, &task);
                let question = question_with_scope(
                    &core,
                    &conversation,
                    &task,
                    &turn,
                    true,
                    RecoveryScope::ExistingAuthorizedExecution,
                );
                core.store.db().execute("UPDATE execution_authorizations SET requirement_version=2 WHERE decision_id='registered-grant'",[]).unwrap();
                core.send(
                    &conversation,
                    &answer(&question, &question.options[0].label),
                )
                .unwrap();
                assert_eq!(
                    core.store.task_by_id(&task.id).unwrap().unwrap().status,
                    status::COORDINATING
                );
                assert_eq!(
                    core.store
                        .db()
                        .query_row(
                            "SELECT state FROM recovery_decisions WHERE decision_id=?1",
                            [&question.id],
                            |r| r.get::<_, String>(0)
                        )
                        .unwrap(),
                    "rejected"
                );
            }
        }
    }

    #[test]
    fn legacy_edit_approval_is_never_promoted_to_validation() {
        let (_directory, core, conversation, task, turn) = recovery_fixture();
        registered_grant(&core, &conversation, &task);
        exhaust(&core, &conversation, &task);
        let question = question(&core, &conversation, &task, &turn, true);
        // Emulate a pre-scope database: the old question remains edit-only.
        core.store
            .db()
            .execute("DELETE FROM recovery_scopes", [])
            .unwrap();
        core.send(
            &conversation,
            &answer(&question, &question.options[0].label),
        )
        .unwrap();
        let guidance = core
            .store
            .pending_mail(&conversation, &format!("worker:{}", task.id))
            .unwrap();
        assert!(guidance.iter().any(|m| m.text.contains("NO builds")));
        assert_eq!(core.store.db().query_row("SELECT json_extract(payload,'$.scope') FROM events WHERE kind='task_recovery' ORDER BY seq DESC LIMIT 1",[],|r|r.get::<_,String>(0)).unwrap(),"no_project_execution");
    }

    #[test]
    fn bound_user_answer_queues_exactly_once_after_the_recovery_limit() {
        let (_directory, core, conversation, mut task, turn) = recovery_fixture();
        task.status = status::COORDINATING.into();
        core.store.update_task(&task).unwrap();
        exhaust(&core, &conversation, &task);
        let question = question(&core, &conversation, &task, &turn, true);
        assert_eq!(
            core.store
                .db()
                .query_row(
                    "SELECT count(*) FROM recovery_decisions WHERE state='pending'",
                    [],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        let text = answer(&question, &question.options[0].label);
        core.store
            .add_message(&conversation, ORCHESTRATOR, &text)
            .unwrap();
        core.apply_recovery_answers(&conversation).unwrap();
        assert_eq!(
            core.store.task_by_id(&task.id).unwrap().unwrap().status,
            task.status,
            "neither the question nor an assistant-authored answer may start work"
        );
        assert!(
            core.store
                .pending_mail(&conversation, &format!("worker:{}", task.id))
                .unwrap()
                .is_empty()
        );
        core.send(&conversation, &text).unwrap();
        let queued = core.store.task_by_id(&task.id).unwrap().unwrap();
        assert_eq!(queued.status, status::QUEUED);
        assert_eq!(queued.spec, task.spec);
        assert_eq!(queued.round, task.round);
        assert_eq!(queued.input_generation, task.input_generation);
        let guidance = core
            .store
            .pending_mail(&conversation, &format!("worker:{}", task.id))
            .unwrap();
        assert!(
            guidance
                .iter()
                .any(|mail| mail.text.contains("NO builds, tests")
                    && mail.text.contains("Keep steps with missing prerequisites"))
        );
        core.store.update_task(&task).unwrap();
        core.send(&conversation, &text).unwrap();
        assert_eq!(
            core.store.task_by_id(&task.id).unwrap().unwrap().status,
            task.status
        );
        assert_eq!(core.store.db().query_row("SELECT count(*) FROM events WHERE kind='task_recovery' AND json_extract(payload,'$.source')='recovery_decision'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
        assert!(
            core.assert_orchestrator_recovery_budget(&conversation, &task.id)
                .is_err(),
            "a one-use approval must not reset the autonomous recovery counter"
        );
    }

    #[test]
    fn generic_approval_rejection_and_free_text_do_not_authorize_recovery() {
        for (bound, selected) in [(false, "Approve once"), (true, "Keep stopped")] {
            let (_directory, core, conversation, task, turn) = recovery_fixture();
            exhaust(&core, &conversation, &task);
            let question = question(&core, &conversation, &task, &turn, bound);
            core.send(&conversation, &answer(&question, selected))
                .unwrap();
            if bound {
                core.send(
                    &conversation,
                    &answer(&question, &question.options[0].label),
                )
                .unwrap();
            }
            assert_eq!(
                core.store.task_by_id(&task.id).unwrap().unwrap().status,
                status::FAILED
            );
            assert!(
                core.store
                    .pending_mail(&conversation, &format!("worker:{}", task.id))
                    .unwrap()
                    .is_empty()
            );
            assert!(
                core.assert_orchestrator_recovery_budget(&conversation, &task.id)
                    .is_err()
            );
        }
    }

    #[test]
    fn invalid_answer_keeps_the_card_open_and_can_be_corrected_after_restart() {
        let (_directory, core, conversation, task, turn) = recovery_fixture();
        exhaust(&core, &conversation, &task);
        let question = question(&core, &conversation, &task, &turn, true);
        let before = core.store.messages_by(&conversation, "user", 0).unwrap();
        for invalid in [
            answer(&question, "yes, generally"),
            format!(
                "Interview answer ({})\nQuestion: wrong question\nAnswer: {}",
                question.id, question.options[0].label
            ),
        ] {
            let error = core.send(&conversation, &invalid).unwrap_err().to_string();
            assert!(error.contains("decision remains open"), "{error}");
            assert_eq!(
                core.store
                    .messages_by(&conversation, "user", 0)
                    .unwrap()
                    .len(),
                before.len()
            );
            assert_eq!(
                core.store.open_user_decision(&conversation).unwrap(),
                Some(Some(question.clone()))
            );
            assert!(
                core.store
                    .answered_recovery_decisions(&conversation)
                    .unwrap()
                    .is_empty()
            );
        }
        let state = core.state.clone();
        let config = core.config_snapshot().as_ref().clone();
        drop(core);
        let core = Core::open(&state, config).unwrap();
        assert!(core.is_idle(&conversation));
        // Suppress only launching agents; the actual send/approval path runs.
        core.running
            .lock()
            .unwrap()
            .insert(conversation.clone(), Arc::new(AtomicBool::new(false)));
        core.plans
            .lock()
            .unwrap()
            .insert(conversation.clone(), Arc::new(AtomicBool::new(false)));
        let text = answer(&question, &question.options[0].label);
        core.send(&conversation, &text).unwrap();
        assert_eq!(
            core.store.task_by_id(&task.id).unwrap().unwrap().status,
            status::QUEUED
        );
        core.store.update_task(&task).unwrap();
        core.send(&conversation, &text).unwrap();
        assert_eq!(
            core.store.task_by_id(&task.id).unwrap().unwrap().status,
            task.status
        );
        assert_eq!(core.store.db().query_row("SELECT count(*) FROM events WHERE kind='task_recovery' AND json_extract(payload,'$.source')='recovery_decision'", [], |row| row.get::<_, i64>(0)).unwrap(), 1);
    }

    #[test]
    fn cold_restart_keeps_a_blocked_and_deferred_queue_idle() {
        let (_directory, core, conversation, original, turn) = recovery_fixture();
        core.store
            .finish_turn(&turn.id, store::TurnStatus::Done, None)
            .unwrap();
        let mut producer = original.spec.clone();
        producer["depends_on"] = json!([]);
        let mut consumer = producer.clone();
        consumer["id"] = "consumer".into();
        consumer["depends_on"] = json!([producer["id"]]);
        let mut future = producer.clone();
        future["id"] = "future".into();
        future["deferred"] = true.into();
        let mut tasks = core
            .store
            .add_plan(&conversation, &[producer, consumer, future])
            .unwrap();
        let plan = tasks[0].plan_id.clone();
        let run = core.store.plan_run(&original.plan_id).unwrap().unwrap();
        core.store
            .link_plan_run(&conversation, &plan, &run)
            .unwrap();
        core.store
            .bind_task_ledger(&conversation, &run, &plan)
            .unwrap();
        core.store.queue_plan(&conversation, &plan).unwrap();
        tasks = core.store.plan(&plan).unwrap();
        tasks[0].status = status::COORDINATING.into();
        core.store.update_task(&tasks[0]).unwrap();
        assert!(!core.has_runnable_dependents(&plan).unwrap());
        let before = core.store.latest_event_seq(&conversation).unwrap();
        let state = core.state.clone();
        let config = core.config_snapshot().as_ref().clone();
        drop(core);
        let core = Core::open(&state, config).unwrap();
        assert!(
            core.is_idle(&conversation),
            "blocked queue must not start a plan or coordinator"
        );
        assert_eq!(
            core.store
                .last_turn(&conversation, ORCHESTRATOR)
                .unwrap()
                .unwrap()
                .id,
            turn.id
        );
        assert_eq!(core.store.latest_event_seq(&conversation).unwrap(), before);
        for task in &tasks {
            assert_eq!(
                core.store.task_by_id(&task.id).unwrap().unwrap().status,
                task.status
            );
        }
        // Once the real prerequisite is satisfied, the shared scheduler
        // predicate still recognizes the consumer as runnable.
        tasks[0].status = status::APPROVED.into();
        core.store.update_task(&tasks[0]).unwrap();
        assert!(core.has_runnable_dependents(&plan).unwrap());
        // Legacy persisted specs omit the default empty dependency list.
        // Treat them like the scheduler does, rather than silently parking
        // otherwise runnable work after a restart.
        tasks[0].status = status::COORDINATING.into();
        core.store.update_task(&tasks[0]).unwrap();
        core.store
            .db()
            .execute(
                "UPDATE tasks SET spec=json_remove(spec,'$.depends_on') WHERE id=?1",
                [&tasks[1].id],
            )
            .unwrap();
        assert!(core.has_runnable_dependents(&plan).unwrap());
    }

    #[test]
    fn captured_approval_survives_reopen_but_stale_inputs_do_not_resume() {
        for (scope, stale) in [
            RecoveryScope::EditsOnly,
            RecoveryScope::ExistingAuthorizedExecution,
        ]
        .into_iter()
        .flat_map(|scope| {
            ["none", "generation", "round", "deferred", "superseded"]
                .into_iter()
                .map(move |stale| (scope, stale))
        }) {
            let (_directory, core, conversation, task, turn) = recovery_fixture();
            exhaust(&core, &conversation, &task);
            if scope == RecoveryScope::ExistingAuthorizedExecution {
                registered_grant(&core, &conversation, &task);
            }
            let question = question_with_scope(&core, &conversation, &task, &turn, true, scope);
            // Crash point: the trusted user message was committed, while the
            // Core callback has not yet run. Reopen the store before recovery.
            core.store
                .add_message(
                    &conversation,
                    "user",
                    &answer(&question, &question.options[0].label),
                )
                .unwrap();
            if stale == "generation" {
                core.store
                    .db()
                    .execute(
                        "UPDATE task_inputs SET generation=generation+1 WHERE task_id=?1",
                        [&task.id],
                    )
                    .unwrap();
            } else if stale == "round" {
                core.store
                    .db()
                    .execute("UPDATE tasks SET round=round+1 WHERE id=?1", [&task.id])
                    .unwrap();
            } else if stale == "deferred" {
                core.store.db().execute("UPDATE tasks SET spec=json_set(spec,'$.deferred',json('true')) WHERE id=?1",[&task.id]).unwrap();
            } else if stale == "superseded" {
                let mut newer = question.clone();
                newer.id = "newer-decision".into();
                core.store.ask_user_decision(&conversation, &newer).unwrap();
            }
            let state = core.state.clone();
            let mut config = core.config_snapshot().as_ref().clone();
            // An unrelated startup turn must not invoke a real coding service.
            for profile in config.profiles.values_mut() {
                profile.binary = Some("/bin/false".into());
                profile.command = None;
                profile.args.clear();
            }
            drop(core);
            let core = Core::open(&state, config).unwrap();
            assert_eq!(
                core.store.task_by_id(&task.id).unwrap().unwrap().status,
                if stale != "none" {
                    status::FAILED
                } else {
                    status::QUEUED
                }
            );
            assert!(
                core.store
                    .answered_recovery_decisions(&conversation)
                    .unwrap()
                    .is_empty()
            );
        }
    }
}
