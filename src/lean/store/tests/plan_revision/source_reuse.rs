use super::*;

#[test]
fn reuse_requires_a_revision_of_the_current_generation() {
    for kind in ["none", "code", "artifact"] {
        for basis in ["present", "missing", "older", "legacy"] {
            for changed in [false, true] {
                let (dir, store, conversation, scope, mut tasks) = fixture();
                let project = dir.path().join("project");
                std::fs::create_dir(&project).unwrap();
                assert!(
                    std::process::Command::new("git")
                        .args(["init", "-q"])
                        .current_dir(&project)
                        .status()
                        .unwrap()
                        .success()
                );
                std::fs::write(project.join("input.txt"), "original").unwrap();
                std::fs::write(project.join(".gitignore"), "output.txt\n").unwrap();
                std::fs::write(project.join("output.txt"), "artifact").unwrap();
                crate::lean::git::commit_all(
                    &project,
                    crate::lean::git::CommitPurpose::TestFixture,
                )
                .unwrap();
                store
                    .db()
                    .execute(
                        "UPDATE conversations SET project=?2 WHERE id=?1",
                        params![conversation, project.to_str().unwrap()],
                    )
                    .unwrap();
                let source = &mut tasks[0];
                source.spec["change_kind"] = kind.into();
                if kind == "artifact" {
                    source.spec["outputs"] = json!(["output.txt"]);
                }
                store
                    .db()
                    .execute(
                        "UPDATE tasks SET spec=?2 WHERE id=?1",
                        params![source.id, source.spec.to_string()],
                    )
                    .unwrap();
                store.queue_plan(&conversation, &source.plan_id).unwrap();
                if basis == "older" {
                    store
                        .db()
                        .execute(
                            "UPDATE task_inputs SET generation=1 WHERE task_id=?1",
                            [&source.id],
                        )
                        .unwrap();
                    source.input_generation = 1;
                }
                source.status = "approved".into();
                source.round = 1;
                source.report = Some("Retained result with durable evidence".into());
                source.feedback = Some(json!({"branch":"historical-branch"}));
                store.update_task(source).unwrap();
                store.test_completion(source, source.report.as_deref().unwrap());
                let output = store.completed_task_output(source).unwrap();
                let revision = if kind == "artifact" {
                    crate::lean::git::snapshot_with_artifacts(
                        &project,
                        &["output.txt".into()],
                        crate::lean::git::CommitPurpose::ArtifactInputSnapshot,
                    )
                    .unwrap()
                } else {
                    crate::lean::git::head(&project).unwrap()
                };
                if basis == "present" || basis == "older" {
                    store
                        .db()
                        .execute(
                            "INSERT INTO task_source_revisions VALUES(?1,?2,?3)",
                            params![source.id, 0, revision],
                        )
                        .unwrap();
                }
                if basis == "legacy" {
                    store
                        .db()
                        .execute_batch("DROP TABLE task_source_revisions")
                        .unwrap();
                }
                drop(store);
                let store = Store::open(&dir.path().join("core.db")).unwrap();
                if changed {
                    std::fs::write(project.join("input.txt"), "changed").unwrap();
                }
                let expected = if basis == "present" && !changed {
                    "approved"
                } else {
                    "queued"
                };
                let mut previous = source.clone();
                for _ in 0..2 {
                    let next = store
                        .add_plan(&conversation, std::slice::from_ref(&source.spec))
                        .unwrap()
                        .remove(0);
                    store
                        .bind_plan_scope(&conversation, &next.plan_id, &scope)
                        .unwrap();
                    store.queue_plan(&conversation, &next.plan_id).unwrap();
                    let reused = store.task_by_id(&next.id).unwrap().unwrap();
                    assert_eq!(reused.status, expected, "{kind}/{basis}/{changed}");
                    assert_eq!(reused.report, source.report);
                    assert_eq!(store.completed_task_output(&reused).unwrap(), output);
                    assert_eq!(reused.feedback.as_ref().unwrap()["carried_from"], source.id);
                    assert_eq!(
                        reused.feedback.as_ref().unwrap()["branch"],
                        "historical-branch"
                    );
                    let revision_count: i64 = store
                        .db()
                        .query_row(
                            "SELECT count(*) FROM task_source_revisions WHERE task_id=?1",
                            [&reused.id],
                            |r| r.get(0),
                        )
                        .unwrap();
                    assert_eq!(revision_count, i64::from(basis == "present"));
                    if expected == "queued" {
                        assert!(reused.verdict.is_none());
                        assert_eq!(reused.feedback.as_ref().unwrap()["resume_role"], "reviewer");
                        let event = store
                            .events_since(&conversation, 0, 1000)
                            .unwrap()
                            .into_iter()
                            .find(|e| {
                                e.kind == "task_reassessment_required"
                                    && e.payload["task"] == reused.id
                            })
                            .unwrap();
                        assert_eq!(
                            event.payload["source_comparison_unknown"],
                            basis != "present"
                        );
                        if basis != "present" {
                            assert!(event.payload["changed_path_count"].is_null());
                            assert!(event.payload["changed_paths"].is_null());
                        } else {
                            assert_eq!(event.payload["changed_paths"], json!(["input.txt"]));
                        }
                    }
                    assert_eq!(
                        store.task_by_id(&previous.id).unwrap().unwrap().report,
                        previous.report
                    );
                    previous = reused;
                }
            }
        }
    }
}

