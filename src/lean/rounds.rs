//! Durable round boundaries and the explicit context carried across them.
use super::*;
use rusqlite::{OptionalExtension, params};

#[derive(Debug)]
pub(crate) struct RoundHandoff {
    pub task: serde_json::Value,
    pub round: u32,
    pub worktree: PathBuf,
    pub report: Option<(u32, String)>,
    pub verdict: Option<serde_json::Value>,
    pub files: Vec<String>,
    pub base: Option<String>,
    pub result: Option<String>,
    pub interrupted: bool,
    pub upstream: String,
    pub instructions: String,
}

impl Core {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn session_for_turn(
        &self,
        conversation: &str,
        role: &str,
        provider: &str,
        policy: &SessionPolicy,
        profile: Option<&ResolvedProfile>,
        cwd: &Path,
        add_dirs: &[PathBuf],
        access: agent::Access,
    ) -> Result<Option<String>> {
        let SessionPolicy::TaskRound { task, round } = policy else {
            return self.store.last_session(conversation, role, provider);
        };
        let candidate: Option<(String, Option<String>)> = self
            .store
            .db()
            .query_row(
                "SELECT t.id,t.session_id FROM turns t JOIN turn_scopes s ON s.turn_id=t.id
             WHERE t.conversation_id=?1 AND t.role=?2
             AND s.task_id=?3 AND s.round=?4
             ORDER BY t.started_ms DESC,t.rowid DESC LIMIT 1",
                params![conversation, role, task, round],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((turn, Some(session))) = candidate else {
            return Ok(None);
        };
        let Some(context) = self.store.run_context(conversation, &turn)? else {
            return Ok(None);
        };
        let access = match access {
            agent::Access::ReadOnly => store::RunAccess::ReadOnly,
            agent::Access::Write => store::RunAccess::Write,
        };
        Ok((context.provider == provider
            && profile.is_some_and(|p| p.profile == context.profile)
            && context.cwd == cwd
            && context.add_dirs == add_dirs
            && context.access == access)
            .then_some(session))
    }

    pub(super) fn round_handoff(&self, id: &str, round: u32, cwd: &Path) -> Result<RoundHandoff> {
        let task = self
            .store
            .task_by_id(id)?
            .ok_or_else(|| Error::Invalid(format!("task {id} does not exist")))?;
        let spec: tasks::TaskSpec = serde_json::from_value(task.spec.clone())?;
        let mut handoff = RoundHandoff {
            task: task.spec.clone(),
            round,
            worktree: cwd.into(),
            report: None,
            verdict: task.verdict.clone(),
            files: vec![],
            base: task.base.clone(),
            result: task.result.clone(),
            interrupted: false,
            upstream: self.upstream(&task.plan_id, &spec)?,
            instructions: String::new(),
        };
        let conversation = self.store.plan_conversation(&task.plan_id)?;
        if let Some(run) = self.store.plan_run(&task.plan_id)? {
            let entries = self.store.ledger_entries(&conversation, &run, None)?;
            let entries: Vec<_> = entries
                .into_iter()
                .filter(|entry| spec.requirement_ids.contains(&entry.spec.id))
                .collect();
            handoff.instructions.push_str(&format!(
                "\nRequirements and acceptance criteria:\n{}",
                serde_json::to_string(&entries)?
            ));
        }
        let comments: Vec<_> = self
            .store
            .comments(&conversation, "open")?
            .into_iter()
            .filter(|comment| {
                comment
                    .plan_id
                    .as_deref()
                    .is_none_or(|plan| plan == task.plan_id)
            })
            .collect();
        handoff.instructions.push_str(&format!(
            "\nCurrent open comments:\n{}",
            serde_json::to_string(&comments)?
        ));
        let mut files = std::collections::BTreeSet::new();
        for (source, record) in self.store.handoff_rounds(id, round)? {
            if handoff.result.is_none() {
                handoff.result = record["result"].as_str().map(str::to_owned);
            }
            if source < round {
                if let (Some(base), Some(result)) =
                    (record["base"].as_str(), record["result"].as_str())
                {
                    files.extend(git::changed_files(cwd, base, result)?);
                }
                if handoff.verdict.is_none() && !record["verdict"].is_null() {
                    handoff.verdict = Some(self.store.handoff_field(&record["verdict"], true)?);
                }
            }
            if handoff.report.is_none()
                && let Some(report) = self.store.completed_worker_report(id, source)?
            {
                handoff.report = Some((source, report));
            }
            let incomplete_worker =
                record["worker_started"] == true && record["worker_completed"] != true;
            if incomplete_worker && handoff.report.is_none() {
                handoff.interrupted = true;
            }
            if !incomplete_worker && handoff.report.is_none() && !record["report"].is_null() {
                let decoded = self.store.handoff_field(&record["report"], false)?;
                if let Some(report) = decoded.as_str() {
                    handoff.report = Some((source, report.to_owned()));
                }
            }
            if source < round && handoff.report.is_none() {
                handoff.interrupted = true;
            }
        }
        handoff.files = files.into_iter().collect();
        Ok(handoff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Arc<Core>, String, Task) {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
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
        let prompt = tasks::round_handoff_prompt(&handoff);
        assert!(prompt.contains(&"Latest complete report\n".repeat(2000)));
        assert!(prompt.contains("Keep all context"));
        assert!(prompt.contains("Check everything"));
        assert!(prompt.contains("file:12"));
        assert!(!prompt.contains("OLD REPORT MUST NOT APPEAR"));
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
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
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
        let profile = core
            .config_snapshot()
            .resolve(WorkKind::Orchestration, RoleKind::Orchestrator)
            .unwrap();
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
        let handoff = core.round_handoff(&task.id, 2, &cwd).unwrap();
        assert_eq!(
            handoff.report,
            Some((1, "Complete report before scheduler crash".into()))
        );
    }
}
