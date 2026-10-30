use super::*;
use crate::lean::core_plan_answer::PlanAnswer;

pub(super) fn finish(
    core: &Arc<Core>,
    context: &OrchestrationContext<'_>,
    answer: &str,
    cancel: &AtomicBool,
) -> Result<()> {
    let result = super::super::turn_result::TurnResult::decode(answer);
    let addressed = result.addressed().map(<[u64]>::to_vec);
    let text = result.text();
    let (plan, awaiting_decision) = submitted_plan(core, context, result)?;
    record_answer_progress(core, context, plan.as_ref(), awaiting_decision)?;
    core.store
        .add_message(context.conversation, ORCHESTRATOR, &text)?;
    finish_interview(core, context, cancel)?;
    handle_plan_answer(core, context, text, plan, addressed, cancel)
}

fn submitted_plan(
    core: &Core,
    context: &OrchestrationContext<'_>,
    result: super::super::turn_result::TurnResult,
) -> Result<(Option<Result<Vec<TaskContract>>>, bool)> {
    let conversation = context.conversation;
    match result {
        super::super::turn_result::TurnResult::Plan { tasks, keep, .. } => Ok((
            Some(core.submitted_graph(conversation, tasks, &keep)),
            false,
        )),
        super::super::turn_result::TurnResult::Question { question } if context.interviewing => {
            core.store.ask_interview_question(conversation, &question)?;
            Ok((None, false))
        }
        super::super::turn_result::TurnResult::Question { question } => {
            core.store.ask_user_decision(conversation, &question)?;
            Ok((None, true))
        }
        super::super::turn_result::TurnResult::Requirements { markdown }
            if context.interviewing =>
        {
            core.store
                .interview_summary(conversation, &markdown, context.upto)?;
            Ok((None, false))
        }
        _ => Ok((None, false)),
    }
}

fn record_answer_progress(
    core: &Core,
    context: &OrchestrationContext<'_>,
    plan: Option<&Result<Vec<TaskContract>>>,
    awaiting_decision: bool,
) -> Result<()> {
    if context.interviewing && plan.is_some() {
        core.notice(context.conversation, "plan withheld during interview; planning starts after the requirements summary is complete");
    }
    if context.workflow.planning() && !awaiting_decision {
        let text = if plan.is_some_and(Result::is_ok) {
            "Plan draft ready"
        } else {
            "Plan draft requires correction"
        };
        core.store.append_event(
            context.conversation,
            None,
            None,
            "planning_progress",
            &json!({"text":text}),
        )?;
    }
    Ok(())
}

fn finish_interview(
    core: &Core,
    context: &OrchestrationContext<'_>,
    cancel: &AtomicBool,
) -> Result<()> {
    if context.interviewing
        && !cancel.load(Ordering::SeqCst)
        && core.autonomous(context.conversation)?
    {
        core.store.finish_interview(context.conversation)?;
    }
    Ok(())
}

fn handle_plan_answer(
    core: &Arc<Core>,
    context: &OrchestrationContext<'_>,
    text: String,
    plan: Option<Result<Vec<TaskContract>>>,
    addressed: Option<Vec<u64>>,
    cancel: &AtomicBool,
) -> Result<()> {
    core.handle_plan_answer(
        PlanAnswer {
            conversation: context.conversation,
            project: &context.project,
            config: &context.config,
            text: &text,
            plan,
            addressed: addressed.as_deref(),
            is_comment: context.is_comment,
            answered: context.answered,
            upto: context.upto,
        },
        cancel,
    )
}
