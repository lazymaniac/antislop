use super::core_plan_answer::PlanAnswer;
use super::core_planning_context::planning_ledger_context;
use super::*;
use prompt::{PlannerPrompt, planner_prompt_text};

#[path = "core_planner_turn/prompt.rs"]
mod prompt;

impl Core {
    /// Drafts the first plan from the agreed requirements in the planner's own
    /// session. Its words reach the user as the orchestrator's, so the
    /// conversation keeps one voice; the plan then follows the same path as
    /// any other draft.
    pub(in crate::lean) fn planner_turn(
        self: &Arc<Self>,
        conversation: &str,
        upto: u64,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let config = self.config_snapshot();
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        let answered = self.store.last_orchestration_answered(conversation)?;
        let asked = self.asked_since(conversation, answered, upto)?;
        let requirements = self.store.workflow(conversation)?.summary;
        self.store.append_event(
            conversation,
            None,
            None,
            "planning_progress",
            &json!({"text": "The planner is drafting the plan"}),
        )?;
        let ledger_context = planning_ledger_context(self, conversation)?;
        let replaced = if asked.iter().any(|message| message.author == REPLAN) {
            let graph = self.store.latest_plan(conversation)?;
            Some(serde_json::to_string(
                &graph.iter().map(|task| &task.spec).collect::<Vec<_>>(),
            )?)
        } else {
            None
        };
        let setup = TurnSetup {
            session_policy: SessionPolicy::Role,
            conversation,
            plan_id: None,
            role: PLANNER.into(),
            profile: config.resolve(WorkKind::Planning, RoleKind::Planner),
            cwd: project.clone(),
            add_dirs: vec![],
            access: agent::Access::ReadOnly,
            upto_message: upto,
        };
        let Some(answer) = self.turn(setup, cancel, |first| {
            planner_prompt_text(PlannerPrompt {
                project: &project,
                first,
                requirements: &requirements,
                asked: &asked,
                replaced: replaced.as_deref(),
                ledger_context: ledger_context.as_deref(),
            })
        })?
        else {
            return Ok(());
        };
        let (text, plan) = super::turn_result::TurnResult::plan_answer(&answer);
        let plan = plan.unwrap_or_else(|| {
            Err(Error::Invalid(
                "the planner ended its turn without submit_plan".into(),
            ))
        });
        self.store.append_event(
            conversation,
            None,
            None,
            "planning_progress",
            &json!({"text": if plan.is_ok() { "Plan draft ready" } else { "Plan draft requires correction" }}),
        )?;
        self.store.add_message(conversation, ORCHESTRATOR, &text)?;
        self.handle_plan_answer(
            PlanAnswer {
                conversation,
                project: &project,
                config: &config,
                text: &text,
                plan: Some(plan),
                addressed: None,
                is_comment: false,
                answered,
                upto,
            },
            cancel,
        )
    }
}

impl Core {
    /// Records that the planner must redraft the plan; the next turn is its.
    /// One request stays open until a turn answers it.
    pub(in crate::lean) fn request_replan(
        &self,
        conversation: &str,
        reason: &str,
    ) -> Result<serde_json::Value> {
        require(
            !self.store.latest_plan(conversation)?.is_empty(),
            "there is no plan to replace; the planner drafts the first one itself",
        )?;
        let answered = self.store.last_orchestration_answered(conversation)?;
        if self
            .store
            .messages_by(conversation, REPLAN, answered)?
            .is_empty()
        {
            self.store.add_message(conversation, REPLAN, reason)?;
        }
        Ok(json!({"state": "requested"}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn task() -> serde_json::Value {
        json!({"id":"t1","title":"T","goal":"g","kind":"analysis","verifier":{"focus":"f"}})
    }

    #[test]
    fn a_replan_request_hands_the_next_turn_to_the_planner() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
        let core = Core::open(&dir.path().join("state"), config).unwrap();
        let id = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        let request = core.store.add_message(&id, "user", "Inspect").unwrap();
        core.store
            .interview_summary(&id, "Inspect", request.seq)
            .unwrap();
        assert!(core.store.finish_interview(&id).unwrap());
        let reason = json!({"reason": "the plan splits the work along the wrong seam"});
        let turn = core
            .store
            .start_turn(&id, ORCHESTRATOR, "codex", request.seq, None)
            .unwrap();

        // Nothing to replace yet: the planner drafts the first plan itself.
        let error = core
            .agent_tool(&id, ORCHESTRATOR, &turn.id, "request_replan", &reason)
            .unwrap_err();
        assert!(error.to_string().contains("no plan to replace"), "{error}");
        std::thread::sleep(std::time::Duration::from_millis(3));
        core.store.add_plan(&id, &[task()]).unwrap();
        let tweak = core.store.add_message(&id, "user", "tweak it").unwrap();
        assert!(!core.planner_owns_turn(&id, tweak.seq).unwrap());

        // Only the orchestrator asks, and one request stays open.
        let auditor = core
            .store
            .start_turn(&id, AUDITOR, "codex", 0, None)
            .unwrap();
        assert!(
            core.agent_tool(&id, AUDITOR, &auditor.id, "request_replan", &reason)
                .is_err()
        );
        for _ in 0..2 {
            let reply = core
                .agent_tool(&id, ORCHESTRATOR, &turn.id, "request_replan", &reason)
                .unwrap();
            assert_eq!(reply["state"], "requested");
        }
        assert_eq!(core.store.messages_by(&id, REPLAN, 0).unwrap().len(), 1);

        // The orchestrator's turn ends; the request is the next thing asked.
        core.store
            .finish_turn(&turn.id, crate::lean::store::TurnStatus::Done, None)
            .unwrap();
        let upto = core
            .unanswered(&id)
            .unwrap()
            .expect("the request is pending");
        assert!(core.planner_owns_turn(&id, upto).unwrap());
    }
}
