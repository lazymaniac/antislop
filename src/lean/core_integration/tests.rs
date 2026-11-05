use super::*;

#[test]
fn untracked_report_overrides_local_commits_and_conflicts_keep_the_worktree() {
    for output in ["report.md", ".antislop/continuation-plan-review.md"] {
        for conflict in [false, true] {
            let directory = tempfile::tempdir().unwrap();
            let project = directory.path().join("project");
            std::fs::create_dir(&project).unwrap();
            assert!(
                std::process::Command::new("git")
                    .args(["init", "-q"])
                    .current_dir(&project)
                    .status()
                    .unwrap()
                    .success()
            );
            std::fs::write(project.join("source.txt"), "user source").unwrap();
            std::fs::write(project.join(".gitignore"), ".antislop/\n").unwrap();
            let base = git::commit_all(&project, "base").unwrap();
            let mut config =
                Config::from_toml(include_str!("../../../examples/antislop.toml")).unwrap();

            config.policy.local_commits = true;
            let core = Core::open(&directory.path().join("state"), config.clone()).unwrap();
            let conversation = core
                .store
                .create_conversation(project.to_str().unwrap())
                .unwrap()
                .id;
            let text = "Write analysis report";
            let message = core.store.add_message(&conversation, "user", text).unwrap();
            let run = core.store.create_run(&conversation, text).unwrap();
            let source = core
                .store
                .register_message_source(&conversation, &run.id, message.seq)
                .unwrap();
            let entry = serde_json::from_value(json!({"id":"R1","source":{"source_id":source.id,"start":0,"end":text.len()},"interpretation":text,"criteria":[{"id":"R1.1","text":"Report delivered"}]})).unwrap();
            core.store
                .apply_ledger_change(
                    &ledger::LedgerChange {
                        conversation: conversation.clone(),
                        run_id: run.id.clone(),
                        expected_spec_revision: 0,
                        operation_id: crate::id(),
                        reason: "Source requirement".into(),
                        entries: vec![entry],
                    },
                    &core.state,
                    true,
                )
                .unwrap();
            let spec: TaskContract=serde_json::from_value(json!({"id":"report","title":"Report","goal":"Write analysis","kind":"analysis","change_kind":"artifact","outputs":[output],"delivery":"untracked","requirement_ids":["R1"],"criterion_ids":["R1.1"]})).unwrap();
            let mut task = core
                .store
                .add_plan(&conversation, &[serde_json::to_value(&spec).unwrap()])
                .unwrap()
                .remove(0);
            core.store
                .link_plan_run(&conversation, &task.plan_id, &run.id)
                .unwrap();
            core.store
                .bind_task_ledger(&conversation, &run.id, &task.plan_id)
                .unwrap();
            let worktree =
                crate::lean::core_path_helpers::worktree_path(&core.state, &task).unwrap();
            let branch =
                crate::lean::core_path_helpers::worktree_branch(&core.state, &task).unwrap();
            git::add_worktree(&project, &worktree, &branch, &base).unwrap();
            std::fs::create_dir_all(worktree.join(output).parent().unwrap()).unwrap();
            std::fs::write(worktree.join(output), "complete report").unwrap();
            task.base = Some(base.clone());
            task.result = Some(
                git::commit_artifacts(&worktree, &spec.outputs, "report checkpoint", true).unwrap(),
            );
            core.store.update_task(&task).unwrap();
            if conflict {
                std::fs::create_dir_all(project.join(output).parent().unwrap()).unwrap();
                std::fs::write(project.join(output), "existing user report").unwrap();
            }
            let index_path = project.join(".git/index");
            let index = std::fs::read(&index_path).unwrap();
            core.integrate(
                &conversation,
                &mut task,
                &spec,
                &project,
                &worktree,
                &config,
                &AtomicBool::new(false),
            )
            .unwrap();
            assert_eq!(git::head(&project).unwrap(), base);
            assert_eq!(std::fs::read(&index_path).unwrap(), index);
            if conflict {
                assert_ne!(task.status, status::APPROVED);
                assert!(worktree.join(output).exists());
                assert_eq!(
                    std::fs::read_to_string(project.join(output)).unwrap(),
                    "existing user report"
                );
            } else {
                assert_eq!(task.status, status::APPROVED);
                assert_eq!(
                    std::fs::read_to_string(project.join(output)).unwrap(),
                    "complete report"
                );
                let output = std::process::Command::new("git")
                    .args(["ls-files", "--", output])
                    .current_dir(&project)
                    .output()
                    .unwrap();
                assert!(output.stdout.is_empty());
            }
        }
    }
}
