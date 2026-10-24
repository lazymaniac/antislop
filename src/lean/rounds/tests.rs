use super::*;

fn fixture() -> (tempfile::TempDir, Arc<Core>, String, Task) {
    let dir = tempfile::tempdir().unwrap();
    let config = Config::from_toml(include_str!("../../../examples/antislop.toml")).unwrap();
    let core = Core::open(&dir.path().join(".antislop"), config).unwrap();
    let conversation = core
        .store
        .create_conversation(dir.path().to_str().unwrap())
        .unwrap();
    let message = core
        .store
        .add_message(&conversation.id, "user", "Keep all context")
        .unwrap();
    let run = core
        .store
        .create_run(&conversation.id, "Keep all context")
        .unwrap();
    let source = core
        .store
        .register_message_source(&conversation.id, &run.id, message.seq)
        .unwrap();
    let change = serde_json::from_value(json!({
        "conversation":conversation.id,"run_id":run.id,"expected_spec_revision":0,
        "operation_id":crate::id(),"reason":"test scope", "entries":[{
            "id":"R1", "source":{"source_id":source.id,"start":0,"end":message.text.len()},
            "interpretation":"Keep all context", "criteria":[{"id":"R1.accept","text":"Check everything"}]
        }]
    })).unwrap();
    core.store
        .apply_ledger_change(&change, &core.state, true)
        .unwrap();
    let task = core
        .store
        .add_plan(
            &conversation.id,
            &[json!({
                "requirement_ids":["R1"], "criterion_ids":["R1.accept"],
            "id":"task", "title":"Round task", "goal":"Keep all context", "writes":false,
                "kind":"implementation", "verifier":{"focus":"Check everything", "risks":[]}
            })],
        )
        .unwrap()
        .remove(0);
    core.store
        .link_plan_run(&conversation.id, &task.plan_id, &run.id)
        .unwrap();
    core.store
        .bind_task_ledger(&conversation.id, &run.id, &task.plan_id)
        .unwrap();
    (dir, core, conversation.id, task)
}

