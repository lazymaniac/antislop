//! Planner-owned preparation. Reports are retained once as ordinary agent turns.
use super::dependencies::page_text;
use super::*;
use crate::lean::prompts;

fn planning_kinds() -> impl Iterator<Item = WorkKind> {
    tasks::FEATURE_SPECIALISTS
        .iter()
        .filter(|agent| agent.kind != WorkKind::Delivery)
        .map(|agent| agent.kind)
}

struct Consultation<'a> {
    conversation: &'a str,
    role: &'a str,
    kind: WorkKind,
    upto_message: u64,
    question: &'a str,
    inputs: &'a [Value],
}

fn prerequisites(kind: WorkKind) -> &'static [WorkKind] {
    match kind {
        WorkKind::Dependencies => &[WorkKind::Research],
        WorkKind::Architecture => &[WorkKind::Analysis, WorkKind::Dependencies],
        WorkKind::TestDesign => &[WorkKind::Analysis, WorkKind::Architecture],
        _ => &[],
    }
}

impl Core {
    fn planning_basis(&self, conversation: &str) -> Result<String> {
        // Rejection wording and replan requests do not invalidate established facts.
        // Scope identity is semantic: attaching an unchanged scope preserves reports.
        // Changed repository evidence is handled by an explicit role refresh.
        let requirements = self.store.workflow(conversation)?.summary;
        let db = self.store.db();
        let scope: Option<String> = db.query_row(
            "SELECT s.summary FROM plan_scopes p JOIN scopes s ON s.id=p.scope_id WHERE p.plan_id=(SELECT plan_id FROM tasks WHERE conversation_id=?1 ORDER BY created_ms DESC,rowid DESC LIMIT 1) ORDER BY p.revision DESC LIMIT 1",
            [conversation], |row| row.get(0),
        ).optional()?;
        let sources = db
            .prepare(
                "SELECT hash FROM conversation_sources WHERE conversation_id=?1 ORDER BY hash",
            )?
            .query_map([conversation], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(crate::hash(serde_json::to_vec(&(
            &requirements,
            scope.as_deref().unwrap_or(&requirements),
            sources,
        ))?))
    }

    fn planning_request(
        &self,
        conversation: &str,
        basis: &str,
        kind: WorkKind,
    ) -> Result<Option<Value>> {
        let encoded: Option<String> = self.store.db().query_row(
            "SELECT payload FROM events WHERE conversation_id=?1 AND kind='planning_consultation' AND json_extract(payload,'$.basis')=?2 AND json_extract(payload,'$.kind')=?3 ORDER BY seq DESC LIMIT 1",
            rusqlite::params![conversation, basis, tasks::work_kind_name(kind)], |row| row.get(0),
        ).optional()?;
        let Some(encoded) = encoded else {
            return Ok(None);
        };
        let request: Value = serde_json::from_str(&encoded)?;
        for input in prerequisites(kind) {
            let current = self.planning_request(conversation, basis, *input)?;
            if !current.is_some_and(|current| {
                request["inputs"].as_array().is_some_and(|inputs| {
                    inputs
                        .iter()
                        .any(|input| input["reference"] == current["reference"])
                })
            }) {
                return Ok(None);
            }
        }
        Ok(Some(request))
    }

    pub(super) fn consult_planning(
        self: &Arc<Self>,
        conversation: &str,
        role: &str,
        turn: &str,
        args: &Value,
    ) -> Result<Value> {
        require(
            role == PLANNER,
            "only the planner may call planning specialists",
        )?;
        require(
            !self.store.workflow(conversation)?.interviewing(),
            "confirm scope before consulting planning specialists",
        )?;
        let kind: WorkKind = serde_json::from_value(args["kind"].clone())?;
        require(
            planning_kinds().any(|candidate| candidate == kind),
            "kind: expected research, analysis, dependencies, architecture or test_design",
        )?;
        let question = required_text(args, "question")?;
        let basis = self.planning_basis(conversation)?;
        let lock = self.locks.turn(
            conversation,
            &format!("consult:{basis}:{}", tasks::work_kind_name(kind)),
        );
        let _request = lock.lock().expect("planning consultation mutex");
        if let Some(request) = self.planning_request(conversation, &basis, kind)? {
            let result = self.read_planning_result(
                conversation,
                role,
                turn,
                &json!({"reference":request["reference"]}),
            )?;
            if args["refresh"] != true || result["state"] == "running" {
                return Ok(result);
            }
        }
        let mut inputs = Vec::new();
        for input in prerequisites(kind) {
            let request = self
                .planning_request(conversation, &basis, *input)?
                .ok_or_else(|| {
                    Error::Invalid(format!("consult {} first", tasks::work_kind_name(*input)))
                })?;
            let result = self.read_planning_result(
                conversation,
                role,
                turn,
                &json!({"reference":request["reference"]}),
            )?;
            require(
                result["state"] == "done",
                &format!(
                    "{} consultation must finish first",
                    tasks::work_kind_name(*input)
                ),
            )?;
            inputs.push(json!({"kind":input,"reference":request["reference"]}));
        }
        let reference = format!("planning:{}", crate::id());
        let specialist = format!(
            "planning-{}:{}",
            tasks::work_kind_name(kind),
            reference.trim_start_matches("planning:")
        );
        self.store.append_event(conversation, Some(turn), None, "planning_consultation", &json!({
            "reference":reference,"basis":basis,"kind":tasks::work_kind_name(kind),"role":specialist,"inputs":inputs
        }))?;
        let core = self.clone();
        let conversation = conversation.to_owned();
        let parent = turn.to_owned();
        let request_ref = reference.clone();
        let upto_message = self
            .store
            .last_turn(&conversation, PLANNER)?
            .map_or(0, |t| t.upto_message);
        let (registered, ready) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = (|| -> Result<()> {
                let operation = match core.operation(&conversation) {
                    Ok(operation) => operation,
                    Err(error) => {
                        let _ = registered.send(Err(error.to_string()));
                        return Err(error);
                    }
                };
                let _ = registered.send(Ok(()));
                core.run_planning_consultation(
                    Consultation {
                        conversation: &conversation,
                        role: &specialist,
                        kind,
                        upto_message,
                        question: &question,
                        inputs: &inputs,
                    },
                    &operation.cancel,
                )
            })();
            if let Err(error) = result {
                let _ = core.store.append_event(
                    &conversation,
                    Some(&parent),
                    None,
                    "planning_consultation_failed",
                    &json!({"reference":request_ref,"error":error.to_string()}),
                );
            }
        });
        ready
            .recv()
            .map_err(|_| {
                Error::Invalid("planning consultation worker stopped before registration".into())
            })?
            .map_err(Error::Invalid)?;
        Ok(json!({"reference":reference,"state":"running"}))
    }

    fn run_planning_consultation(
        self: &Arc<Self>,
        request: Consultation<'_>,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let Consultation {
            conversation,
            role,
            kind,
            upto_message,
            question,
            inputs,
        } = request;
        let config = self.config_snapshot();
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        let requirements = self.store.workflow(conversation)?.summary;
        let scope = crate::lean::core::orchestration::planning_context::planning_scope_context(
            self,
            conversation,
        )?
        .unwrap_or_default();
        let setup = TurnSetup {
            session_policy: SessionPolicy::Independent,
            conversation,
            plan_id: None,
            role: role.into(),
            profile: config.resolve(kind, RoleKind::Executor),
            cwd: project,
            add_dirs: vec![],
            upto_message,
        };
        let answer = self.turn(setup, cancel, |_| {
            format!(
                prompts::template!(RUN_PLANNING_CONSULTATION),
                tasks::work_kind_name(kind),
                crate::lean::prompt_context::untrusted(
                    prompts::CONFIRMED_REQUIREMENTS_LABEL,
                    &requirements
                ),
                scope,
                crate::lean::prompt_context::untrusted(prompts::PLANNER_QUESTION_LABEL, question),
                crate::lean::prompt_context::untrusted(
                    prompts::PLANNING_PREREQUISITES_LABEL,
                    &crate::lean::prompt_context::markdown(&inputs)
                ),
            )
        })?;
        if answer.is_none() {
            let error = self
                .store
                .last_turn(conversation, role)?
                .and_then(|t| t.error)
                .unwrap_or_else(|| "planning consultation interrupted".into());
            return Err(Error::Invalid(error));
        }
        Ok(())
    }

    pub(super) fn read_planning_result(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        args: &Value,
    ) -> Result<Value> {
        let reference = required_text(args, "reference")?;
        let (encoded, created): (String, i64) = self.store.db().query_row(
            "SELECT payload,created_ms FROM events WHERE conversation_id=?1 AND kind='planning_consultation' AND json_extract(payload,'$.reference')=?2",
            rusqlite::params![conversation, reference], |row| Ok((row.get(0)?,row.get(1)?)),
        ).optional()?.ok_or_else(|| Error::Invalid("unknown planning reference in this conversation".into()))?;
        let request: Value = serde_json::from_str(&encoded)?;
        let failed: Option<String> = self.store.db().query_row(
            "SELECT json_extract(payload,'$.error') FROM events WHERE conversation_id=?1 AND kind='planning_consultation_failed' AND json_extract(payload,'$.reference')=?2 ORDER BY seq DESC LIMIT 1",
            rusqlite::params![conversation, reference], |row| row.get(0),
        ).optional()?;
        if let Some(error) = failed {
            return Ok(json!({"reference":reference,"state":"failed","error":error}));
        }
        let specialist = required_text(&request, "role")?;
        let Some(result) = self.store.last_turn(conversation, &specialist)? else {
            return Ok(
                json!({"reference":reference,"state":if created < self.opened_ms { "interrupted" } else { "running" }}),
            );
        };
        if result.status != TurnStatus::Done {
            return Ok(json!({"reference":reference,"state":result.status,"error":result.error}));
        }
        let answer = self
            .store
            .turn_final_answer(conversation, &result.id)?
            .ok_or_else(|| Error::Invalid("planning report was not retained".into()))?;
        let report = TurnResult::decode(&answer).text();
        let kind: WorkKind = serde_json::from_value(request["kind"].clone())?;
        let basis = self.planning_basis(conversation)?;
        let current = self.planning_request(conversation, &basis, kind)?;
        let historical = current.is_none_or(|current| current["reference"] != reference);
        let binding = crate::hash(serde_json::to_vec(&(conversation, role, turn, &reference))?);
        let (report, cursor) = page_text(&report, &binding, args)?;
        Ok(
            json!({"reference":reference,"kind":request["kind"],"state":"done","historical":historical,"report":report,"complete":cursor.is_none(),"cursor":cursor}),
        )
    }

    pub(super) fn require_planning_inputs(&self, conversation: &str, turn: &str) -> Result<()> {
        let basis = self.planning_basis(conversation)?;
        for kind in planning_kinds() {
            let request = self
                .planning_request(conversation, &basis, kind)?
                .ok_or_else(|| {
                    Error::Invalid(format!(
                        "before submit_plan call consult_planning for {} and use its findings",
                        tasks::work_kind_name(kind)
                    ))
                })?;
            let result = self.read_planning_result(
                conversation,
                PLANNER,
                turn,
                &json!({"reference":request["reference"]}),
            )?;
            require(
                result["state"] == "done",
                &format!(
                    "{} preparation is {}; read_planning_result or refresh the failed consultation before submit_plan",
                    tasks::work_kind_name(kind),
                    result["state"]
                ),
            )?;
        }
        Ok(())
    }
}

