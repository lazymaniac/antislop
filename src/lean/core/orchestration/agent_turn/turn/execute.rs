use super::*;
use crate::lean::agent_tools::turn_inputs::TurnInputs;

struct PreparedAgent {
    spec: agent::Spec,
    scope: String,
    native: Arc<native::Session>,
}

pub(super) fn execute(
    core: &Core,
    state: TurnState<'_>,
    cancel: &AtomicBool,
    prompt: impl FnOnce(
        crate::lean::prompt_context::SessionContext,
        &mut crate::lean::agent_tools::turn_inputs::TurnInputs,
    ) -> String,
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
    prompt: impl FnOnce(
        crate::lean::prompt_context::SessionContext,
        &mut crate::lean::agent_tools::turn_inputs::TurnInputs,
    ) -> String,
    events: &TurnEvents<'_>,
) -> Result<agent::Outcome> {
    let _execution = enter_execution(core, state, cancel)?;
    check_task_inputs(core, &state.role)?;
    check_retained_inputs(core, state)?;
    let mut prepared = prepare_agent(core, state, profile)?;
    let mut inputs = TurnInputs::default();
    let text = prompt::prepare_prompt(core, state, &prepared.spec, prompt, &mut inputs)?;
    events.prompt_bytes.set(Some(text.len()));
    check_task_inputs(core, &state.role)?;
    events.announce("agent", json!({"type":"prompt", "text": text}));
    let inputs = Arc::new(std::sync::Mutex::new(inputs));
    let handler = tool_handler(core, state, inputs.clone());
    let outcome = run_cli(core, state, &mut prepared, &text, &handler, cancel, events);
    *inputs.lock().expect("turn inputs mutex") = TurnInputs::default();
    outcome
}

fn enter_execution(
    core: &Core,
    state: &TurnState<'_>,
    cancel: &AtomicBool,
) -> Result<crate::lean::core::execution::Lease> {
    loop {
        setup::wait_until_resumed(core, state.conversation, &state.role, cancel)?;
        let lease = core.execution.enter(&state.turn.id, cancel)?;
        if ORCHESTRATION_ROLES.contains(&state.role.as_str())
            || state.role == PLAN_REVIEWER
            || !core.store.paused(state.conversation)?
        {
            return Ok(lease);
        }
        // The conversation was paused while this turn queued. Do not reserve
        // the only execution slot while waiting for the user to resume it.
        drop(lease);
    }
}

fn clone_error(error: &Error) -> Error {
    Error::Invalid(error.to_string())
}

fn prepare_agent(
    core: &Core,
    state: &TurnState<'_>,
    profile: ResolvedProfile,
) -> Result<PreparedAgent> {
    let (profile_name, effort, spec) = agent_spec(core, state, profile);
    if let Some((id, _reviewer)) = task_role(&state.role) {
        let task = core
            .store
            .task_by_id(id)?
            .ok_or_else(|| Error::Invalid("task no longer exists".into()))?;
        let contract: TaskContract = serde_json::from_value(task.spec.clone())?;
        contract.require_not_deferred()?;
    }
    let scope = session_scope(state, &profile_name, effort, &spec)?;
    let native =
        core.native
            .prepare_scoped(core, state.conversation, &state.role, &spec, &scope)?;
    record_run_context(
        core,
        state,
        &spec,
        &profile_name,
        if spec.provider == crate::config::ProviderKind::Pi {
            // A fresh Pi path is allocated before startup can succeed. Bind
            // it only once this process is ready to receive the prompt;
            // a startup retry may still allocate another path.
            state.resume.clone()
        } else {
            native.identity().or_else(|| state.resume.clone())
        },
    )?;
    Ok(PreparedAgent {
        spec,
        scope,
        native,
    })
}