#[test]
fn sessions_follow_persisted_round_and_compatible_execution_context() {
    let (_dir, core, conversation, mut task) = fixture();
    let profile = core
        .config_snapshot()
        .resolve(WorkKind::Implementation, RoleKind::Executor)
        .unwrap();
    let cwd = PathBuf::from(core.store.conversation(&conversation).unwrap().project);
    for role in [
        format!("worker:{}", task.id),
        format!("verifier:{}", task.id),
    ] {
        task.round = 1;
        core.store.update_task(&task).unwrap();
        let turn = core
            .store
            .start_turn(
                &conversation,
                &role,
                agent::name(profile.provider),
                0,
                Some(&task.plan_id),
            )
            .unwrap();
        core.store
            .record_run_context(&store::RunContext {
                turn_id: turn.id.clone(),
                conversation: conversation.clone(),
                role: role.clone(),
                provider: agent::name(profile.provider).into(),
                profile: profile.profile.clone(),
                cwd: cwd.clone(),
                add_dirs: vec![],
                access: store::RunAccess::ReadOnly,
                backend: store::RunBackend::Cli,
                provider_thread_id: None,
                provider_session_id: None,
                provider_turn_id: None,
            })
            .unwrap();
        core.store
            .append_event(
                &conversation,
                Some(&turn.id),
                Some(&task.plan_id),
                "turn_started",
                &json!({"execution_signature":execution_signature(&profile).unwrap()}),
            )
            .unwrap();
        core.store
            .set_session(&turn.id, "round-one-session")
            .unwrap();
        let lookup = |round, cwd: &Path, access| {
            core.session_for_turn(
                &conversation,
                &role,
                agent::name(profile.provider),
                &SessionPolicy::TaskRound {
                    task: task.id.clone(),
                    round,
                },
                Some(&profile),
                cwd,
                &[],
                access,
            )
            .unwrap()
        };
        assert_eq!(
            lookup(1, &cwd, agent::Access::ReadOnly).as_deref(),
            Some("round-one-session")
        );
        assert_eq!(lookup(2, &cwd, agent::Access::ReadOnly), None);
        assert_eq!(lookup(1, &cwd.join("other"), agent::Access::ReadOnly), None);
        assert_eq!(lookup(1, &cwd, agent::Access::Write), None);
        let mut changed_profile = profile.clone();
        changed_profile.profile.push_str("-changed");
        assert!(
            core.session_for_turn(
                &conversation,
                &role,
                agent::name(profile.provider),
                &SessionPolicy::TaskRound {
                    task: task.id.clone(),
                    round: 1
                },
                Some(&changed_profile),
                &cwd,
                &[],
                agent::Access::ReadOnly
            )
            .unwrap()
            .is_none()
        );
        for field in ["model", "effort", "configured_effort", "binary", "args"] {
            let mut changed = profile.clone();
            match field {
                "model" => changed.model = Some("another-model".into()),
                "effort" => changed.native_effort = Some("another-effort".into()),
                "configured_effort" => {
                    changed.effort = if profile.effort == crate::config::Effort::Low {
                        crate::config::Effort::High
                    } else {
                        crate::config::Effort::Low
                    }
                }
                "binary" => changed.binary = cwd.join("different-cli"),
                "args" => changed.args.push("--different".into()),
                _ => unreachable!(),
            }
            assert!(
                core.session_for_turn(
                    &conversation,
                    &role,
                    agent::name(profile.provider),
                    &SessionPolicy::TaskRound {
                        task: task.id.clone(),
                        round: 1
                    },
                    Some(&changed),
                    &cwd,
                    &[],
                    agent::Access::ReadOnly
                )
                .unwrap()
                .is_none(),
                "{field}"
            );
        }
        let other = core
            .store
            .start_turn(
                &conversation,
                &role,
                "other-provider",
                0,
                Some(&task.plan_id),
            )
            .unwrap();
        let mut context = core
            .store
            .run_context(&conversation, &turn.id)
            .unwrap()
            .unwrap();
        context.turn_id = other.id.clone();
        context.provider = "other-provider".into();
        context.provider_thread_id = None;
        context.provider_session_id = None;
        core.store.record_run_context(&context).unwrap();
        core.store.set_session(&other.id, "other-session").unwrap();
        assert_eq!(lookup(1, &cwd, agent::Access::ReadOnly), None);
    }
}

