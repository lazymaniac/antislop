use super::super::reviewer_setup;
use crate::lean::*;

pub(super) struct ReviewRound<'a> {
    pub core: &'a Arc<Core>,
    pub conversation: &'a str,
    pub task: &'a mut Task,
    pub cancel: &'a AtomicBool,
    pub spec: &'a TaskContract,
    pub workdir: &'a Path,
    pub add_dirs: &'a [PathBuf],
    pub upstream: &'a str,
    pub reviewer: &'a str,
    pub round: u32,
    pub findings: &'a [serde_json::Value],
    pub report: &'a str,
    pub changes: Option<&'a str>,
}

struct ReviewTurn<'a> {
    core: &'a Core,
    cancel: &'a AtomicBool,
    setup: TurnSetup<'a>,
    reviewdir: &'a Path,
    spec: &'a TaskContract,
    report: &'a str,
    changes: Option<&'a str>,
    round: u32,
    upstream: &'a str,
    findings: &'a [serde_json::Value],
    user_checks: String,
}

pub(super) struct VerdictSave<'a> {
    pub core: &'a Core,
    pub task: &'a mut Task,
    pub spec: &'a TaskContract,
    pub verdict: Verdict,
    pub report: &'a str,
    pub changes: Option<&'a str>,
    pub summary_effect: Option<(String, bool)>,
    pub repairs: u64,
}

pub(super) fn run(mut input: ReviewRound<'_>) -> Result<Option<Verdict>> {
    let answer = request_review(&mut input)?;
    let Some(answer) = answer else {
        input
            .core
            .stop_task(input.conversation, input.task, input.reviewer, input.cancel)?;
        return Ok(None);
    };
    parse_verdict(
        input.core,
        input.conversation,
        input.task,
        answer,
        input.report,
    )
}

fn request_review(input: &mut ReviewRound<'_>) -> Result<Option<String>> {
    input
        .core
        .set_status(input.conversation, input.task, status::REVIEWING)?;
    let setup = reviewer_setup::build(
        input.conversation,
        input.task,
        input.reviewer.to_owned(),
        input
            .core
            .config_snapshot()
            .resolve(input.spec.kind, RoleKind::Reviewer),
        input.workdir,
        input.add_dirs.to_vec(),
    );
    let checks = input
        .core
        .check_comments(input.conversation, &input.task.plan_id, &input.spec.id);
    run_review_turn(ReviewTurn {
        core: input.core,
        cancel: input.cancel,
        setup,
        reviewdir: input.workdir,
        spec: input.spec,
        report: input.report,
        changes: input.changes,
        round: input.round,
        upstream: input.upstream,
        findings: input.findings,
        user_checks: checks,
    })
}

fn run_review_turn(input: ReviewTurn<'_>) -> Result<Option<String>> {
    input.core.turn(input.setup, input.cancel, |first, _| {
        let prompt = tasks::reviewer_turn_prompt(
            &input.reviewdir.to_string_lossy(),
            input.spec,
            input.report,
            input.changes,
            first || input.round == 1,
            input.upstream,
            input.findings,
        );
        format!("{}{prompt}", input.user_checks)
    })
}

fn parse_verdict(
    core: &Core,
    conversation: &str,
    task: &mut Task,
    answer: String,
    report: &str,
) -> Result<Option<Verdict>> {
    let completion: turn_result::TurnResult = serde_json::from_str(&answer)?;
    match completion {
        turn_result::TurnResult::Review { verdict, .. } => Ok(Some(verdict)),
        turn_result::TurnResult::NeedsInput {
            request,
            checkpoint,
        } => {
            task.report = Some(format!("{report}\n{checkpoint}"));
            core.wait_for_question(conversation, task, &request, true)?;
            Ok(None)
        }
        _ => Err(Error::Invalid("reviewer did not submit a review".into())),
    }
}

pub(super) fn record_checks(
    core: &Core,
    conversation: &str,
    task: &Task,
    spec: &TaskContract,
    verdict: &Verdict,
) -> Result<()> {
    let obligations = tasks::obligations(spec);
    for (id, _) in &obligations {
        record_one(core, conversation, task, verdict, id);
    }
    Ok(())
}

fn record_one(core: &Core, conversation: &str, task: &Task, verdict: &Verdict, id: &str) {
    let Some(entry) = verdict.coverage.get(id) else {
        return;
    };
    let status = entry["status"]
        .as_str()
        .map(str::to_ascii_lowercase)
        .filter(|s| matches!(s.as_str(), "ok" | "issue" | "blocked" | "n/a"));
    let note = tasks::bound(
        entry["note"].as_str().unwrap_or_default(),
        tasks::DETAIL_LIMIT,
    );
    core.record_check(
        conversation,
        task,
        &tasks::CheckUpdate {
            id: id.to_owned(),
            status,
            note,
        },
    );
}

pub(super) fn save_verdict(mut input: VerdictSave<'_>) -> Result<()> {
    stamp_verdict(input.task, &mut input.verdict);
    input.task.verdict = Some(serde_json::to_value(&input.verdict)?);
    set_feedback(
        input.task,
        input.spec,
        &input.verdict,
        input.report,
        input.changes,
        input.summary_effect,
        input.repairs,
    );
    input.core.store.update_task(input.task)?;
    Ok(())
}

fn stamp_verdict(task: &Task, verdict: &mut Verdict) {
    let mut value = serde_json::to_value(&*verdict).expect("verdict serializes");
    if let Some(object) = value.as_object_mut() {
        object.insert("round".into(), json!(task.round));
        if let Some(result) = &task.result {
            object.insert("result".into(), json!(result));
        }
    }
    *verdict = serde_json::from_value(value).expect("stamped verdict deserializes");
}

fn set_feedback(
    task: &mut Task,
    spec: &TaskContract,
    verdict: &Verdict,
    report: &str,
    changes: Option<&str>,
    summary_effect: Option<(String, bool)>,
    repairs: u64,
) {
    let obligations = tasks::obligations(spec);
    let effect = summary_effect
        .or_else(|| changes.map(|change| (change.to_owned(), false)))
        .or_else(|| (!spec.change_kind.has_delivery()).then(|| tasks::report_effect(report)));
    let (effect, effect_cut) = tasks::effect_keys(&effect);
    let carried = task
        .feedback
        .as_ref()
        .and_then(|f| f["carried_from"].as_str())
        .map(str::to_owned);
    task.feedback = Some(
        json!({"round":task.round,"effect":effect,"effect_cut":effect_cut,
        "coverage":tasks::coverage(verdict, &obligations)}),
    );
    if let Some(source) = carried {
        task.feedback.as_mut().unwrap()["carried_from"] = source.into();
    }
    if repairs > 0 {
        task.feedback.as_mut().expect("round feedback")["integration_repairs"] = json!(repairs);
    }
}