#[cfg(test)]
pub(super) fn prepared(core: &Core, conversation: &str) -> Vec<Value> {
    let basis = core.planning_basis(conversation).unwrap();
    let mut requests: Vec<Value> = Vec::new();
    for kind in planning_kinds() {
        let reference = format!("planning:{}", crate::id());
        let role = format!("planning-{}:{}", tasks::work_kind_name(kind), reference);
        let inputs: Vec<_> = requests
            .iter()
            .filter(|r| {
                prerequisites(kind)
                    .iter()
                    .any(|kind| r["kind"] == tasks::work_kind_name(*kind))
            })
            .map(|r| json!({"reference":r["reference"]}))
            .collect();
        let request = json!({"reference":reference,"role":role,"kind":tasks::work_kind_name(kind),"basis":basis,"inputs":inputs});
        core.store
            .append_event(conversation, None, None, "planning_consultation", &request)
            .unwrap();
        let turn = core
            .store
            .start_turn(conversation, &role, "fixture", 0, None)
            .unwrap();
        core.store
            .append_event(
                conversation,
                Some(&turn.id),
                None,
                "agent_final",
                &json!({"text":"Established facts and concrete verification scenarios"}),
            )
            .unwrap();
        core.store
            .finish_turn(&turn.id, TurnStatus::Done, None)
            .unwrap();
        requests.push(request);
    }
    requests
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planning_requires_completed_inputs_and_reuses_them_across_corrections() {
        let (_dir, core, conversation) = super::super::tests::core();
        core.store.db().execute("UPDATE session_workflow SET phase='planning',summary='Preserve drafts' WHERE conversation_id=?1", [&conversation]).unwrap();
        let turn = core
            .store
            .start_turn(&conversation, PLANNER, "fixture", 0, None)
            .unwrap();
        assert!(
            core.require_planning_inputs(&conversation, &turn.id)
                .unwrap_err()
                .to_string()
                .contains("research")
        );
        let requests = prepared(&core, &conversation);
        core.require_planning_inputs(&conversation, &turn.id)
            .unwrap();
        for request in &requests {
            let result = core
                .agent_tool(
                    &conversation,
                    PLANNER,
                    &turn.id,
                    "consult_planning",
                    &json!({"kind":request["kind"],"question":"Use established findings"}),
                )
                .unwrap();
            assert_eq!(result["reference"], request["reference"]);
            assert_eq!(result["state"], "done");
            assert!(
                result["report"]
                    .as_str()
                    .unwrap()
                    .contains("Established facts")
            );
        }
        core.store
            .add_message(
                &conversation,
                REPLAN,
                "Rearrange tasks without redoing preparation",
            )
            .unwrap();
        core.store
            .add_message(&conversation, PLAN_REVIEWER, "Clarify acceptance criteria")
            .unwrap();
        core.require_planning_inputs(&conversation, &turn.id)
            .unwrap();
        assert_eq!(
            core.store
                .events_since(&conversation, 0, 100)
                .unwrap()
                .iter()
                .filter(|e| e.kind == "planning_consultation")
                .count(),
            5
        );
        let plan = core.store.add_plan(&conversation, &[json!({"id":"inspect","title":"Inspect","goal":"Report findings","kind":"analysis","change_kind":"none","acceptance_criteria":[{"id":"C1","text":"Findings are supported"}]})]).unwrap();
        let scope = core
            .store
            .create_scope(&conversation, "Preserve drafts")
            .unwrap();
        core.store
            .bind_plan_scope(&conversation, &plan[0].plan_id, &scope.id)
            .unwrap();
        core.require_planning_inputs(&conversation, &turn.id)
            .unwrap();
        core.store
            .register_source(&conversation, "Additional source evidence")
            .unwrap();
        assert!(
            core.require_planning_inputs(&conversation, &turn.id)
                .is_err()
        );
        prepared(&core, &conversation);
        core.require_planning_inputs(&conversation, &turn.id)
            .unwrap();
        let changed = core
            .store
            .create_scope(&conversation, "Expanded accepted scope")
            .unwrap();
        let next = core
            .store
            .add_plan(&conversation, &[plan[0].spec.clone()])
            .unwrap();
        core.store
            .bind_plan_scope(&conversation, &next[0].plan_id, &changed.id)
            .unwrap();
        assert!(
            core.require_planning_inputs(&conversation, &turn.id)
                .is_err()
        );
        core.store
            .db()
            .execute(
                "UPDATE session_workflow SET summary='New requirements' WHERE conversation_id=?1",
                [&conversation],
            )
            .unwrap();
        assert!(
            core.require_planning_inputs(&conversation, &turn.id)
                .is_err()
        );
    }

    #[test]
    fn refreshing_a_producer_invalidates_only_its_consumers_and_history_stays_readable() {
        let (_dir, core, conversation) = super::super::tests::core();
        let turn = core
            .store
            .start_turn(&conversation, PLANNER, "fixture", 0, None)
            .unwrap();
        let requests = prepared(&core, &conversation);
        let mut new_research = requests[0].clone();
        new_research["reference"] = json!(format!("planning:{}", crate::id()));
        core.store
            .append_event(
                &conversation,
                Some(&turn.id),
                None,
                "planning_consultation",
                &new_research,
            )
            .unwrap();
        let basis = core.planning_basis(&conversation).unwrap();
        assert!(
            core.planning_request(&conversation, &basis, WorkKind::Analysis)
                .unwrap()
                .is_some()
        );
        for kind in [
            WorkKind::Dependencies,
            WorkKind::Architecture,
            WorkKind::TestDesign,
        ] {
            assert!(
                core.planning_request(&conversation, &basis, kind)
                    .unwrap()
                    .is_none()
            );
        }
        let old = core
            .agent_tool(
                &conversation,
                PLANNER,
                &turn.id,
                "read_planning_result",
                &json!({"reference":requests[2]["reference"]}),
            )
            .unwrap();
        assert_eq!(old["state"], "done");
        assert_eq!(old["historical"], true);
        let other = core.store.create_conversation("other").unwrap();
        let foreign = core
            .store
            .start_turn(&other.id, PLANNER, "fixture", 0, None)
            .unwrap();
        assert!(
            core.agent_tool(
                &other.id,
                PLANNER,
                &foreign.id,
                "read_planning_result",
                &json!({"reference":requests[0]["reference"]})
            )
            .is_err()
        );
    }

    #[test]
    fn consultation_order_failures_and_paging_are_explicit() {
        let (_dir, core, conversation) = super::super::tests::core();
        core.store
            .db()
            .execute(
                "UPDATE session_workflow SET phase='planning' WHERE conversation_id=?1",
                [&conversation],
            )
            .unwrap();
        let turn = core
            .store
            .start_turn(&conversation, PLANNER, "fixture", 0, None)
            .unwrap();
        let error = core
            .agent_tool(
                &conversation,
                PLANNER,
                &turn.id,
                "consult_planning",
                &json!({"kind":"architecture","question":"Define interfaces"}),
            )
            .unwrap_err();
        assert!(
            error.to_string().contains("consult analysis first"),
            "{error}"
        );
        let requests = prepared(&core, &conversation);
        let research_role = requests[0]["role"].as_str().unwrap();
        let report_turn = core
            .store
            .last_turn(&conversation, research_role)
            .unwrap()
            .unwrap();
        let report = "界e\u{301}".repeat(9000);
        core.store
            .append_event(
                &conversation,
                Some(&report_turn.id),
                None,
                "agent_final",
                &json!({"text":report}),
            )
            .unwrap();
        let mut args = json!({"reference":requests[0]["reference"]});
        let mut read = String::new();
        loop {
            let page = core
                .agent_tool(
                    &conversation,
                    PLANNER,
                    &turn.id,
                    "read_planning_result",
                    &args,
                )
                .unwrap();
            let text = page["report"].as_str().unwrap();
            assert!(text.len() <= 16384);
            read.push_str(text);
            if page["complete"] == true {
                break;
            }
            args["cursor"] = page["cursor"].clone();
        }
        assert_eq!(read, report);
        core.store
            .append_event(
                &conversation,
                Some(&turn.id),
                None,
                "planning_consultation_failed",
                &json!({"reference":requests[0]["reference"],"error":"spawn: executable missing"}),
            )
            .unwrap();
        let failed = core
            .agent_tool(
                &conversation,
                PLANNER,
                &turn.id,
                "read_planning_result",
                &json!({"reference":requests[0]["reference"]}),
            )
            .unwrap();
        assert_eq!(failed["state"], "failed");
        assert_eq!(failed["error"], "spawn: executable missing");
        assert!(
            core.require_planning_inputs(&conversation, &turn.id)
                .is_err()
        );
    }
}
