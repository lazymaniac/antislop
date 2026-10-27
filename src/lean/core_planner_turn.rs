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
                ledger_context: ledger_context.as_deref(),
            })
        })?
        else {
            return Ok(());
        };
        let (text, plan) = tasks::parse_required_plan(&answer);
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
