use super::*;

struct PreparedAgent {
    spec: agent::Spec,
    scope: String,
    native: Arc<native::Session>,
}

pub(super) fn execute(
    core: &Core,
    state: TurnState<'_>,
    cancel: &AtomicBool,
    prompt: impl FnOnce(bool, bool) -> String,
    events: &TurnEvents<'_>,
) -> Result<agent::Outcome> {
    let profile = state.profile.as_ref().map_err(clone_error)?.clone();
    execute_profile(core, &state, profile, cancel, prompt, events)
}

fn execute_profile(
    core: &Core,
    state: &TurnState<'_>,
    profile: ResolvedProfile,
    cancel: &AtomicBool,
    prompt: impl FnOnce(bool, bool) -> String,
    events: &TurnEvents<'_>,
) -> Result<agent::Outcome> {
    let mut prepared = prepare_agent(core, state, profile)?;
    let text = prompt::prepare_prompt(core, state, &prepared.spec, prompt)?;
    check_task_inputs(core, &state.role)?;
    let handler = tool_handler(core, state);
    run_cli(core, state, &mut prepared, &text, &handler, cancel, events)
}

fn clone_error(error: &Error) -> Error {
    Error::Invalid(error.to_string())
}

fn prepare_agent(
    core: &Core,
    state: &TurnState<'_>,
    profile: ResolvedProfile,
) -> Result<PreparedAgent> {
    let (profile_name, effort, spec) = agent_spec(state, profile);
    let scope = session_scope(state, &profile_name, effort, &spec)?;
    let native =
        core.native
            .prepare_scoped(core, state.conversation, &state.role, &spec, &scope)?;
    record_run_context(
        core,
        state,
        &spec,
        &profile_name,
        native.identity().or_else(|| state.resume.clone()),
    )?;
    Ok(PreparedAgent {
        spec,
        scope,
        native,
    })
}

fn agent_spec(
    state: &TurnState<'_>,
    profile: ResolvedProfile,
) -> (String, crate::config::Effort, agent::Spec) {
    let name = profile.profile.clone();
    let effort = profile.effort;
    let spec = agent::Spec {
        provider: profile.provider,
        binary: profile.binary,
        args: profile.args,
        model: profile.model,
        effort: profile.native_effort,
        cwd: state.cwd.clone(),
        add_dirs: state.add_dirs.clone(),
        access: state.access,
        resume: state.resume.clone(),
    };
    (name, effort, spec)
}

fn session_scope(
    state: &TurnState<'_>,
    profile: &str,
    effort: crate::config::Effort,
    spec: &agent::Spec,
) -> Result<String> {
    match &state.session_policy {
        SessionPolicy::Role => Ok(String::new()),
        SessionPolicy::Independent => Ok(format!("turn:{}", state.turn.id)),
        policy @ SessionPolicy::TaskRound { .. } => Ok(format!(
            "{policy:?}:{profile}:{}",
            rounds::spec_signature(profile, effort, spec)?
        )),
    }
}

fn record_run_context(
    core: &Core,
    state: &TurnState<'_>,
    spec: &agent::Spec,
    profile: &str,
    identity: Option<String>,
) -> Result<()> {
    core.store
        .record_run_context(&run_context(state, spec, profile, identity.clone()))?;
    if let Some(identity) = identity {
        core.store.set_session(&state.turn.id, &identity)?;
    }
    Ok(())
}

fn run_context(
    state: &TurnState<'_>,
    spec: &agent::Spec,
    profile: &str,
    identity: Option<String>,
) -> store::RunContext {
    store::RunContext {
        turn_id: state.turn.id.clone(),
        conversation: state.conversation.into(),
        role: state.role.clone(),
        provider: state.provider.clone(),
        profile: profile.into(),
        cwd: spec.cwd.clone(),
        add_dirs: spec.add_dirs.clone(),
        access: run_access(state.access),
        provider_thread_id: (spec.provider == crate::config::ProviderKind::Codex)
            .then(|| identity.clone())
            .flatten(),
        provider_session_id: (spec.provider != crate::config::ProviderKind::Codex)
            .then(|| identity.clone())
            .flatten(),
    }
}

