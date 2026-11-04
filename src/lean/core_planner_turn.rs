use super::core_plan_answer::PlanAnswer;
use super::core_planning_context::planning_ledger_context;
use super::*;
use prompt::{PlannerPrompt, planner_prompt_text};

#[path = "core_planner_turn/prompt.rs"]
mod prompt;

struct PlannerTurnContext<'a> {
    conversation: &'a str,
    upto: u64,
    config: Arc<Config>,
    project: PathBuf,
    answered: u64,
    asked: Vec<store::Message>,
    requirements: String,
    ledger_context: Option<String>,
    keepable: Option<String>,
    replaced: Option<String>,
}

impl Core {
    pub(in crate::lean) fn planner_turn(
        self: &Arc<Self>,
        conversation: &str,
        upto: u64,
        cancel: &AtomicBool,
    ) -> Result<()> {
        if !self.planning_requirements_ready(conversation, upto, cancel)? {
            return Ok(());
        }
        let context = self.planner_turn_context(conversation, upto)?;
        let Some(answer) = self.run_planner_turn(&context, cancel)? else {
            return Ok(());
        };
        self.handle_planner_answer(&context, &answer, cancel)
    }

    fn planning_requirements_ready(
        self: &Arc<Self>,
        conversation: &str,
        upto: u64,
        cancel: &AtomicBool,
    ) -> Result<bool> {
        if self
            .planning_requirements(conversation, None, cancel)?
            .is_some()
        {
            return Ok(true);
        }
        if self
            .unanswered(conversation)?
            .is_some_and(|next| next > upto)
        {
            return Ok(false);
        }
        Err(Error::Invalid(
            "requirements are not ready for planning".into(),
        ))
    }

    fn planner_turn_context<'a>(
        &self,
        conversation: &'a str,
        upto: u64,
    ) -> Result<PlannerTurnContext<'a>> {
        let config = self.config_snapshot();
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        let answered = self.store.last_orchestration_answered(conversation)?;
        let asked = self.asked_since(conversation, answered, upto)?;
        let requirements = self.store.workflow(conversation)?.summary;
        announce_planner_draft(&self.store, conversation)?;
        let ledger_context = planning_ledger_context(self, conversation)?;
        let keepable = self.keepable_tasks_note(conversation)?;
        let replaced = self.replaced_graph(conversation, &asked)?;
        Ok(PlannerTurnContext {
            conversation,
            upto,
            config,
            project,
            answered,
            asked,
            requirements,
            ledger_context,
            keepable,
            replaced,
        })
    }

    fn replaced_graph(
        &self,
        conversation: &str,
        asked: &[store::Message],
    ) -> Result<Option<String>> {
        if !asked
            .iter()
            .any(|message| matches!(message.author.as_str(), REPLAN | PLAN_REVIEWER))
        {
            return Ok(None);
        }
        let graph = self.store.latest_plan(conversation)?;
        Ok(Some(serde_json::to_string(
            &graph.iter().map(|task| &task.spec).collect::<Vec<_>>(),
        )?))
    }

    fn run_planner_turn(
        self: &Arc<Self>,
        context: &PlannerTurnContext<'_>,
        cancel: &AtomicBool,
    ) -> Result<Option<String>> {
        let setup = planner_setup(context);
        self.turn(setup, cancel, |first, _| planner_prompt(context, first))
    }

    fn handle_planner_answer(
        self: &Arc<Self>,
        context: &PlannerTurnContext<'_>,
        answer: &str,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let (text, plan) = super::turn_result::TurnResult::plan_answer(answer);
        if plan.is_none() {
            record_planner_result(&self.store, context.conversation, false)?;
            self.store
                .add_message(context.conversation, PLANNER_BLOCKER, &text)?;
            return Ok(());
        }
        let plan = self.planner_submitted_plan(context.conversation, plan);
        record_planner_result(&self.store, context.conversation, plan.is_ok())?;
        self.store
            .add_message(context.conversation, ORCHESTRATOR, &text)?;
        self.handle_plan_answer(
            PlanAnswer {
                conversation: context.conversation,
                project: &context.project,
                config: &context.config,
                text: &text,
                plan: Some(plan),
                addressed: None,
                is_comment: false,
                answered: context.answered,
                upto: context.upto,
            },
            cancel,
        )
    }

    fn planner_submitted_plan(
        &self,
        conversation: &str,
        plan: Option<super::turn_result::SubmittedPlan>,
    ) -> Result<Vec<TaskContract>> {
        match plan {
            Some((tasks, keep)) => self.submitted_graph(conversation, tasks, &keep),
            None => Err(Error::Invalid(
                "the planner could not draft a plan; its message names what blocks it".into(),
            )),
        }
    }
}

fn planner_setup<'a>(context: &PlannerTurnContext<'a>) -> TurnSetup<'a> {
    TurnSetup {
        session_policy: SessionPolicy::Role,
        conversation: context.conversation,
        plan_id: None,
        role: PLANNER.into(),
        profile: context
            .config
            .resolve(WorkKind::Planning, RoleKind::Planner),
        cwd: context.project.clone(),
        add_dirs: vec![],
        access: agent::Access::ReadOnly,
        upto_message: context.upto,
    }
}

