use super::*;

impl Core {
    pub(super) fn merge_session_with_main_tool(
        &self,
        conversation: &str,
        turn: &str,
    ) -> Result<Value> {
        let session = self.store.conversation(conversation)?;
        let completion: Option<String> = self.store.db().query_row(
            "SELECT payload FROM events WHERE conversation_id=?1 AND kind='session_completed' AND json_extract(payload,'$.cleanup_complete')=1 ORDER BY seq DESC LIMIT 1",
            [conversation],
            |row| row.get(0),
        ).optional()?;
        let branch = completion
            .as_deref()
            .and_then(|payload| serde_json::from_str::<Value>(payload).ok())
            .and_then(|payload| payload["branch"].as_str().map(str::to_owned));

        let result = match (completion, branch) {
            (None, _) => crate::lean::daemon::SessionActionResult::MissingInputs {
                inputs: vec!["completed session with preserved branch".into()],
            },
            (_, None) => crate::lean::daemon::SessionActionResult::MissingInputs {
                inputs: vec!["preserved session branch recorded at completion".into()],
            },
            (_, Some(branch)) if !branch.starts_with("antislop/session/") => {
                crate::lean::daemon::SessionActionResult::Failed {
                    details: format!(
                        "completed session records unexpected source branch {branch:?}; refusing to merge it into main"
                    ),
                }
            }
            (_, Some(branch)) => {
                let _integration = self.locks.integration();
                let outcome = crate::lean::git::merge_session_into_main(
                    std::path::Path::new(&session.project),
                    &branch,
                );
                match outcome {
                    Ok(crate::lean::git::SessionMergeOutcome::Succeeded { commit }) => {
                        crate::lean::daemon::SessionActionResult::Succeeded {
                            summary: format!("Merged {branch} into main at commit {commit}."),
                        }
                    }
                    Ok(crate::lean::git::SessionMergeOutcome::Failed { details }) => {
                        crate::lean::daemon::SessionActionResult::Failed { details }
                    }
                    Ok(crate::lean::git::SessionMergeOutcome::MissingInputs { inputs }) => {
                        crate::lean::daemon::SessionActionResult::MissingInputs { inputs }
                    }
                    Err(error) => crate::lean::daemon::SessionActionResult::Failed {
                        details: error.to_string(),
                    },
                }
            }
        };

        let payload = json!({
            "action": crate::lean::daemon::SessionActionKind::MergeWithMain,
            "result": result,
        });
        self.store.append_event(
            conversation,
            Some(turn),
            None,
            "session_action_result",
            &payload,
        )?;
        Ok(payload["result"].clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use std::process::Command;

    fn git(path: &std::path::Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn orchestrator_merge_tool_records_confirmed_result_in_same_conversation() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        let state = directory.path().join("state");
        std::fs::create_dir_all(&project).unwrap();
        git(&project, &["init", "--initial-branch=main"]);
        git(&project, &["config", "user.name", "Test User"]);
        git(&project, &["config", "user.email", "test@example.test"]);
        std::fs::write(project.join("base.txt"), "base\n").unwrap();
        git(&project, &["add", "base.txt"]);
        git(&project, &["commit", "-m", "base"]);
        git(&project, &["branch", "antislop/session/tool-test"]);
        git(&project, &["checkout", "antislop/session/tool-test"]);
        std::fs::write(project.join("session.txt"), "result\n").unwrap();
        git(&project, &["add", "session.txt"]);
        git(&project, &["commit", "-m", "session result"]);
        git(&project, &["checkout", "main"]);

        let config = Config::from_toml(include_str!("../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&state, config).unwrap();
        let conversation = core
            .store
            .create_conversation(project.to_str().unwrap())
            .unwrap();
        core.store
            .append_event(
                &conversation.id,
                None,
                None,
                "session_completed",
                &json!({"cleanup_complete":true,"branch":"antislop/session/tool-test"}),
            )
            .unwrap();
        let turn = core
            .store
            .start_turn(&conversation.id, ORCHESTRATOR, "codex", 1, None)
            .unwrap();

        let result = core
            .agent_tool(
                &conversation.id,
                ORCHESTRATOR,
                &turn.id,
                "merge_session_with_main",
                &json!({}),
            )
            .unwrap();
        assert_eq!(result["status"], "succeeded");
        assert!(result["summary"].as_str().unwrap().contains("into main"));
        assert_eq!(core.store.conversations().unwrap().len(), 1);
        let event: String = core.store.db().query_row(
            "SELECT payload FROM events WHERE conversation_id=?1 AND kind='session_action_result' ORDER BY seq DESC LIMIT 1",
            [&conversation.id],
            |row| row.get(0),
        ).unwrap();
        let event: Value = serde_json::from_str(&event).unwrap();
        assert_eq!(event["action"], "merge_with_main");
        assert_eq!(event["result"], result);
        assert_eq!(
            String::from_utf8_lossy(
                &Command::new("git")
                    .arg("-C")
                    .arg(&project)
                    .args(["show", "HEAD:session.txt"])
                    .output()
                    .unwrap()
                    .stdout
            ),
            "result\n"
        );
    }

    #[test]
    fn orchestrator_merge_tool_persists_raw_conflict_as_failure() {
        let directory = tempfile::tempdir().unwrap();
        let project = directory.path().join("project");
        let state = directory.path().join("state");
        std::fs::create_dir_all(&project).unwrap();
        git(&project, &["init", "--initial-branch=main"]);
        git(&project, &["config", "user.name", "Test User"]);
        git(&project, &["config", "user.email", "test@example.test"]);
        std::fs::write(project.join("file.txt"), "base\n").unwrap();
        git(&project, &["add", "file.txt"]);
        git(&project, &["commit", "-m", "base"]);
        git(&project, &["branch", "antislop/session/conflict"]);
        git(&project, &["checkout", "antislop/session/conflict"]);
        std::fs::write(project.join("file.txt"), "session edit\n").unwrap();
        git(&project, &["add", "file.txt"]);
        git(&project, &["commit", "-m", "session edit"]);
        git(&project, &["checkout", "main"]);
        std::fs::write(project.join("file.txt"), "main edit\n").unwrap();
        git(&project, &["add", "file.txt"]);
        git(&project, &["commit", "-m", "main edit"]);

        let config = Config::from_toml(include_str!("../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&state, config).unwrap();
        let conversation = core
            .store
            .create_conversation(project.to_str().unwrap())
            .unwrap();
        core.store
            .append_event(
                &conversation.id,
                None,
                None,
                "session_completed",
                &json!({"cleanup_complete":true,"branch":"antislop/session/conflict"}),
            )
            .unwrap();
        let turn = core
            .store
            .start_turn(&conversation.id, ORCHESTRATOR, "codex", 1, None)
            .unwrap();

        let result = core
            .agent_tool(
                &conversation.id,
                ORCHESTRATOR,
                &turn.id,
                "merge_session_with_main",
                &json!({}),
            )
            .unwrap();
        assert_eq!(result["status"], "failed");
        let details = result["details"].as_str().unwrap();
        assert!(
            details.contains("CONFLICT") || details.contains("conflict"),
            "{details}"
        );
        assert_eq!(
            std::fs::read_to_string(project.join("file.txt")).unwrap(),
            "main edit\n"
        );
        let recorded: String = core.store.db().query_row(
            "SELECT payload FROM events WHERE conversation_id=?1 AND kind='session_action_result' ORDER BY seq DESC LIMIT 1",
            [&conversation.id],
            |row| row.get(0),
        ).unwrap();
        let recorded: Value = serde_json::from_str(&recorded).unwrap();
        assert_eq!(recorded["result"], result);
    }
}
