use super::*;

pub(super) fn prepare_prompt(
    core: &Core,
    state: &TurnState<'_>,
    spec: &agent::Spec,
    prompt: impl FnOnce(bool, bool) -> String,
) -> Result<String> {
    let mut text = inbox_text(state);
    append_task_context(core, state, spec, &mut text)?;
    append_allocation_context(state, spec, &mut text);
    let introduce = role_introduction(core, state)?;
    text.push_str(&prompt(introduce, state.resume.is_none()));
    append_round_handoff(core, state, spec, &mut text)?;
    Ok(text)
}

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
    let Some((id, verifier)) = task_role(&state.role) else {
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
    append_questions(core, &state.turn.id, text)?;
    append_local_checks(&task.spec, text)?;
    text.push_str(if verifier {
        "Task role: verifier\n"
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
        task_role(&state.role).is_some_and(|(_, verifier)| verifier),
    ));
    Ok(())
}