#[test]
fn handoff_recovers_latest_report_after_interruption_without_older_reports() {
    let (_dir, core, conversation, mut task) = fixture();
    // This also exercises a read-only task outside Git.
    let cwd = core.state.parent().unwrap();
    task.round = 1;
    task.report = Some("OLD REPORT MUST NOT APPEAR".into());
    core.store.update_task(&task).unwrap();
    let historical = core
        .store
        .start_turn(
            &conversation,
            &format!("worker:{}", task.id),
            "codex",
            0,
            Some(&task.plan_id),
        )
        .unwrap();
    core.store
        .append_event(
            &conversation,
            Some(&historical.id),
            Some(&task.plan_id),
            "agent",
            &json!({"type":"tool","name":"shell","detail":"HISTORICAL TOOL NEVER IN HANDOFF"}),
        )
        .unwrap();
    core.store
        .finish_turn(&historical.id, TurnStatus::Done, None)
        .unwrap();
    task.round = 2;
    task.report = Some("Latest complete report\n".repeat(2000));
    task.verdict = Some(json!({"findings":[{"message":"Fix this", "evidence":"file:12"}]}));
    core.store.update_task(&task).unwrap();
    task.round = 3;
    task.report = Some("[failed] CLI failure before producing a report".into());
    task.verdict = None;
    core.store.update_task(&task).unwrap();
    let turn = core
        .store
        .start_turn(
            &conversation,
            &format!("worker:{}", task.id),
            "codex",
            0,
            Some(&task.plan_id),
        )
        .unwrap();
    core.store
        .finish_turn(&turn.id, TurnStatus::Interrupted, Some("CLI failure"))
        .unwrap();
    let handoff = core.round_handoff(&task.id, 4, cwd).unwrap();
    assert!(handoff.interrupted);
    assert_eq!(handoff.report.as_ref().unwrap().0, 2);
    let prompt = tasks::round_handoff_prompt(&handoff, false);
    assert!(prompt.contains(&"Latest complete report\n".repeat(2000)));
    assert!(prompt.contains("Keep all context"));
    assert!(prompt.contains("Check everything"));
    assert!(prompt.contains("file:12"));
    assert!(!prompt.contains("OLD REPORT MUST NOT APPEAR"));
    assert!(!prompt.contains("HISTORICAL TOOL NEVER IN HANDOFF"));
    assert!(handoff.files.is_empty());
}
#[test]
fn cli_rounds_rotate_and_same_round_followups_resume_after_restart() {
    let (dir, core, conversation, mut task) = fixture();
    let script = dir.path().join("fake-codex");
    std::fs::write(
        &script,
        r#"#!/bin/sh
session="session-$$"
for arg; do
  case "$arg" in session-*) session="$arg" ;; esac
done
printf '{"type":"thread.started","thread_id":"%s"}\n' "$session"
echo '{"type":"item.completed","item":{"type":"agent_message","text":"complete report"}}'
echo '{"type":"turn.completed"}'
"#,
    )
    .unwrap();
    std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let mut previous = HashMap::new();
    for round in 1..=2 {
        task.round = round;
        task.report = Some(format!("report for round {round}"));
        core.store.update_task(&task).unwrap();
        for role in [
            format!("worker:{}", task.id),
            format!("verifier:{}", task.id),
        ] {
            for _ in 0..2 {
                let mut profile = core
                    .config_snapshot()
                    .resolve(WorkKind::Orchestration, RoleKind::Orchestrator)
                    .unwrap();
                profile.provider = crate::config::ProviderKind::Codex;
                profile.binary = script.clone();
                profile.args = vec!["--fake-cli".into()];
                let setup = TurnSetup {
                    session_policy: SessionPolicy::TaskRound {
                        task: task.id.clone(),
                        round,
                    },
                    conversation: &conversation,
                    plan_id: Some(task.plan_id.clone()),
                    role: role.clone(),
                    profile: Ok(profile),
                    cwd: dir.path().into(),
                    add_dirs: vec![],
                    access: agent::Access::ReadOnly,
                    upto_message: 0,
                };
                assert_eq!(
                    core.turn(setup, &AtomicBool::new(false), |_| "Do the task".into())
                        .unwrap()
                        .as_deref(),
                    Some("complete report")
                );
            }
            let sessions = core.store.sessions(&conversation).unwrap();
            let turns: Vec<_> = sessions.iter().filter(|s| s.role == role).collect();
            assert_eq!(
                turns[turns.len() - 1].session_id,
                turns[turns.len() - 2].session_id
            );
            let session = turns.last().unwrap().session_id.clone();
            if let Some(old) = previous.insert(role, session.clone()) {
                assert_ne!(old, session);
            }
        }
    }
    let state = core.state.clone();
    let config = (*core.config_snapshot()).clone();
    drop(core);
    let core = Core::open(&state, config).unwrap();
    let handoff = core.round_handoff(&task.id, 3, dir.path()).unwrap();
    assert_eq!(handoff.report.unwrap(), (2, "complete report".into()));
    let role = format!("worker:{}", task.id);
    let mut profile = core
        .config_snapshot()
        .resolve(WorkKind::Orchestration, RoleKind::Orchestrator)
        .unwrap();
    profile.binary = script;
    profile.args = vec!["--fake-cli".into()];
    for (round, expected) in [(2, true), (3, false)] {
        assert_eq!(
            core.session_for_turn(
                &conversation,
                &role,
                "codex",
                &SessionPolicy::TaskRound {
                    task: task.id.clone(),
                    round
                },
                Some(&profile),
                dir.path(),
                &[],
                agent::Access::ReadOnly
            )
            .unwrap()
            .is_some(),
            expected
        );
    }
}