fn run_access(access: agent::Access) -> store::RunAccess {
    match access {
        agent::Access::ReadOnly => store::RunAccess::ReadOnly,
        agent::Access::Write => store::RunAccess::Write,
    }
}

#[path = "prompt.rs"]
mod prompt;

fn check_task_inputs(core: &Core, role: &str) -> Result<()> {
    let Some((id, _)) = task_role(role) else {
        return Ok(());
    };
    if let Some(task) = core.store.task_by_id(id)? {
        require(
            task.status != status::STALE,
            "task was invalidated before its process started",
        )?;
        core.store.assert_task_inputs(&task)?;
    }
    Ok(())
}

fn tool_handler(core: &Core, state: &TurnState<'_>) -> Arc<crate::lean::mcp::Handler> {
    let weak = core.weak.get().expect("core weak reference").clone();
    let (conversation, role, turn) = (
        state.conversation.to_owned(),
        state.role.clone(),
        state.turn.id.clone(),
    );
    Arc::new(move |tool, args| {
        let core = weak
            .upgrade()
            .ok_or_else(|| Error::Invalid("daemon stopped".into()))?;
        let outcome = core.agent_tool(&conversation, &role, &turn, tool, args);
        record_rejected_tool(&core, &conversation, &turn, tool, &outcome)?;
        outcome
    })
}

fn record_rejected_tool(
    core: &Core,
    conversation: &str,
    turn: &str,
    tool: &str,
    outcome: &Result<serde_json::Value>,
) -> Result<()> {
    let Err(error) = outcome else {
        return Ok(());
    };
    core.store
        .append_event(
            conversation,
            Some(turn),
            None,
            "agent_tool_rejected",
            &json!({"tool":tool,"error":error.to_string()}),
        )
        .map_err(|logging| {
            Error::Invalid(format!(
                "{error}; recording tool rejection also failed: {logging}"
            ))
        })?;
    Ok(())
}

fn run_cli(
    core: &Core,
    state: &TurnState<'_>,
    prepared: &mut PreparedAgent,
    text: &str,
    handler: &Arc<crate::lean::mcp::Handler>,
    cancel: &AtomicBool,
    events: &TurnEvents<'_>,
) -> Result<agent::Outcome> {
    prepared.native.set_tool_handler(handler.clone())?;
    let mut attempts = 0;
    loop {
        let result = prepared
            .native
            .run(&core.native, &state.turn.id, text, cancel);
        if !should_retry(&prepared.native, &result, attempts, cancel) {
            return result;
        }
        attempts += 1;
        announce_startup_retry(events, attempts);
        std::thread::sleep(std::time::Duration::from_secs(3 * u64::from(attempts)));
        restart_native(core, state, prepared, handler)?;
    }
}

fn should_retry(
    native: &native::Session,
    result: &Result<agent::Outcome>,
    attempts: u32,
    cancel: &AtomicBool,
) -> bool {
    result
        .as_ref()
        .is_err_and(|error| !tasks::needs_user_action(&error.to_string()))
        && native.failed_before_start()
        && attempts < STARTUP_RETRIES
        && !cancel.load(Ordering::SeqCst)
}

fn announce_startup_retry(events: &TurnEvents<'_>, attempts: u32) {
    events.announce(
        "planning_progress",
        json!({"text": format!(
            "The agent CLI failed while starting; starting it again ({attempts}/{STARTUP_RETRIES})"
        )}),
    );
}

fn restart_native(
    core: &Core,
    state: &TurnState<'_>,
    prepared: &mut PreparedAgent,
    handler: &Arc<crate::lean::mcp::Handler>,
) -> Result<()> {
    prepared.native = core.native.prepare_scoped(
        core,
        state.conversation,
        &state.role,
        &prepared.spec,
        &prepared.scope,
    )?;
    prepared.native.set_tool_handler(handler.clone())
}