fn agent_spec(
    core: &Core,
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
        runtime: core.state.clone(),
        add_dirs: state.add_dirs.clone(),

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

        provider_thread_id: (spec.provider == crate::config::ProviderKind::Codex)
            .then(|| identity.clone())
            .flatten(),
        provider_session_id: (spec.provider != crate::config::ProviderKind::Codex)
            .then(|| identity.clone())
            .flatten(),
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

fn check_retained_inputs(core: &Core, state: &TurnState<'_>) -> Result<()> {
    let Some((id, _)) = task_role(&state.role) else {
        return Ok(());
    };
    let task = core
        .store
        .task_by_id(id)?
        .ok_or_else(|| Error::Invalid("task no longer exists".into()))?;
    let contract: TaskContract = serde_json::from_value(task.spec)?;
    core.check_planning_task_inputs(state.conversation, &state.role, &state.turn.id, &contract)?;
    for input in &contract.result_inputs {
        let result = core
            .store
            .historical_result(state.conversation, &input.result_ref)
            .and_then(|reference| core.store.referenced_worker_report(&reference));
        result.map_err(|error| {
            Error::Invalid(format!(
                "task {}: required input {}: {error}",
                contract.id, input.alias
            ))
        })?;
    }
    Ok(())
}

fn tool_handler(
    core: &Core,
    state: &TurnState<'_>,
    inputs: Arc<std::sync::Mutex<TurnInputs>>,
) -> Arc<crate::lean::mcp::Handler> {
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
        let suspension = (!core.config_snapshot().policy.single_agent && coordination_tool(tool))
            .then(|| core.execution.suspend(&turn))
            .flatten();
        let outcome = (|| {
            if tool == "read_turn_input" {
                core.require_open_agent_turn(&conversation, &role, &turn)?;
                return inputs.lock().expect("turn inputs mutex").read(&turn, args);
            }
            inputs
                .lock()
                .expect("turn inputs mutex")
                .require_reads(tool)?;
            core.agent_tool(&conversation, &role, &turn, tool, args)
        })();
        if let Some(suspension) = suspension {
            suspension.resume(|| false)?;
        }
        record_rejected_tool(&core, &conversation, &turn, tool, &outcome)?;
        outcome
    })
}

// Only coordination can need another agent before this turn finishes. Ordinary
// file/evidence tools retain the slot to avoid needless local-model context swaps.
fn coordination_tool(tool: &str) -> bool {
    matches!(
        tool,
        "ask"
            | "consult_planning"
            | "read_planning_result"
            | "prepare_validation"
            | "revise_plan"
            | "request_replan"
            | "resume_task"
            | "retry_task"
            | "answer_agent"
            | "decide_approval"
            | "defer_to_user"
            | "wait_for_answer"
    )
}

