use super::*;
use crate::lean::agent_tools::turn_inputs::{ReadFor, TurnInputs};
use crate::lean::prompts;

const VALIDATION_REUSE: &str = prompts::VALIDATION_REUSE;

pub(super) fn prepare_prompt(
    core: &Core,
    state: &TurnState<'_>,
    spec: &agent::Spec,
    prompt: impl FnOnce(
        crate::lean::prompt_context::SessionContext,
        &mut crate::lean::agent_tools::turn_inputs::TurnInputs,
    ) -> String,
    inputs: &mut TurnInputs,
) -> Result<String> {
    use crate::lean::prompt_context::{Continuation, PromptContext, SessionContext, SessionStart};
    let introduce = role_introduction(core, state)?;
    let session = SessionContext {
        start: if state.resume.is_none() {
            SessionStart::Initial
        } else if introduce {
            SessionStart::Reintroduced
        } else {
            SessionStart::Resumed
        },
        continuation: match &state.session_policy {
            SessionPolicy::TaskRound { round, .. } => Continuation::TaskRound(*round),
            SessionPolicy::Independent => Continuation::IndependentReview,
            SessionPolicy::Role => Continuation::Role,
        },
    };
    let mut context = PromptContext::default();
    context.section(
        prompts::CURRENT_TURN_LABEL,
        format!(
            prompts::template!(CURRENT_TURN),
            task_role(&state.role).map_or_else(
                || format!(prompts::template!(ROLE), state.role),
                |(_, reviewer)| format!(
                    prompts::template!(TASK_ROLE),
                    if reviewer { "reviewer" } else { "worker" }
                )
            ),
            session.start,
            session.continuation
        ),
    );
    context.section(prompts::ROLE_POLICY_LABEL, execution_policy(&state.role));
    if task_role(&state.role).is_some() && core.config_snapshot().policy.single_agent {
        context.section(prompts::EXECUTION_MODE_LABEL, prompts::SINGLE_AGENT_POLICY);
    }
    if crate::lean::mcp::allowed_tools(&state.role).contains(&"prepare_validation")
        && !matches!(
            state.role.as_str(),
            ORCHESTRATOR | COMMENT_ORCHESTRATOR | PLANNER
        )
        && !state
            .profile
            .as_ref()
            .is_ok_and(|profile| profile.work_kind == WorkKind::Options)
    {
        context.section(prompts::VALIDATION_REUSE_LABEL, VALIDATION_REUSE);
        let history = core.store.validation_index(state.conversation, None)?;
        if !history.is_empty() {
            let index: Vec<_> = history.iter().take(8).map(|run| json!({
                "validation":run.id,"spec":run.spec,"status":run.status,"evidence":run.evidence,
            })).collect();
            context.section(
                prompts::PRIOR_VALIDATIONS_LABEL,
                format!(
                    prompts::template!(PRIOR_VALIDATION_INDEX),
                    crate::lean::prompt_context::untrusted(
                        prompts::VALIDATION_INDEX_LABEL,
                        &crate::lean::prompt_context::markdown(&index)
                    )
                ),
            );
        }
    }
    context.section(prompts::INBOX_LABEL, inbox_text(state));
    let assignment = prompt(session, inputs);
    let findings_in_assignment = assignment.contains(prompts::EARLIER_FINDINGS_HEADING);
    context.section(prompts::ASSIGNMENT_LABEL, assignment);
    if task_role(&state.role).is_some() {
        let mut allocation = String::new();
        append_allocation_context(core, state, spec, &mut allocation)?;
        context.section(prompts::ALLOCATION_LABEL, allocation);
        let mut task_context = String::new();
        append_task_context(core, state, spec, &mut task_context, inputs)?;
        context.section(prompts::INPUTS_AND_EVIDENCE_LABEL, task_context);
    }
    let mut handoff = String::new();
    append_round_handoff(
        core,
        state,
        spec,
        findings_in_assignment,
        &mut handoff,
        inputs,
    )?;
    context.section(prompts::CONTINUATION_LABEL, handoff);
    context.prepend_required_inputs(inputs.required_read_instructions());
    let text = context.render();
    core.store.append_event(state.conversation, Some(&state.turn.id), state.turn.plan_id.as_deref(), "prompt_sections", &json!({"body_bytes":text.len(),"sections":context.bytes(),"measurement":"UTF-8 prompt body bytes; excludes footer, provider context and tokens"}))?;
    Ok(text)
}