#[test]
fn file_history_includes_deletions_renames_and_restored_paths() {
    let (dir, core, _conversation, mut task) = fixture();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    };
    git(&["init"]);
    git(&["config", "user.name", "Test"]);
    git(&["config", "user.email", "test@example.com"]);
    std::fs::write(dir.path().join("deleted"), "delete").unwrap();
    std::fs::write(dir.path().join("old-name"), "rename").unwrap();
    std::fs::write(dir.path().join("restored"), "original").unwrap();
    git(&["add", "deleted", "old-name", "restored"]);
    git(&["commit", "-m", "base"]);
    let base = git(&["rev-parse", "HEAD"]);
    std::fs::remove_file(dir.path().join("deleted")).unwrap();
    std::fs::rename(dir.path().join("old-name"), dir.path().join("new-name")).unwrap();
    std::fs::write(dir.path().join("restored"), "temporary").unwrap();
    std::fs::write(dir.path().join("added"), "added").unwrap();
    git(&[
        "add", "deleted", "old-name", "new-name", "restored", "added",
    ]);
    git(&["commit", "-m", "round one"]);
    task.round = 1;
    task.base = Some(base.clone());
    task.result = Some(git(&["rev-parse", "HEAD"]));
    core.store.update_task(&task).unwrap();
    std::fs::write(dir.path().join("restored"), "original").unwrap();
    git(&["add", "restored"]);
    git(&["commit", "-m", "restore"]);
    task.round = 2;
    task.result = Some(git(&["rev-parse", "HEAD"]));
    core.store.update_task(&task).unwrap();
    task.status = "approved".into();
    core.store.update_task(&task).unwrap();
    core.store
        .enqueue_agent_message(&store::AgentMessage {
            conversation: core.store.plan_conversation(&task.plan_id).unwrap(),
            plan: Some(task.plan_id.clone()),
            recipient: format!("worker:{}", task.id),
            operation_id: crate::id(),
            expected_round: Some(2),
            expected_run: None,
            text: "Review this result again".into(),
        })
        .unwrap();
    let queued = core.store.task_by_id(&task.id).unwrap().unwrap();
    assert!(queued.result.is_none());
    core.store.update_task(&queued).unwrap();
    let handoff = core.round_handoff(&task.id, 3, dir.path()).unwrap();
    assert_eq!(handoff.result, task.result);
    assert_eq!(
        handoff.files,
        ["added", "deleted", "new-name", "old-name", "restored"]
    );
    assert!(
        !git::changed_files(dir.path(), &base, task.result.as_deref().unwrap())
            .unwrap()
            .contains(&"restored".into())
    );
}
#[test]
fn missing_report_content_and_git_objects_surface_the_original_failure() {
    let (dir, core, _conversation, mut task) = fixture();
    task.round = 1;
    task.report = Some("complete report".into());
    core.store.update_task(&task).unwrap();
    let records = core.store.handoff_rounds(&task.id, 1).unwrap();
    let reference = records[0].1["report"]["session_content"]["id"]
        .as_str()
        .unwrap();
    std::fs::remove_file(core.state.join("session-content").join(reference)).unwrap();
    task.round = 2;
    task.report = None;
    core.store.update_task(&task).unwrap();
    let error = core
        .round_handoff(&task.id, 2, dir.path())
        .unwrap_err()
        .to_string();
    assert!(error.contains("No such file"), "{error}");
    task.round = 1;
    task.report = Some("restored report".into());
    task.base = Some("missing-base".into());
    task.result = Some("missing-result".into());
    core.store.update_task(&task).unwrap();
    let error = core
        .round_handoff(&task.id, 2, dir.path())
        .unwrap_err()
        .to_string();
    assert!(error.contains("git diff:"), "{error}");
}
#[test]
fn final_answer_survives_restart_before_round_snapshot_is_updated() {
    let (_dir, core, conversation, mut task) = fixture();
    task.round = 1;
    task.report = None;
    core.store.update_task(&task).unwrap();
    let turn = core
        .store
        .start_turn(
            &conversation,
            &format!("worker:{}", task.id),
            "codex",
            0,
            Some(&task.plan_id),
        )
        .unwrap();
    core.store
        .append_event(
            &conversation,
            Some(&turn.id),
            Some(&task.plan_id),
            "agent_final",
            &json!({"text":"Complete report before scheduler crash"}),
        )
        .unwrap();
    let state = core.state.clone();
    let cwd = state.parent().unwrap().to_path_buf();
    let config = (*core.config_snapshot()).clone();
    drop(core);
    let core = Core::open(&state, config).unwrap();
    let receipt = core
        .store
        .start_turn(
            &conversation,
            &format!("worker:{}", task.id),
            "codex",
            0,
            Some(&task.plan_id),
        )
        .unwrap();
    core.store
        .append_event(
            &conversation,
            Some(&receipt.id),
            Some(&task.plan_id),
            "turn_started",
            &json!({"mail_only":true}),
        )
        .unwrap();
    core.store
        .append_event(
            &conversation,
            Some(&receipt.id),
            Some(&task.plan_id),
            "agent_final",
            &json!({"text":"Mail receipt must not replace the report"}),
        )
        .unwrap();
    let handoff = core.round_handoff(&task.id, 2, &cwd).unwrap();
    assert_eq!(
        handoff.report,
        Some((1, "Complete report before scheduler crash".into()))
    );
}
fn fake_cli(dir: &Path) -> PathBuf {
    let binary = dir.join("offline-codex");
    std::fs::write(
        &binary,
        r#"#!/bin/sh
session="session-$$"
reject=false
resumed=false
omit_session=false
for arg; do
  case "$arg" in session-*) session="$arg"; resumed=true ;; --reject-context) reject=true ;; --omit-resumed-session-id) omit_session=true ;; esac
