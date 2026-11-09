use super::*;
use crate::lean::core::plan::answer::PlanAnswer;

pub(super) fn finish(
    core: &Arc<Core>,
    context: &OrchestrationContext<'_>,
    answer: &str,
    cancel: &AtomicBool,
) -> Result<()> {
    let result = super::super::turn_result::TurnResult::decode(answer);
    let submitted = submitted_graph(core, context.conversation, &result);
    if let Some(audit) = &context.audit_response
        && let Some(reason) = audit_response_error(
            audit,
            &result.text(),
            submitted.as_ref().is_some_and(Result::is_ok),
        )
    {
        core.store.add_message(
            context.conversation,
            RUNTIME,
            &format!(
                "The auditor report is still awaiting your response: {reason} Re-read the report and provide the required finding-by-finding Audit response before proceeding."
            ),
        )?;
        return Ok(());
    }
    let addressed = result.addressed().map(<[u64]>::to_vec);
    let text = result.text();
    let (plan, awaiting_decision) = submitted_plan(core, context, result, submitted)?;
    record_answer_progress(core, context, plan.as_ref(), awaiting_decision)?;
    core.store
        .add_message(context.conversation, ORCHESTRATOR, &text)?;
    finish_interview(core, context, cancel)?;
    handle_plan_answer(core, context, text, plan, addressed, cancel)
}

fn audit_response_error(
    audit: &super::AuditResponse,
    response: &str,
    submits_plan: bool,
) -> Option<String> {
    let lines = response.lines().map(str::trim).collect::<Vec<_>>();
    if audit.error.is_some() {
        let acknowledged = lines.iter().any(|line| {
            line.to_ascii_lowercase()
                .starts_with("audit status: unaudited - ")
                && line.len() > "audit status: unaudited - ".len()
        });
        return (!acknowledged)
            .then(|| "the audit failure was not acknowledged as unaudited".to_owned());
    }
    if audit.findings.is_empty() {
        let status = if audit.approved { "complete" } else { "open" };
        let prefix = format!("audit status: {status} - ");
        if !lines
            .iter()
            .any(|line| line.to_ascii_lowercase().starts_with(&prefix) && line.len() > prefix.len())
        {
            return Some(format!("the report requires `{prefix}<assessment>`"));
        }
        return (status == "open" && !submits_plan).then(|| {
            "an open audit result needs a remediation plan submitted for independent review".into()
        });
    }
    let Some(section) = lines
        .iter()
        .position(|line| line.eq_ignore_ascii_case("audit response:"))
        .map(|section| &lines[section + 1..])
    else {
        return Some("the response is missing the `Audit response:` section".into());
    };
    let mut has_open_finding = false;
    for (index, _) in audit.findings.iter().enumerate() {
        let number = index + 1;
        let resolved = format!("finding {number}: resolved - ");
        let open = format!("finding {number}: open - ");
        if !section.iter().any(|line| {
            let line = line.to_ascii_lowercase();
            [resolved.as_str(), open.as_str()]
                .iter()
                .any(|prefix| line.starts_with(prefix) && line.len() > prefix.len())
        }) {
            return Some(format!(
                "finding {number} needs a `resolved` or `open` disposition with a reason"
            ));
        }
        has_open_finding |= section
            .iter()
            .any(|line| line.to_ascii_lowercase().starts_with(&open));
    }
    if has_open_finding && !submits_plan {
        return Some(
            "open audit findings need a remediation plan submitted for independent review".into(),
        );
    }
    None
}