fn execution_policy(role: &str) -> String {
    let role_rules = if task_role(role).is_some_and(|(_, reviewer)| reviewer)
        || matches!(role, PLAN_REVIEWER | AUDITOR)
        || role.starts_with("code-review:")
    {
        tasks::READ_ONLY_REVIEW
    } else if role.starts_with("planning-")
        || matches!(role, "planner" | "orchestrator" | "comment-orchestrator")
    {
        prompts::COORDINATOR_EXECUTION_RULES
    } else {
        prompts::WORKER_EXECUTION_RULES
    };
    format!(
        prompts::template!(EXECUTION_POLICY),
        tasks::FILESYSTEM_GUIDANCE,
        role_rules = role_rules
    )
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
    inputs: &mut TurnInputs,
) -> Result<()> {
    let Some((id, reviewer)) = task_role(&state.role) else {
        return Ok(());
    };
    let language_sample = core.store.workflow(state.conversation)?.summary;
    if !language_sample.trim().is_empty() {
        text.push_str(&format!(
            prompts::template!(LANGUAGE_GUIDANCE),
            crate::lean::prompt_context::untrusted(
                prompts::LANGUAGE_SAMPLE_LABEL,
                &tasks::bound(&language_sample, 512)
            )
        ));
    }
    let task = core
        .store
        .task_by_id(id)?
        .ok_or_else(|| Error::Invalid("task no longer exists".into()))?;
    if git::is_repository(&spec.cwd) {
        text.push_str(&format!(
            prompts::template!(INPUT_REVISION),
            task.input_generation,
            git::head(&spec.cwd)?
        ));
    }
    let runtime = crate::lean::core::task::runtime::directory(&core.state, &task)?;
    let sources = crate::lean::core::task::runtime::sources(core, &task)?;
    text.push_str(&format!(
        prompts::template!(RUNTIME_ALLOCATION),
        runtime.display(),
        crate::lean::prompt_context::untrusted(
            prompts::RUNTIME_SOURCES_LABEL,
            &crate::lean::prompt_context::markdown(&sources)
        )
    ));
    append_questions(core, &state.turn.id, text)?;
    let evidence = core
        .store
        .task_evidence_context(&task.id, task.input_generation)?;
    if !evidence.is_empty() {
        text.push_str(&format!(
            prompts::template!(HISTORICAL_EVIDENCE_INDEX),
            crate::lean::prompt_context::untrusted(
                prompts::EVIDENCE_INDEX_LABEL,
                &crate::lean::prompt_context::markdown(&evidence)
            )
        ));
    }
    let contract: TaskContract = serde_json::from_value(task.spec.clone())?;
    if let Some(owner) = &contract.options_for
        && let Some(owner) = core.store.task_in_plan_by_id(&task.plan_id, owner)?
    {
        let label = format!("Options owner contract for {}", owner.spec["id"]);
        let required = if reviewer {
            ReadFor::Review
        } else {
            ReadFor::Work
        };
        text.push_str(&inputs.block(
            &label,
            &crate::lean::prompt_context::markdown(&owner.spec),
            required,
        ));
        text.push('\n');
    }
    if !contract.planning_inputs.is_empty() {
        text.push_str("Required planning reports: call read_planning_result with each exact reference and follow its cursor.\n");
        text.push_str(&crate::lean::prompt_context::markdown(
            &contract.planning_inputs,
        ));
        text.push('\n');
    }
    if !contract.result_inputs.is_empty() {
        text.push_str(prompts::HISTORICAL_REPORT_INPUTS);
        text.push_str(&crate::lean::prompt_context::markdown(
            &contract.result_inputs,
        ));
    }
    if contract.goal.contains("antislop activate") {
        text.push_str(tasks::ACTIVATION_GUIDANCE);
    }
    Ok(())
}

