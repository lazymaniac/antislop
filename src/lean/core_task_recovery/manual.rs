use super::*;

mod finish;

struct ManualReview {
    snapshot: String,
    base: String,
    stat: String,
    verdict: Verdict,
}

pub(super) fn apply(core: &Arc<Core>, work: &mut RecoveryWork) -> Result<()> {
    let review = prepare_review(core, work)?;
    update_review_state(work, &review)?;
    let fixed =
        git::changed_files(&work.project, &review.base, &review.snapshot).unwrap_or_default();
    work.task.feedback = Some(feedback(&review, &work.spec, &fixed, work.task.round));
    if review.verdict.approved {
        finish::approve(core, work, &review, &fixed)
    } else {
        finish::reject(core, work, &review, &fixed)
    }
}

fn prepare_review(core: &Core, work: &mut RecoveryWork) -> Result<ManualReview> {
    let snapshot = git::snapshot(
        &work.project,
        &format!("manual fix verification: {}", work.spec.title),
    )?;
    let base = work
        .task
        .base
        .clone()
        .unwrap_or_else(|| git::head(&work.project).unwrap_or_default());
    let stat = git::diff_stat(&work.project, &base, &snapshot)?;
    let changes = tasks::changes_text(&base, &snapshot, &stat);
    work.task.round += 1;
    work.task.verdict = None;
    work.task.base = Some(base.clone());
    work.task.result = Some(snapshot.clone());
    core.set_status(&work.req.conversation, &mut work.task, status::VERIFYING)?;
    let verdict = run_verifier(core, work, &snapshot, &changes)?;
    let obligations = tasks::obligations(&work.spec);
    let mut verdict = verdict;
    tasks::settle(&mut verdict, &obligations, !work.spec.writes);
    Ok(ManualReview {
        snapshot,
        base,
        stat,
        verdict,
    })
}

fn run_verifier(
    core: &Core,
    work: &mut RecoveryWork,
    snapshot: &str,
    changes: &str,
) -> Result<Verdict> {
    let (setup, project) = verifier_setup(core, work, snapshot)?;
    let upstream = core.upstream(&work.task.plan_id, &work.spec)?;
    let cancel = AtomicBool::new(false);
    let answer = core.turn(setup, &cancel, |fresh, _| {
        verifier_prompt(
            &project,
            &work.spec,
            work.task.report.as_deref().unwrap_or(""),
            changes,
            fresh,
            &upstream,
        )
    })?;
    decode_answer(core, work, &answer, &cancel)
}

fn verifier_setup<'a>(
    core: &Core,
    work: &'a RecoveryWork,
    snapshot: &str,
) -> Result<(TurnSetup<'a>, String)> {
    let (verifier, reviewdir, branch) = manual_review_paths(core, work);
    git::add_worktree(&work.project, &reviewdir, &branch, snapshot)?;
    let project = reviewdir.to_string_lossy().into_owned();
    let setup = TurnSetup {
        session_policy: SessionPolicy::TaskRound {
            task: work.task.id.clone(),
            round: work.task.round,
        },
        conversation: &work.req.conversation,
        plan_id: Some(work.task.plan_id.clone()),
        role: verifier,
        profile: work.config.resolve(work.spec.kind, RoleKind::Verifier),
        cwd: reviewdir,
        add_dirs: vec![],
        access: agent::Access::ReadOnly,
        upto_message: 0,
    };
    Ok((setup, project))
}

fn manual_review_paths(core: &Core, work: &RecoveryWork) -> (String, PathBuf, String) {
    let suffix = format!(
        "{}-manual-review-g{}-{}",
        work.task.id, work.task.input_generation, work.task.round
    );
    let branch = format!("antislop/{suffix}");
    let dir = core.state.join("worktrees").join(suffix);
    (format!("verifier:{}", work.task.id), dir, branch)
}

fn verifier_prompt(
    project: &str,
    spec: &TaskContract,
    report: &str,
    changes: &str,
    fresh: bool,
    upstream: &str,
) -> String {
    tasks::verifier_turn_prompt(project, spec, report, Some(changes), fresh, upstream, &[])
}

fn decode_answer(
    core: &Core,
    work: &mut RecoveryWork,
    answer: &Option<String>,
    cancel: &AtomicBool,
) -> Result<Verdict> {
    let Some(answer) = answer else {
        return failed_verifier(core, work, cancel);
    };
    match serde_json::from_str::<turn_result::TurnResult>(answer)? {
        turn_result::TurnResult::Review { verdict, .. } => Ok(verdict),
        turn_result::TurnResult::NeedsInput {
            request,
            checkpoint,
        } => awaiting_answer(core, work, &request, checkpoint),
        _ => Err(Error::Invalid(
            "manual snapshot requires an independent review".into(),
        )),
    }
}

fn failed_verifier(core: &Core, work: &mut RecoveryWork, cancel: &AtomicBool) -> Result<Verdict> {
    let verifier = format!("verifier:{}", work.task.id);
    core.stop_task(&work.req.conversation, &mut work.task, &verifier, cancel)?;
    Err(Error::Invalid(
        "manual snapshot verifier failed; inspect its raw turn error".into(),
    ))
}

fn awaiting_answer(
    core: &Core,
    work: &mut RecoveryWork,
    request: &str,
    checkpoint: String,
) -> Result<Verdict> {
    work.task.report = Some(checkpoint);
    core.wait_for_question(&work.req.conversation, &mut work.task, request, true)?;
    Err(Error::Invalid(
        "manual snapshot awaits orchestrator answer".into(),
    ))
}

fn update_review_state(work: &mut RecoveryWork, review: &ManualReview) -> Result<()> {
    let mut value = serde_json::to_value(&review.verdict)?;
    if let Some(object) = value.as_object_mut() {
        object.insert("round".into(), json!(work.task.round));
        object.insert("result".into(), json!(review.snapshot));
    }
    work.task.verdict = Some(value);
    Ok(())
}

fn feedback(
    review: &ManualReview,
    spec: &TaskContract,
    fixed: &[String],
    round: u32,
) -> serde_json::Value {
    let (effect, effect_cut) = tasks::effect(&review.stat, fixed);
    let obligations = tasks::obligations(spec);
    json!({ "round": round, "effect": effect, "effect_cut": effect_cut,
        "coverage": tasks::coverage(&review.verdict, &obligations) })
}