fn submitted_plan(
    core: &Core,
    context: &OrchestrationContext<'_>,
    result: super::super::turn_result::TurnResult,
    submitted: Option<Result<Vec<TaskContract>>>,
) -> Result<(Option<Result<Vec<TaskContract>>>, bool)> {
    let conversation = context.conversation;
    match result {
        super::super::turn_result::TurnResult::Plan { .. } => Ok((submitted, false)),
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

fn submitted_graph(
    core: &Core,
    conversation: &str,
    result: &super::super::turn_result::TurnResult,
) -> Option<Result<Vec<TaskContract>>> {
    match result {
        super::super::turn_result::TurnResult::Plan { tasks, keep, .. } => {
            Some(core.submitted_graph(conversation, tasks.clone(), keep))
        }
        _ => None,
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
    if context.interviewing && !cancel.load(Ordering::SeqCst) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lean::{Core, RUNTIME};
    use serde_json::json;
    use std::sync::Arc;

    fn with_audited_context(
        test: impl FnOnce(&Arc<Core>, &str, &super::super::OrchestrationContext<'_>),
    ) {
        with_audited_context_and_comment(false, test);
    }

    fn with_audited_context_and_comment(
        include_comment: bool,
        test: impl FnOnce(&Arc<Core>, &str, &super::super::OrchestrationContext<'_>),
    ) {
        let directory = tempfile::tempdir().unwrap();
        let config = crate::config::Config::from_toml(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/antislop.toml"
        )))
        .unwrap();
        let core = Core::open(&directory.path().join("state"), config).unwrap();
        let conversation = core.store.create_conversation("fixture").unwrap().id;
        let task = core
            .store
            .add_plan(
                &conversation,
                &[json!({"id":"implementation","title":"Implementation","goal":"Deliver the request","change_kind":"code","kind":"implementation"})],
            )
            .unwrap()
            .remove(0);
        core.store
            .append_event(
                &conversation,
                None,
                Some(&task.plan_id),
                "audit",
                &json!({"approved":false,"findings":[{"id":"finding:abc12345","severity":"high","issue":"Missing evidence for the delivered behavior","where":"validation report"}]}),
            )
            .unwrap();
        let report = core
            .store
            .add_message(
                &conversation,
                RUNTIME,
                "## Independent audit\nThe request is not fully delivered: Missing evidence for the delivered behavior.",
            )
            .unwrap();
        let upto = if include_comment {
            core.store
                .add_message(
                    &conversation,
                    "user",
                    &format!("{COMMENT_PREFIX}1] Please explain the audit finding."),
                )
                .unwrap()
                .seq
        } else {
            report.seq
        };
        let context = super::super::prepare(&core, &conversation, upto).unwrap();
        test(&core, &conversation, &context);
        drop(directory);
    }

    #[test]
    fn ignored_audit_findings_are_rejected_before_orchestration_progress() {
        with_audited_context(|core, conversation, context| {
            finish(
                core,
                context,
                "The request is done. I will proceed.",
                &AtomicBool::new(false),
            )
            .unwrap();

            assert!(
                core.store
                    .messages_by(conversation, ORCHESTRATOR, 0)
                    .unwrap()
                    .is_empty()
            );
            let runtime = core.store.messages_by(conversation, RUNTIME, 0).unwrap();
            assert_eq!(runtime.len(), 2);
            assert!(runtime[1].text.contains("still awaiting your response"));
        });
    }

    #[test]
    fn rejected_audit_response_remains_pending_after_the_turn_cursor_advances() {
        with_audited_context(|core, conversation, context| {
            let turn = core
                .store
                .start_turn(conversation, ORCHESTRATOR, "fixture", context.upto, None)
                .unwrap();
            core.store
                .finish_turn(&turn.id, crate::lean::store::TurnStatus::Done, None)
                .unwrap();

            finish(
                core,
                context,
                "The request is done. I will proceed.",
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(
                core.store
                    .last_orchestration_answered(conversation)
                    .unwrap(),
                context.upto
            );

            let reminder = core.store.messages_by(conversation, RUNTIME, 0).unwrap()[1].clone();
            let retry = super::super::prepare(core, conversation, reminder.seq).unwrap();
            assert_eq!(retry.asked.len(), 1);
            assert!(retry.asked[0].text.contains("still awaiting your response"));
            assert!(retry.audit_response.is_some());
            let prompt = super::super::render_prompt(&retry, false, false);
            assert!(prompt.contains("AUTHORITATIVE AUDIT RESPONSE INSTRUCTIONS:"));
            assert!(prompt.contains(r"## Independent audit\nThe request is not fully delivered"));
            assert!(prompt.contains("Missing evidence for the delivered behavior"));

            finish(
                core,
                &retry,
                "The request is done. I will proceed again.",
                &AtomicBool::new(false),
            )
            .unwrap();
            assert!(
                core.store
                    .messages_by(conversation, ORCHESTRATOR, 0)
                    .unwrap()
                    .is_empty()
            );
            let runtime = core.store.messages_by(conversation, RUNTIME, 0).unwrap();
            assert_eq!(runtime.len(), 3);
            assert!(runtime[2].text.contains("still awaiting your response"));

            finish(
                core,
                &retry,
                "Audit response:\nfinding 1: resolved - I verified the retained validation evidence.",
                &AtomicBool::new(false),
            )
            .unwrap();
            let answers = core
                .store
                .messages_by(conversation, ORCHESTRATOR, 0)
                .unwrap();
            assert_eq!(answers.len(), 1);
            assert!(answers[0].text.contains("finding 1: resolved"));
            assert!(
                super::super::prepare(core, conversation, answers[0].seq)
                    .unwrap()
                    .audit_response
                    .is_none()
            );
        });
    }

    #[test]
    fn orchestrator_response_records_a_disposition_for_each_finding() {
        with_audited_context(|core, conversation, context| {
            let setup = super::super::turn_setup(
                conversation,
                context.upto,
                &context.project,
                &context.config,
                context.is_comment,
                context.audit_response.is_some(),
            );
            assert_eq!(setup.role, ORCHESTRATOR);
            let prompt = super::super::render_prompt(context, false, false);
            assert!(prompt.contains("AUTHORITATIVE AUDIT RESPONSE INSTRUCTIONS:"));
            assert!(prompt.contains("Missing evidence for the delivered behavior"));
            assert!(prompt.contains("finding N: resolved -"));
            assert!(prompt.contains("finding N: open -"));

            finish(
                core,
                context,
                "Audit response:\nfinding 1: resolved - I verified the retained validation evidence and it directly covers the delivered behavior.",
                &AtomicBool::new(false),
            )
            .unwrap();
            let answers = core
                .store
                .messages_by(conversation, ORCHESTRATOR, 0)
                .unwrap();
            assert_eq!(answers.len(), 1);
            assert!(answers[0].text.contains("finding 1: resolved"));
        });
    }

    #[test]
    fn mixed_audit_report_and_user_comment_keep_orchestrator_handoff() {
        with_audited_context_and_comment(true, |core, conversation, context| {
            assert!(context.is_comment);
            assert!(context.audit_response.is_some());

            let setup = super::super::turn_setup(
                conversation,
                context.upto,
                &context.project,
                &context.config,
                context.is_comment,
                context.audit_response.is_some(),
            );
            assert_eq!(setup.role, ORCHESTRATOR);

            let prompt = super::super::render_prompt(context, false, false);
            assert!(prompt.contains("AUTHORITATIVE AUDIT RESPONSE INSTRUCTIONS:"));
            assert!(prompt.contains("Missing evidence for the delivered behavior"));
            assert!(prompt.contains("Address each new user comment explicitly"));

            finish(
                core,
                context,
                "Audit response:\nfinding 1: resolved - I verified the retained evidence.",
                &AtomicBool::new(false),
            )
            .unwrap();
            let answers = core
                .store
                .messages_by(conversation, ORCHESTRATOR, 0)
                .unwrap();
            assert_eq!(answers.len(), 1);
            assert!(answers[0].text.contains("finding 1: resolved"));
        });
    }

    #[test]
    fn open_findings_cannot_advance_without_a_reviewed_plan_submission() {
        with_audited_context(|core, conversation, context| {
            finish(
                core,
                context,
                "Audit response:\nfinding 1: open - The validation evidence is missing; I will fix it later.",
                &AtomicBool::new(false),
            )
            .unwrap();

            assert!(
                core.store
                    .messages_by(conversation, ORCHESTRATOR, 0)
                    .unwrap()
                    .is_empty()
            );
            assert!(
                core.store.messages_by(conversation, RUNTIME, 0).unwrap()[1]
                    .text
                    .contains("remediation plan submitted for independent review")
            );
        });
    }

    #[test]
    fn rejected_remediation_plan_does_not_handle_open_findings() {
        with_audited_context(|core, conversation, context| {
            let answer = serde_json::json!({
                "type": "plan",
                "document": "Audit response:\nfinding 1: open - The missing evidence needs to be added.",
                "tasks": [],
                "keep": ["missing-task"]
            })
            .to_string();
            let answered_before = core
                .store
                .last_orchestration_answered(conversation)
                .unwrap();

            finish(core, context, &answer, &AtomicBool::new(false)).unwrap();

            assert!(
                core.store
                    .messages_by(conversation, ORCHESTRATOR, 0)
                    .unwrap()
                    .is_empty()
            );
            assert_eq!(
                core.store
                    .last_orchestration_answered(conversation)
                    .unwrap(),
                answered_before
            );
            let events = core.store.events_since(conversation, 0, 128).unwrap();
            assert!(!events.iter().any(|event| event.kind == "plan_invalid"));
            let runtime = core.store.messages_by(conversation, RUNTIME, 0).unwrap();
            assert_eq!(runtime.len(), 2);
            assert!(
                runtime[1]
                    .text
                    .contains("remediation plan submitted for independent review")
            );
        });
    }
}