fn append_questions(core: &Core, turn: &str, text: &mut String) -> Result<()> {
    let questions = core.store.local_questions(turn)?;
    if !questions.is_empty() {
        text.push_str(&format!(
            prompts::template!(QUESTION_HISTORY),
            crate::lean::prompt_context::untrusted(
                prompts::QUESTION_HISTORY_LABEL,
                &crate::lean::prompt_context::markdown(&questions)
            )
        ));
    }
    Ok(())
}

fn append_allocation_context(
    core: &Core,
    state: &TurnState<'_>,
    spec: &agent::Spec,
    text: &mut String,
) -> Result<()> {
    let Some((task, _)) = task_role(&state.role) else {
        return Ok(());
    };
    let allocation = json!({"cwd":spec.cwd,
        "conversation":state.conversation,"task":task,"runtime_state":core.state,
        "runtime_binary":std::env::current_exe()?});
    text.push_str(&format!(
        prompts::template!(APPEND_ALLOCATION_CONTEXT),
        crate::lean::prompt_context::markdown(&allocation)
    ));
    Ok(())
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
    findings_in_assignment: bool,
    text: &mut String,
    inputs: &mut TurnInputs,
) -> Result<()> {
    let SessionPolicy::TaskRound { task, round } = &state.session_policy else {
        return Ok(());
    };
    let handoff = core.round_handoff(task, *round, &spec.cwd)?;
    if task_role(&state.role).is_some_and(|(_, reviewer)| reviewer)
        && !findings_in_assignment
        && let Some(findings) = handoff
            .verdict
            .as_ref()
            .and_then(|v| v["findings"].as_array())
    {
        text.push_str(&tasks::earlier_findings(findings, inputs));
    }
    text.push_str(&tasks::round_handoff_prompt(
        &handoff,
        task_role(&state.role).is_some_and(|(_, reviewer)| reviewer),
        inputs,
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lean::prompt_context::SessionStart;

    #[test]
    fn completed_prompt_layers_preserve_contracts_and_measure_sections_for_every_rigor_and_session()
    {
        let (directory, core, conversation) = crate::lean::agent_tools::tests::core();
        git::ok(
            directory.path(),
            &["init", "--quiet", "--initial-branch=main"],
        )
        .unwrap();
        std::fs::write(directory.path().join(".gitignore"), "state/\n").unwrap();
        git::commit_all(directory.path(), git::CommitPurpose::TestFixture).unwrap();
        for team in tasks::Team::ALL {
            for delivery in ["none", "artifact", "untracked", "code"] {
                let contract: TaskContract = serde_json::from_value(json!({
                    "id":"subject", "title":"Subject", "goal":"GOAL_SENTINEL preserve e\u{301} and 👩‍💻",
                    "kind":"analysis", "change_kind":if delivery == "untracked" {"artifact"} else {delivery},
                    "delivery":if delivery == "untracked" {json!("untracked")} else {json!(null)},
                    "outputs":if matches!(delivery, "artifact" | "untracked") {json!(["report.md"])} else {json!([])},
                    "acceptance_criteria":[{"id":"C1","text":"CRITERION_SENTINEL retains every qualifier"}],
                })).unwrap();
                let specs =
                    crate::lean::core::plan::helpers::finalize_team_plan(vec![contract], team);
                let tasks = core
                    .store
                    .add_plan(
                        &conversation,
                        &specs
                            .iter()
                            .map(|s| serde_json::to_value(s).unwrap())
                            .collect::<Vec<_>>(),
                    )
                    .unwrap();
                let task = tasks
                    .iter()
                    .find(|task| task.spec["id"] == "subject")
                    .unwrap();
                let contract: TaskContract = serde_json::from_value(task.spec.clone()).unwrap();
                for reviewer in [false, true] {
                    let role = format!(
                        "{}:{}",
                        if reviewer { "reviewer" } else { "worker" },
                        task.id
                    );
                    for start in [
                        SessionStart::Initial,
                        SessionStart::Resumed,
                        SessionStart::Reintroduced,
                    ] {
                        let session = format!("{role}-{start:?}");
                        if start != SessionStart::Initial {
                            let previous = core
                                .store
                                .start_turn(&conversation, &role, "fixture", 0, Some(&task.plan_id))
                                .unwrap();
                            core.store.set_session(&previous.id, &session).unwrap();
                            core.store
                                .finish_turn(&previous.id, store::TurnStatus::Done, None)
                                .unwrap();
                            if start == SessionStart::Reintroduced {
                                core.store
                                    .db()
                                    .execute(
                                        "UPDATE turns SET started_ms=?1 WHERE id=?2",
                                        rusqlite::params![core.opened_ms - 1, previous.id],
                                    )
                                    .unwrap();
                            }
                        }
                        let turn = core
                            .store
                            .start_turn(&conversation, &role, "fixture", 0, Some(&task.plan_id))
                            .unwrap();
                        let project =
                            PathBuf::from(core.store.conversation(&conversation).unwrap().project);
                        let state = TurnState {
                            session_policy: SessionPolicy::TaskRound {
                                task: task.id.clone(),
                                round: task.round,
                            },
                            conversation: &conversation,
                            role: role.clone(),
                            profile: core.config_snapshot().resolve(
                                WorkKind::Analysis,
                                if reviewer {
                                    RoleKind::Reviewer
                                } else {
                                    RoleKind::Executor
                                },
                            ),
                            cwd: project.clone(),
                            add_dirs: vec![],
                            provider: "fixture".into(),
                            resume: (start != SessionStart::Initial).then_some(session),
                            inbox: vec![],
                            history_mail: Default::default(),
                            comments: vec![],
                            turn,
                        };
                        let spec = agent::Spec {
                            provider: crate::config::ProviderKind::Codex,
                            binary: "unused-preview".into(),
                            args: vec![],
                            model: None,
                            effort: None,
                            cwd: project.clone(),
                            runtime: core.state.clone(),
                            add_dirs: vec![],
                            resume: state.resume.clone(),
                        };
                        let prompt = prepare_prompt(
                            &core,
                            &state,
                            &spec,
                            |session, _| {
                                assert_eq!(session.start, start);
                                if reviewer {
                                    tasks::reviewer_turn_prompt(
                                        &project.to_string_lossy(),
                                        &contract,
                                        "RESULT_SENTINEL",
                                        None,
                                        session.introduce(),
                                        "",
                                        &[],
                                    )
                                } else {
                                    tasks::worker_turn_prompt(
                                        &project.to_string_lossy(),
                                        &contract,
                                        None,
                                        "",
                                    )
                                }
                            },
                            &mut TurnInputs::default(),
                        )
                        .unwrap();
                        assert_eq!(
                            prompt.matches("GOAL_SENTINEL").count(),
                            1,
                            "{} {delivery} {start:?}",
                            team.name()
                        );
                        assert_eq!(prompt.matches("CRITERION_SENTINEL").count(), 1);
                        assert_eq!(prompt.matches("Current runtime allocation").count(), 1);
                        assert!(
                            !prompt.contains("always allowed")
                                && !prompt.contains("full filesystem access")
                        );
                        assert!(
                            !prompt.contains("```json")
                                && !prompt.contains("antislop activate --binary")
                        );
                        for (id, text) in tasks::obligations(&contract).iter().skip(1) {
                            assert!(prompt.contains(id) && prompt.contains(text));
                        }
                        let event = core
                            .store
                            .events_since(&conversation, 0, usize::MAX)
                            .unwrap()
                            .into_iter()
                            .rev()
                            .find(|event| event.kind == "prompt_sections")
                            .unwrap();
                        assert_eq!(event.payload["body_bytes"], prompt.len());
                        let total: u64 = event.payload["sections"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|part| part[1].as_u64().unwrap())
                            .sum();
                        assert_eq!(total, prompt.len() as u64);
                        core.store
                            .finish_turn(&state.turn.id, store::TurnStatus::Done, None)
                            .unwrap();
                    }
                }
            }
        }
    }
}
