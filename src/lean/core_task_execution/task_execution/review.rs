use super::super::super::core_path_helpers::task_path_component;
use super::super::super::*;

pub(super) struct ReviewRound<'a> {
    pub core: &'a Arc<Core>,
    pub conversation: &'a str,
    pub task: &'a mut Task,
    pub cancel: &'a AtomicBool,
    pub spec: &'a TaskContract,
    pub project: &'a Path,
    pub workdir: &'a Path,
    pub add_dirs: &'a [PathBuf],
    pub upstream: &'a str,
    pub verifier: &'a str,
    pub round: u32,
    pub findings: &'a [serde_json::Value],
    pub report: &'a str,
    pub changes: Option<&'a str>,
}

pub(super) fn run(mut input: ReviewRound<'_>) -> Result<Option<Verdict>> {
    let answer = request_review(&mut input)?;
    let Some(answer) = answer else {
        input
            .core
            .stop_task(input.conversation, input.task, input.verifier, input.cancel)?;
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
        .set_status(input.conversation, input.task, status::VERIFYING)?;
    let reviewdir = review_worktree(
        input.core,
        input.task,
        input.project,
        input.workdir,
        input.round,
    )?;
    request_review_turn(input, &reviewdir)
}

fn request_review_turn(input: &ReviewRound<'_>, reviewdir: &Path) -> Result<Option<String>> {
    let setup = verifier_setup(
        input.core,
        input.conversation,
        input.task,
        input.spec,
        reviewdir,
        input.add_dirs,
        input.verifier,
    );
    let checks = input
        .core
        .check_comments(input.conversation, &input.task.plan_id, &input.spec.id);
    run_review_turn(
        input.core,
        input.cancel,
        setup,
        reviewdir,
        input.spec,
        input.report,
        input.changes,
        input.round,
        input.upstream,
        input.findings,
        checks,
    )
}

fn run_review_turn(
    core: &Core,
    cancel: &AtomicBool,
    setup: TurnSetup<'_>,
    reviewdir: &Path,
    spec: &TaskContract,
    report: &str,
    changes: Option<&str>,
    round: u32,
    upstream: &str,
    findings: &[serde_json::Value],
    user_checks: String,
) -> Result<Option<String>> {
    core.turn(setup, cancel, |first, _| {
        let prompt = tasks::verifier_turn_prompt(
            &reviewdir.to_string_lossy(),
            spec,
            report,
            changes,
            first || round == 1,
            upstream,
            findings,
        );
        format!("{user_checks}{prompt}")
    })
}

fn review_worktree(
    core: &Core,
    task: &Task,
    project: &Path,
    workdir: &Path,
    round: u32,
) -> Result<PathBuf> {
    let review_commit = task.result.clone().or_else(|| git::head(workdir).ok());
    let Some(result) = review_commit else {
        return Ok(workdir.to_path_buf());
    };
    let path = core.state.join("worktrees").join(format!(
        "{}-review-g{}-{round}",
        task_path_component(&task.id)?,
        task.input_generation
    ));
    let branch = format!(
        "antislop/{}-review-g{}-{round}",
        task.id, task.input_generation
    );
    git::add_worktree(project, &path, &branch, &result)?;
    Ok(path)
}

fn verifier_setup<'a>(
    core: &Core,
    conversation: &'a str,
    task: &Task,
    spec: &TaskContract,
    reviewdir: &Path,
    add_dirs: &[PathBuf],
    verifier: &str,
) -> TurnSetup<'a> {
    TurnSetup {
        session_policy: SessionPolicy::TaskRound {
            task: task.id.clone(),
            round: task.round,
        },
        conversation,
        plan_id: Some(task.plan_id.clone()),
        role: verifier.to_owned(),
        profile: core
            .config_snapshot()
            .resolve(spec.kind, RoleKind::Verifier),
        cwd: reviewdir.to_path_buf(),
        add_dirs: add_dirs.to_vec(),
        access: agent::Access::ReadOnly,
        upto_message: 0,
    }
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
        _ => Err(Error::Invalid("verifier did not submit a review".into())),
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

pub(super) fn save_verdict(
    core: &Core,
    conversation: &str,
    task: &mut Task,
    spec: &TaskContract,
    mut verdict: Verdict,
    report: &str,
    changes: Option<&str>,
    summary_effect: Option<(String, bool)>,
    repairs: u64,
) -> Result<()> {
    stamp_verdict(task, &mut verdict);
    task.verdict = Some(serde_json::to_value(&verdict)?);
    set_feedback(
        task,
        spec,
        &verdict,
        report,
        changes,
        summary_effect,
        repairs,
    );
    core.store.update_task(task)?;
    let _ = conversation;
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
        .or_else(|| (!spec.writes).then(|| tasks::report_effect(report)));
    let (effect, effect_cut) = tasks::effect_keys(&effect);
    task.feedback = Some(
        json!({"round":task.round,"effect":effect,"effect_cut":effect_cut,
        "coverage":tasks::coverage(verdict, &obligations)}),
    );
    if repairs > 0 {
        task.feedback.as_mut().expect("round feedback")["integration_repairs"] = json!(repairs);
    }
}