done
if [ "$resumed" != true ] || [ "$omit_session" != true ]; then
  printf '{"type":"thread.started","thread_id":"%s"}\n' "$session"
fi
if [ "$reject" = true ]; then
  echo 'context_length_exceeded: full handoff rejected by offline provider' >&2
  exit 1
fi
echo '{"type":"item.completed","item":{"type":"agent_message","text":"complete report"}}'
echo '{"type":"turn.completed"}'
"#,
    )
    .unwrap();
    std::fs::set_permissions(&binary, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    binary
}

fn fake_profile(core: &Core, binary: &Path) -> ResolvedProfile {
    let mut profile = core
        .config_snapshot()
        .resolve(WorkKind::Orchestration, RoleKind::Orchestrator)
        .unwrap();
    profile.provider = crate::config::ProviderKind::Codex;
    profile.binary = binary.into();
    profile.args = vec!["--offline-cli".into()];
    profile
}

fn execute(
    core: &Arc<Core>,
    conversation: &str,
    task: &Task,
    profile: ResolvedProfile,
    verifier: bool,
) -> Option<String> {
    core.turn(
        TurnSetup {
            session_policy: SessionPolicy::TaskRound {
                task: task.id.clone(),
                round: task.round,
            },
            conversation,
            plan_id: Some(task.plan_id.clone()),
            role: format!(
                "{}:{}",
                if verifier { "verifier" } else { "worker" },
                task.id
            ),
            cwd: PathBuf::from(core.store.conversation(conversation).unwrap().project),
            add_dirs: vec![],
            access: agent::Access::ReadOnly,
            profile: Ok(profile),
            upto_message: 0,
        },
        &AtomicBool::new(false),
        |_| "Perform the task using the round handoff.".into(),
    )
    .unwrap()
}

