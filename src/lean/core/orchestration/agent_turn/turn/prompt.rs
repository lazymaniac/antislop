use super::*;

pub(super) fn prepare_prompt(
    core: &Core,
    state: &TurnState<'_>,
    spec: &agent::Spec,
    prompt: impl FnOnce(bool, bool) -> String,
) -> Result<String> {
    let mut text = EXECUTION_POLICY.to_owned();
    text.push_str(&inbox_text(state));
    append_task_context(core, state, spec, &mut text)?;
    append_allocation_context(state, spec, &mut text);
    let introduce = role_introduction(core, state)?;
    text.push_str(&prompt(introduce, state.resume.is_none()));
    append_round_handoff(core, state, spec, &mut text)?;
    Ok(text)
}

const EXECUTION_POLICY: &str = "\nAll roles have full filesystem, shell and network access. Continue implementation, builds, tests and repairs without permission questions or execution budgets. Work in your allocated checkout and preserve results in durable runtime state. Report actual failures and missing inputs honestly; acceptance criteria and dependency correctness require evidence. Ask the user only for information or product decisions the team cannot infer. Manual pause and cancellation remain authoritative.\n";

fn inbox_text(state: &TurnState<'_>) -> String {
    let mut mail = state
        .inbox
        .iter()
        .filter(|item| !state.history_mail.contains(&item.id))
        .peekable();
    if mail.peek().is_none() {
        String::new()
    } else {
        tasks::inbox_text_from(mail)
    }
}

fn append_task_context(
    core: &Core,
    state: &TurnState<'_>,
    spec: &agent::Spec,
    text: &mut String,
) -> Result<()> {
    let Some((id, reviewer)) = task_role(&state.role) else {
        return Ok(());
    };
    let task = core
        .store
        .task_by_id(id)?
        .ok_or_else(|| Error::Invalid("task no longer exists".into()))?;
    if git::is_repository(&spec.cwd) {
        text.push_str(&format!(
            "Input revision: {}; source commit: {}\n",
            task.input_generation,
            git::head(&spec.cwd)?
        ));
    }
    let runtime = crate::lean::core::task::runtime::directory(&core.state, &task)?;
    let sources = crate::lean::core::task::runtime::sources(core, &task)?;
    text.push_str(&format!("Durable task runtime allocation: {}\nStore journals, snapshots, training state, raw measurements and files needed by later tasks in this directory, never inside the disposable checkout. It is outside worktree cleanup and survives restarts and input revisions; older generations stay in its parent until the task is approved, then only the approved generation is kept, so a result must not depend on files in an older generation. Use unique run subdirectories; never overwrite completed runs. When a round settles, entries created before that round began are removed unless the round's report or verdict, a plan task's contract, report or verdict, or this task's evidence names their exact path or their name; cite a run directory to keep it whole, and do not cite superseded or failed runs you no longer need. All roles can use their allocated runtime directory. Build caches may stay in the checkout. Include exact runtime paths in the result; code commits and text evidence alone do not retain these files.\nRuntime handoff sources (availability is directory presence, not content or digest verification): {}\nThese paths supplement the code checkout. Inspect existing files without modifying the producer's state; do not pretend missing states exist; preserve immutable completed measurements.\n", runtime.display(), serde_json::to_string(&sources)?));
    append_questions(core, &state.turn.id, text)?;
    let evidence = core
        .store
        .task_evidence_context(&task.id, task.input_generation)?;
    if !evidence.is_empty() {
        text.push_str(&format!("Historical execution evidence for this task's current inputs (up to 64 most recent records; read_evidence): {}\nThese immutable references survive shortened reports. Read relevant records before claiming a required execution never ran or requesting a repeat. They do not attest the current code: compare what changed since the producing turn. Preserve relevant references with complete's evidence field when amending a report.\n", serde_json::to_string(&evidence)?));
    }
    append_local_checks(&task.spec, text)?;
    text.push_str(if reviewer {
        "Task role: reviewer\n"
    } else {
        "Task role: worker\n"
    });
    Ok(())
}

fn append_questions(core: &Core, turn: &str, text: &mut String) -> Result<()> {
    let questions = core.store.local_questions(turn)?;
    if !questions.is_empty() {
        text.push_str(&format!(
            "Your task questions and answers: {}\n",
            serde_json::to_string(&questions)?
        ));
    }
    Ok(())
}

fn append_local_checks(spec: &serde_json::Value, text: &mut String) -> Result<()> {
    let local: TaskContract = serde_json::from_value(spec.clone())?;
    let obligations = tasks::obligations(&local);
    let ids = obligations.iter().map(|(id, _)| id).collect::<Vec<_>>();
    text.push_str(&format!(
        "Local check ids: {}\n",
        serde_json::to_string(&ids)?
    ));
    Ok(())
}

fn append_allocation_context(state: &TurnState<'_>, spec: &agent::Spec, text: &mut String) {
    if task_role(&state.role).is_none() {
        return;
    }
    let allocation = json!({"turn":state.turn.id,"role":state.role,"cwd":spec.cwd,"session_policy":format!("{:?}",state.session_policy)});
    text.push_str(&format!("Current runtime allocation (for this turn only): {allocation}\nThe current cwd is authoritative for your role, including resumed turns. Paths in earlier reports identify historical allocations; do not copy them as the current checkout or infer cross-checkout equivalence from matching HEAD. Allocation metadata alone does not attest file contents or uncommitted state.\n"));
}

fn role_introduction(core: &Core, state: &TurnState<'_>) -> Result<bool> {
    match &state.resume {
        None => Ok(true),
        Some(session) => Ok(core
            .store
            .session_last_used_ms(state.conversation, session, &state.turn.id)?
            .is_none_or(|used| used < core.opened_ms)),
    }
}

fn append_round_handoff(
    core: &Core,
    state: &TurnState<'_>,
    spec: &agent::Spec,
    text: &mut String,
) -> Result<()> {
    if state.resume.is_some() {
        return Ok(());
    }
    let SessionPolicy::TaskRound { task, round } = &state.session_policy else {
        return Ok(());
    };
    let handoff = core.round_handoff(task, *round, &spec.cwd)?;
    text.push_str(&tasks::round_handoff_prompt(
        &handoff,
        task_role(&state.role).is_some_and(|(_, reviewer)| reviewer),
    ));
    Ok(())
}