#[test]
fn explicit_keep_revalidates_the_approved_result_and_its_authored_dependencies() {
    for change in [
        "none",
        "contract",
        "generation",
        "round",
        "stale",
        "result",
        "dependency",
        "unkept-dependency",
    ] {
        let (dir, store, conversation, _, tasks) = fixture_for(true);
        store.queue_plan(&conversation, &tasks[0].plan_id).unwrap();
        let mut sources = store.plan(&tasks[0].plan_id).unwrap();
        for source in &mut sources {
            source.status = "approved".into();
            source.round = 1;
            source.report = Some(format!("Accepted {}", source.spec["id"]));
            source.verdict = Some(json!({"approved":true}));
            store.update_task(source).unwrap();
            store.test_completion(source, source.report.as_deref().unwrap());
        }
        let specs: Vec<_> = sources.iter().map(|t| t.spec.clone()).collect();
        let next = store.add_plan(&conversation, &specs).unwrap();
        let scope = store.create_scope(&conversation, "Expanded scope").unwrap();
        store
            .bind_plan_scope(&conversation, &next[0].plan_id, &scope.id)
            .unwrap();
        let keep = if change == "unkept-dependency" {
            vec!["b".into()]
        } else {
            vec!["a".into(), "b".into()]
        };
        store
            .retain_plan_tasks(&conversation, &next[0].plan_id, &keep)
            .unwrap();
        match change {
            "contract" | "dependency" => {
                let target = &next[usize::from(change == "contract")];
                let mut spec = target.spec.clone();
                spec["goal"] = "Different behavior".into();
                store
                    .db()
                    .execute(
                        "UPDATE tasks SET spec=?2 WHERE id=?1",
                        params![target.id, spec.to_string()],
                    )
                    .unwrap();
            }
            "generation" => {
                store
                    .db()
                    .execute(
                        "UPDATE task_inputs SET generation=generation+1 WHERE task_id=?1",
                        [&sources[0].id],
                    )
                    .unwrap();
            }
            "round" => {
                sources[0].round += 1;
                store.update_task(&sources[0]).unwrap();
            }
            "stale" => {
                sources[0].status = "stale".into();
                store.update_task(&sources[0]).unwrap();
            }
            "result" => store.test_completion(&sources[0], "Replacement result"),
            _ => {}
        }
        // Retention survives daemon restart, but never promotes an altered result.
        drop(store);
        let store = Store::open(&dir.path().join("core.db")).unwrap();
        let preview = store
            .retained_plan_preview(&conversation, &next[0].plan_id)
            .unwrap();
        let expected = if change == "none" {
            2
        } else if change == "contract" {
            1
        } else {
            0
        };
        assert_eq!(preview.as_array().unwrap().len(), expected, "{change}");
        store.queue_plan(&conversation, &next[0].plan_id).unwrap();
        let activated = store.plan(&next[0].plan_id).unwrap();
        assert_eq!(
            activated.iter().filter(|t| t.status == "approved").count(),
            expected,
            "{change}"
        );
        for task in activated.iter().filter(|t| t.status == "queued") {
            assert!(task.report.is_some(), "retained checkpoint: {change}");
            assert_eq!(
                task.feedback.as_ref().unwrap()["resume_role"],
                "reviewer",
                "{change}"
            );
        }
    }
}