fn turn_prompt(core: &Core, conversation: &str, turn: &str) -> String {
    core.store
        .events_since(conversation, 0, 10000)
        .unwrap()
        .into_iter()
        .find(|event| {
            event.turn_id.as_deref() == Some(turn)
                && event.kind == "agent"
                && event.payload["type"] == "prompt"
        })
        .unwrap()
        .payload["text"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn editing_same_named_profile_rotates_cli_and_supplies_full_context_after_restart() {
    let (dir, core, conversation, mut task) = fixture();
    let binary = fake_cli(dir.path());
    task.round = 1;
    core.store.update_task(&task).unwrap();
    let profile = fake_profile(&core, &binary);
    assert_eq!(
        execute(&core, &conversation, &task, profile.clone(), false).as_deref(),
        Some("complete report")
    );
    let role = format!("worker:{}", task.id);
    let first = core.store.last_turn(&conversation, &role).unwrap().unwrap();
    let mut changed = profile.clone();
    changed.model = Some("changed-model-under-the-same-name".into());
    assert_eq!(
        execute(&core, &conversation, &task, changed.clone(), false).as_deref(),
        Some("complete report")
    );
    let second = core.store.last_turn(&conversation, &role).unwrap().unwrap();
    assert_ne!(first.session_id, second.session_id);
    let prompt = turn_prompt(&core, &conversation, &second.id);
    assert!(prompt.contains("Latest worker report (source round 1):\ncomplete report"));
    assert!(prompt.contains("Check everything"));
    assert_eq!(
        execute(&core, &conversation, &task, changed.clone(), false).as_deref(),
        Some("complete report")
    );
    assert_eq!(
        core.store
            .last_turn(&conversation, &role)
            .unwrap()
            .unwrap()
            .session_id,
        second.session_id
    );
    let state = core.state.clone();
    let config = (*core.config_snapshot()).clone();
    drop(core);
    let core = Core::open(&state, config).unwrap();
    assert_eq!(
        execute(&core, &conversation, &task, changed.clone(), false).as_deref(),
        Some("complete report")
    );
    assert_eq!(
        core.store
            .last_turn(&conversation, &role)
            .unwrap()
            .unwrap()
            .session_id,
        second.session_id
    );
    changed.native_effort = Some("low".into());
    assert_eq!(
        execute(&core, &conversation, &task, changed, false).as_deref(),
        Some("complete report")
    );
    assert_ne!(
        core.store
            .last_turn(&conversation, &role)
            .unwrap()
            .unwrap()
            .session_id,
        second.session_id
    );
}

#[test]
fn provider_context_rejection_preserves_full_report_and_raw_reason() {
    let (dir, core, conversation, mut task) = fixture();
    let report = "COMPLETE-LONG-REPORT\n".repeat(4000);
    task.round = 1;
    task.report = Some(report.clone());
    core.store.update_task(&task).unwrap();
    task.round = 2;
    task.report = None;
    core.store.update_task(&task).unwrap();
    let mut profile = fake_profile(&core, &fake_cli(dir.path()));
    profile.args.push("--reject-context".into());
    assert!(execute(&core, &conversation, &task, profile, false).is_none());
    let turn = core
        .store
        .last_turn(&conversation, &format!("worker:{}", task.id))
        .unwrap()
        .unwrap();
    assert!(
        turn.error
            .as_deref()
            .unwrap()
            .contains("context_length_exceeded: full handoff rejected by offline provider")
    );
    assert!(turn_prompt(&core, &conversation, &turn.id).contains(&report));
    assert_eq!(
        core.store.task_round(&task.id, 1).unwrap()["report"],
        report
    );
    assert_eq!(
        core.round_handoff(&task.id, 3, dir.path()).unwrap().report,
        Some((1, report))
    );
}

#[test]
fn restart_before_report_or_verdict_keeps_handoff_and_next_round_sessions_fresh() {
    for after_report in [false, true] {
        let (dir, core, conversation, mut task) = fixture();
        let profile = fake_profile(&core, &fake_cli(dir.path()));
        task.round = 1;
        task.report = Some("earlier complete report".into());
        core.store.update_task(&task).unwrap();
        task.round = 2;
        task.report = after_report.then(|| "completed report awaiting verdict".into());
        core.store.update_task(&task).unwrap();
        let role = format!(
            "{}:{}",
            if after_report { "verifier" } else { "worker" },
            task.id
        );
        let turn = core
            .store
            .start_turn(&conversation, &role, "codex", 0, Some(&task.plan_id))
            .unwrap();
        core.store
            .set_session(&turn.id, "interrupted-round-two-session")
            .unwrap();
        core.store
            .append_event(
                &conversation,
                Some(&turn.id),
                Some(&task.plan_id),
                "turn_started",
                &json!({"execution_signature":execution_signature(&profile).unwrap()}),
            )
            .unwrap();
        let state = core.state.clone();
        let config = (*core.config_snapshot()).clone();
        drop(core);
        let core = Core::open(&state, config).unwrap();
        assert_eq!(
            core.store
                .last_turn(&conversation, &role)
                .unwrap()
                .unwrap()
                .status,
            TurnStatus::Interrupted
        );
        let handoff = core.round_handoff(&task.id, 3, dir.path()).unwrap();
        let expected = if after_report {
            "completed report awaiting verdict"
        } else {
            "earlier complete report"
        };
        assert_eq!(handoff.report.as_ref().unwrap().1, expected);
        if !after_report {
            assert!(handoff.interrupted)
        }
        let context = tasks::round_handoff_prompt(&handoff, false);
        assert!(context.contains("interrupted"));
        assert!(context.contains(&role));
        task.round = 3;
        task.report = None;
        core.store.update_task(&task).unwrap();
        execute(&core, &conversation, &task, profile.clone(), false).unwrap();
        execute(&core, &conversation, &task, profile, true).unwrap();
        for prefix in ["worker", "verifier"] {
            let latest = core
                .store
                .last_turn(&conversation, &format!("{prefix}:{}", task.id))
                .unwrap()
                .unwrap();
            assert_ne!(
                latest.session_id.as_deref(),
                Some("interrupted-round-two-session")
            );
            let prompt = turn_prompt(&core, &conversation, &latest.id);
            assert!(prompt.contains("Round: 3"));
            assert!(prompt.contains("Check everything"));
            if prefix == "worker" {
                assert!(prompt.contains(expected))
            }
        }
    }
}
#[test]
fn protection_data_is_formatted_for_the_actual_workspace_mode() {
    let (dir, core, _conversation, mut task) = fixture();
    let git = |args: &[&str]| {
        let output = std::process::Command::new("git")
            .current_dir(dir.path())
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    };
    git(&["init"]);
    git(&["config", "user.name", "Test"]);
    git(&["config", "user.email", "test@example.com"]);
    std::fs::write(dir.path().join(".gitignore"), ".antislop/\n").unwrap();
    std::fs::write(dir.path().join("user.txt"), "original").unwrap();
    git(&["add", ".gitignore", "user.txt"]);
    git(&["commit", "-m", "base"]);
    std::fs::write(dir.path().join("user.txt"), "user draft").unwrap();
    task.spec["writes"] = true.into();
    core.store
        .db()
        .execute(
            "UPDATE tasks SET spec=?2 WHERE id=?1",
            params![task.id, task.spec.to_string()],
        )
        .unwrap();
    task.round = 1;
    task.base = Some(git::snapshot(dir.path(), "user baseline").unwrap());
    core.store.update_task(&task).unwrap();
    let handoff = core.round_handoff(&task.id, 2, dir.path()).unwrap();
    assert_eq!(handoff.protected_paths, ["user.txt"]);
    let prompt = tasks::round_handoff_prompt(&handoff, false);
    assert!(prompt.contains("Changes already present in the primary working tree"));
    assert!(!prompt.contains("You work directly in the user's directory"));
    let mut config = (*core.config_snapshot()).clone();
    config.workspace.mode = WorkspaceMode::InPlace;
    *core.config.write().unwrap() = Arc::new(config);
    let handoff = core.round_handoff(&task.id, 2, dir.path()).unwrap();
    assert_eq!(handoff.protected_paths, ["user.txt"]);
    let prompt = tasks::round_handoff_prompt(&handoff, false);
    assert!(prompt.contains("You work directly in the user's directory"));
    assert!(prompt.contains("do not modify them"));
}
#[test]
fn handoff_reads_only_requested_dependency_reports_and_no_dependency_verdicts() {
    let (_dir, core, conversation, mut task) = fixture();
    let extra_specs: Vec<_>=["dep","unrelated"].into_iter().map(|id|json!({
        "id":id,"title":id,"goal":"Provide evidence","kind":"analysis","verifier":{"focus":"Check evidence"}
    })).collect();
    let mut others = core.store.add_plan(&conversation, &extra_specs).unwrap();
    let full_report = "FULL DEPENDENCY REPORT\n".repeat(2000);
    for (index, other) in others.iter_mut().enumerate() {
        core.store
            .db()
            .execute(
                "UPDATE tasks SET plan_id=?2,position=?3 WHERE id=?1",
                params![other.id, task.plan_id, 100 + index as i64],
            )
            .unwrap();
        other.plan_id = task.plan_id.clone();
        other.round = 1;
        other.status = status::APPROVED.into();
        other.report = Some(if index == 0 {
            full_report.clone()
        } else {
            "UNRELATED REPORT".into()
        });
        other.verdict = Some(json!({"approved":true,"findings":[]}));
        core.store.update_task(other).unwrap();
    }
    let content_path = |id: &str, field: &str| {
        let encoded: String = core
            .store
            .db()
            .query_row(
                &format!("SELECT {field} FROM tasks WHERE id=?1"),
                [id],
                |row| row.get(0),
            )
            .unwrap();
        let reference: serde_json::Value =
            serde_json::from_str(encoded.strip_prefix("\0antislop-content:").unwrap()).unwrap();
        core.state
            .join("session-content")
            .join(reference["id"].as_str().unwrap())
    };
    std::fs::remove_file(content_path(&others[1].id, "report")).unwrap();
    std::fs::remove_file(content_path(&others[0].id, "verdict")).unwrap();
    task.spec["depends_on"] = json!(["dep"]);
    core.store
        .db()
        .execute(
            "UPDATE tasks SET spec=?2 WHERE id=?1",
            params![task.id, task.spec.to_string()],
        )
        .unwrap();
    assert_eq!(core.store.plan_specs(&task.plan_id).unwrap().len(), 3);
    let handoff = core
        .round_handoff(&task.id, 1, core.state.parent().unwrap())
        .unwrap();
    assert_eq!(handoff.dependencies.len(), 1);
    assert_eq!(handoff.dependencies[0].report, full_report);
    let prompt = tasks::round_handoff_prompt(&handoff, false);
    assert!(prompt.contains(&full_report));
    assert!(!prompt.contains("UNRELATED REPORT"));
    std::fs::remove_file(content_path(&others[0].id, "report")).unwrap();
    let error = core
        .round_handoff(&task.id, 1, core.state.parent().unwrap())
        .unwrap_err()
        .to_string();
    assert!(error.contains("No such file"), "{error}");
}
#[test]
fn same_round_cli_turns_keep_known_identity_when_resumed_output_omits_it() {
    let (dir, core, conversation, mut task) = fixture();
    task.round = 1;
    core.store.update_task(&task).unwrap();
    let mut profile = fake_profile(&core, &fake_cli(dir.path()));
    profile.args.push("--omit-resumed-session-id".into());
    for verifier in [false, true] {
        let role = format!(
            "{}:{}",
            if verifier { "verifier" } else { "worker" },
            task.id
        );
        execute(&core, &conversation, &task, profile.clone(), verifier).unwrap();
        let first = core.store.last_turn(&conversation, &role).unwrap().unwrap();
        assert!(first.session_id.is_some());
        for _ in 0..2 {
            execute(&core, &conversation, &task, profile.clone(), verifier).unwrap();
            let next = core.store.last_turn(&conversation, &role).unwrap().unwrap();
            assert_eq!(next.session_id, first.session_id);
            assert!(!turn_prompt(&core, &conversation, &next.id).contains("Round handoff"));
        }
    }
}