fn planner_prompt(context: &PlannerTurnContext<'_>, first: bool) -> String {
    planner_prompt_text(PlannerPrompt {
        project: &context.project,
        first,
        requirements: &context.requirements,
        asked: &context.asked,
        replaced: context.replaced.as_deref(),
        keepable: context.keepable.as_deref(),
        ledger_context: context.ledger_context.as_deref(),
    })
}

fn announce_planner_draft(store: &Store, conversation: &str) -> Result<()> {
    store.append_event(
        conversation,
        None,
        None,
        "planning_progress",
        &json!({"text":"The planner is drafting the plan"}),
    )?;
    Ok(())
}

fn record_planner_result(store: &Store, conversation: &str, valid: bool) -> Result<()> {
    let text = if valid {
        "Plan draft ready"
    } else {
        "Plan draft requires correction"
    };
    store.append_event(
        conversation,
        None,
        None,
        "planning_progress",
        &json!({"text":text}),
    )?;
    Ok(())
}

impl Core {
    /// A planner ends its turn without a plan only to report what blocks it,
    /// once submit_plan or the plan reviewer refused its plan; otherwise a planner that
    /// skipped the plan would stop the session until the user writes again.
    pub(in crate::lean) fn require_refused_plan(
        &self,
        conversation: &str,
        turn: &str,
    ) -> Result<()> {
        let refused: i64 = self.store.db().query_row(
            "SELECT count(*) FROM events WHERE conversation_id=?1 AND turn_id=?2 AND kind='agent_tool_rejected' AND json_extract(payload,'$.tool')='submit_plan'",
            rusqlite::params![conversation, turn],
            |row| row.get(0),
        )?;
        let upto: i64 = self.store.db().query_row(
            "SELECT upto_message FROM turns WHERE id=?1 AND conversation_id=?2",
            rusqlite::params![turn, conversation],
            |row| row.get(0),
        )?;
        let answered = self.store.last_orchestration_answered(conversation)?;
        let reviewed = self
            .asked_since(conversation, answered, upto as u64)?
            .iter()
            .any(|message| message.author == PLAN_REVIEWER);
        // A requested replan may itself demand an unavailable capability.
        // Hand that blocker back instead of requiring a fabricated submission.
        let replanning = self
            .store
            .messages_by(conversation, REPLAN, answered)?
            .iter()
            .any(|message| message.seq <= upto as u64);
        require(
            refused > 0 || reviewed || replanning,
            "submit the plan with submit_plan; finish_turn reports a concrete blocker after a refusal or during a requested replan; only the orchestrator can call revise_plan",
        )
    }

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
        if !self
            .store
            .messages_by(conversation, REPLAN, answered)?
            .is_empty()
        {
            return Ok(json!({"state":"requested"}));
        }
        let user_message: i64 = self.store.db().query_row(
            "SELECT coalesce(max(seq),0) FROM messages WHERE conversation_id=?1 AND author='user'",
            [conversation],
            |r| r.get(0),
        )?;
        let run = self
            .store
            .planning_run(conversation)?
            .map(|run| (run.id, run.spec_revision));
        let reason_key = reason
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
            .to_lowercase();
        let basis = crate::hash(serde_json::to_vec(&(user_message, run, reason_key))?);
        let condition =
            "new user input, changed requirements, or concrete new evidence resolving this blocker";
        let fresh = self.store.db().execute("INSERT OR IGNORE INTO replan_blockers(conversation_id,basis,resume_condition) VALUES(?1,?2,?3)",rusqlite::params![conversation,basis,condition])?;
        if fresh == 0 {
            return Ok(json!({"state":"blocked","resume_condition":condition}));
        }
        self.store.add_message(conversation, REPLAN, reason)?;
        Ok(json!({"state": "requested"}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn task() -> serde_json::Value {
        json!({"id":"t1","title":"T","goal":"g","kind":"analysis","reviewer":{"focus":"f"}})
    }

    #[test]
    fn a_planner_reports_a_blocker_only_after_its_plan_was_refused() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
        let core = Core::open(&dir.path().join("state"), config).unwrap();
        let id = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        let turn = core
            .store
            .start_turn(&id, PLANNER, "codex", 0, None)
            .unwrap();
        let blocker = json!({"text": "keep refuses the task the instructions say to keep"});
        let error = core
            .agent_tool(&id, PLANNER, &turn.id, "finish_turn", &blocker)
            .unwrap_err();
        assert!(error.to_string().contains("submit_plan"), "{error}");
        core.store
            .append_event(
                &id,
                Some(&turn.id),
                None,
                "agent_tool_rejected",
                &json!({"tool": "submit_plan", "error": "keep: x cannot be kept"}),
            )
            .unwrap();
        core.agent_tool(&id, PLANNER, &turn.id, "finish_turn", &blocker)
            .unwrap();
    }

    #[test]
    fn requested_replan_can_return_a_capability_blocker_without_faking_a_submission() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
        let core = Core::open(&dir.path().join("state"), config).unwrap();
        let conversation = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        let tasks = core.store.add_plan(&conversation, &[task()]).unwrap();
        let request = core
            .store
            .add_message(
                &conversation,
                REPLAN,
                "Incrementally revise the existing plan with revise_plan",
            )
            .unwrap();
        let turn = core
            .store
            .start_turn(&conversation, PLANNER, "fixture", request.seq, None)
            .unwrap();
        core.agent_tool(&conversation, PLANNER, &turn.id, "finish_turn", &json!({"text":"revise_plan belongs to the orchestrator; it must submit this incremental revision itself"})).unwrap();
        assert_eq!(
            core.store.latest_plan(&conversation).unwrap()[0].id,
            tasks[0].id
        );
        assert!(
            core.store
                .latest_role_result(&conversation, PLANNER)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn reviewer_findings_return_the_draft_to_the_planner_and_blockers_go_to_orchestration() {
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
        let plan = core.store.add_plan(&id, &[task()]).unwrap()[0]
            .plan_id
            .clone();
        core.store
            .set_plan_status(&plan, status::PROPOSED, status::REVISED)
            .unwrap();
        let draft = core
            .store
            .start_turn(&id, PLANNER, "codex", request.seq, None)
            .unwrap();
        core.store
            .finish_turn(&draft.id, TurnStatus::Done, None)
            .unwrap();
        let findings = core
            .store
            .add_message(&id, PLAN_REVIEWER, "The correction needs a user decision")
            .unwrap();
        let upto = core.unanswered(&id).unwrap().unwrap();
        assert_eq!(upto, findings.seq);
        assert!(core.planner_owns_turn(&id, upto).unwrap());
        let comment = core
            .store
            .add_message(&id, "user", &format!("{COMMENT_PREFIX}1] Add detail"))
            .unwrap();
        assert_eq!(
            core.unanswered(&id).unwrap(),
            Some(findings.seq),
            "a queued comment must not consume reviewer findings"
        );
        assert!(!core.planner_owns_turn(&id, comment.seq).unwrap());
        let context = core.planner_turn_context(&id, upto).unwrap();
        let prompt = planner_prompt(&context, true);
        assert!(prompt.contains("The correction needs a user decision"));
        assert!(prompt.contains("task contracts"));
        assert!(context.replaced.as_ref().unwrap().contains("t1"));
        let turn = core
            .store
            .start_turn(&id, PLANNER, "codex", upto, None)
            .unwrap();
        core.agent_tool(
            &id,
            PLANNER,
            &turn.id,
            "finish_turn",
            &json!({"text": "A user decision is needed"}),
        )
        .unwrap();
        core.store
            .finish_turn(&turn.id, TurnStatus::Done, None)
            .unwrap();
        let answer = "A user decision is needed";
        core.handle_planner_answer(&context, answer, &AtomicBool::new(false))
            .unwrap();
        let comment_turn = core
            .store
            .start_turn(&id, COMMENT_ORCHESTRATOR, "codex", comment.seq, None)
            .unwrap();
        core.store
            .finish_turn(&comment_turn.id, TurnStatus::Done, None)
            .unwrap();
        let next = core.unanswered(&id).unwrap().expect("blocker is queued");
        assert!(!core.planner_owns_turn(&id, next).unwrap());
        assert_eq!(
            core.store
                .messages_by(&id, PLANNER_BLOCKER, 0)
                .unwrap()
                .len(),
            1
        );
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

#[cfg(test)]
mod replan_blocker_tests {
    use super::*;
    #[test]
    fn identical_replans_leave_one_blocker_until_new_user_input() {
        let directory = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
        let core = Core::open(&directory.path().join("state"), config).unwrap();
        let conversation = core
            .store
            .create_conversation(directory.path().to_str().unwrap())
            .unwrap()
            .id;
        core.store
            .add_plan(
                &conversation,
                &[json!({"id":"analysis","title":"Analysis","goal":"Report","kind":"analysis"})],
            )
            .unwrap();
        assert_eq!(
            core.request_replan(&conversation, "Missing evidence")
                .unwrap()["state"],
            "requested"
        );
        let request = core
            .store
            .messages_by(&conversation, REPLAN, 0)
            .unwrap()
            .pop()
            .unwrap();
        let turn = core
            .store
            .start_turn(&conversation, PLANNER, "fixture", request.seq, None)
            .unwrap();
        core.store
            .finish_turn(&turn.id, TurnStatus::Done, None)
            .unwrap();
        let blocked = core
            .request_replan(&conversation, "Missing  evidence")
            .unwrap();
        assert_eq!(blocked["state"], "blocked");
        assert!(blocked["resume_condition"].is_string());
        assert_eq!(
            core.store
                .messages_by(&conversation, REPLAN, 0)
                .unwrap()
                .len(),
            1
        );
        core.store
            .add_message(&conversation, "user", "Here is the missing evidence")
            .unwrap();
        assert_eq!(
            core.request_replan(&conversation, "Missing evidence")
                .unwrap()["state"],
            "requested"
        );
    }
}