fn protocol_observer(
    core: &Core,
    state: &TurnState<'_>,
) -> Arc<crate::lean::mcp::ProtocolObserver> {
    let weak = core.weak.get().expect("core weak reference").clone();
    let conversation = state.conversation.to_owned();
    let turn = state.turn.id.clone();
    Arc::new(move |event| {
        let core = weak
            .upgrade()
            .ok_or_else(|| Error::Invalid("daemon stopped".into()))?;
        core.store
            .append_event(
                &conversation,
                Some(&turn),
                None,
                "agent_tool_rejected",
                event,
            )
            .map(|_| ())
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
            &json!({
                "tool":agent::persisted_error(tool),
                "error":agent::persisted_error(&error.to_string())
            }),
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
    prepared
        .native
        .set_protocol_observer(protocol_observer(core, state))?;
    prepared
        .native
        .set_approver(crate::lean::core::orchestration::approvals::approver(
            core,
            state.conversation,
            &state.role,
            &state.turn.id,
            &prepared.spec,
        ))?;
    let mut attempts: u32 = 0;
    loop {
        let result = prepared
            .native
            .run(&core.native, &state.turn.id, text, cancel);
        if let Err(error) = &result
            && crate::lean::ORCHESTRATION_ROLES.contains(&state.role.as_str())
        {
            if is_rejected_session(error, state.resume.as_deref()) {
                core.store
                    .retire_session(&state.turn.id, &agent::persisted_error(&error.to_string()))?;
            } else if (prepared.spec.provider != crate::config::ProviderKind::Pi
                || !prepared.native.failed_before_start())
                && let Some(identity) = prepared.native.identity()
                && state
                    .resume
                    .as_deref()
                    .is_none_or(|resume| resume == identity)
            {
                core.store.set_session(&state.turn.id, &identity)?;
            }
        }
        if !should_retry(&prepared.native, &result, cancel) {
            return result;
        }
        attempts = attempts.saturating_add(1);
        announce_startup_retry(events, attempts);
        for _ in 0..(30 * u64::from(attempts).min(10)) {
            if cancel.load(std::sync::atomic::Ordering::SeqCst) {
                return result;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        restart_native(core, state, prepared, handler)?;
    }
}

fn is_rejected_session(error: &Error, resumed: Option<&str>) -> bool {
    if resumed.is_none() {
        return false;
    }
    let diagnostic = error.to_string().to_ascii_lowercase();
    (diagnostic.contains("session ") && diagnostic.contains(" was not found"))
        || [
            "session not found",
            "thread not found",
            "unknown session",
            "invalid session id",
            "session has expired",
            "cannot resume session",
            "unable to resume thread",
        ]
        .iter()
        .any(|marker| diagnostic.contains(marker))
}

fn should_retry(
    native: &native::Session,
    result: &Result<agent::Outcome>,
    cancel: &AtomicBool,
) -> bool {
    result.as_ref().is_err_and(|error| {
        !tasks::needs_user_action(&error.to_string())
            && ![
                "[unexpected_dialog]",
                crate::lean::launch::providers::pi::CONFIGURATION_ERROR,
                "unknown option",
                "unknown flag",
                "unrecognized option",
                "unexpected argument",
                "unrecognized arguments",
            ]
            .iter()
            .any(|marker| error.to_string().to_ascii_lowercase().contains(marker))
    }) && native.failed_before_start()
        && !cancel.load(Ordering::SeqCst)
}

fn announce_startup_retry(events: &TurnEvents<'_>, attempts: u32) {
    events.announce("planning_progress", json!({"text":format!("The agent CLI failed while starting; starting it again (attempt {attempts})")}));
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
    prepared.native.set_tool_handler(handler.clone())?;
    prepared
        .native
        .set_protocol_observer(protocol_observer(core, state))?;
    prepared
        .native
        .set_approver(crate::lean::core::orchestration::approvals::approver(
            core,
            state.conversation,
            &state.role,
            &state.turn.id,
            &prepared.spec,
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn rejected_dependency_tool_redacts_persisted_diagnostic_after_reopen() {
        let directory = tempfile::tempdir().unwrap();
        let state = directory.path().join("state");
        let config =
            Config::from_toml(include_str!("../../../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&state, config.clone()).unwrap();
        let conversation = core
            .store
            .create_conversation(directory.path().to_str().unwrap())
            .unwrap()
            .id;
        let tasks = core
            .store
            .add_plan(
                &conversation,
                &[
                    json!({"id":"inventory","title":"Inventory","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"List inputs","change_kind":"none","kind":"analysis"}),
                    json!({"id":"consumer","title":"Consumer","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Use inventory","change_kind":"none","kind":"analysis","depends_on":["inventory"]}),
                ],
            )
            .unwrap();
        let mut inventory = tasks
            .iter()
            .find(|task| task.spec["id"] == "inventory")
            .unwrap()
            .clone();
        inventory.status = crate::lean::tasks::status::APPROVED.into();
        inventory.report = Some("Inventory findings".into());
        core.store.update_task(&inventory).unwrap();
        core.store
            .test_completion(&inventory, inventory.report.as_deref().unwrap());
        let consumer = tasks
            .iter()
            .find(|task| task.spec["id"] == "consumer")
            .unwrap();
        let role = format!("worker:{}", consumer.id);
        let turn = core
            .store
            .start_turn(&conversation, &role, "fixture", 0, Some(&consumer.plan_id))
            .unwrap();

        let outcome = core.agent_tool(
            &conversation,
            &role,
            &turn.id,
            "read_dependency_result",
            &json!({"dependency":"missing token=dependency-secret"}),
        );
        let error = outcome.as_ref().unwrap_err().to_string();
        assert!(error.contains("inventory"), "{error}");
        assert!(error.contains(&consumer.id), "{error}");
        assert!(error.contains("dependency-secret"), "{error}");
        record_rejected_tool(
            &core,
            &conversation,
            &turn.id,
            "read_dependency_result",
            &outcome,
        )
        .unwrap();

        let unknown_tool = "unknown token=tool-secret";
        let unknown_outcome =
            core.agent_tool(&conversation, &role, &turn.id, unknown_tool, &json!({}));
        record_rejected_tool(
            &core,
            &conversation,
            &turn.id,
            unknown_tool,
            &unknown_outcome,
        )
        .unwrap();

        drop(core);
        let reopened = Core::open(&state, config).unwrap();
        let events = reopened.store.events_since(&conversation, 0, 100).unwrap();
        let rejections = events
            .iter()
            .filter(|event| event.kind == "agent_tool_rejected")
            .collect::<Vec<_>>();
        assert_eq!(rejections.len(), 2);
        let rejection = rejections[0];
        assert_eq!(rejection.turn_id.as_deref(), Some(turn.id.as_str()));
        assert_eq!(rejection.payload["tool"], "read_dependency_result");
        assert_eq!(rejection.payload["error"], agent::persisted_error(&error));
        assert!(!rejection.payload.to_string().contains("dependency-secret"));
        assert_eq!(rejections[1].payload["tool"], "unknown [REDACTED]");
        assert!(!rejections[1].payload.to_string().contains("tool-secret"));

        let mut replayed_view = crate::lean::tui::view::View::default();
        for event in &events {
            replayed_view.apply(&serde_json::to_value(event).unwrap());
        }
        assert!(replayed_view.cards.iter().all(|card| !matches!(card,
            crate::lean::tui::view::TranscriptCard::System { title, text, .. }
                if title == "Agent tool rejected" || text.contains("dependency-secret")
        )));
        assert!(
            replayed_view
                .transcript
                .iter()
                .all(|entry| !entry.text.contains("dependency-secret"))
        );
    }
}
