use super::*;
use crate::lean::git::{self, test_observation};
use crate::lean::store::sessions_and_plans::carry::measurements;
use std::sync::Arc;

fn prepared_fixture(
    artifact: bool,
) -> (
    tempfile::TempDir,
    Arc<Store>,
    String,
    String,
    Vec<Task>,
    Vec<Task>,
) {
    let (dir, store, conversation, scope, mut sources) = fixture_for(true);
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    git::ok(&project, &["init", "-q"]).unwrap();
    std::fs::write(project.join("input"), "before").unwrap();
    std::fs::write(project.join(".gitignore"), "report.md\nother.md\n").unwrap();
    std::fs::write(project.join("report.md"), "before").unwrap();
    std::fs::write(project.join("other.md"), "before").unwrap();
    git::commit_all(&project, git::CommitPurpose::TestFixture).unwrap();
    store
        .db()
        .execute(
            "UPDATE conversations SET project=?2 WHERE id=?1",
            params![conversation, project.to_str().unwrap()],
        )
        .unwrap();
    if artifact {
        for source in &mut sources {
            source.spec["change_kind"] = "artifact".into();
            source.spec["outputs"] = json!(["report.md"]);
            store
                .db()
                .execute(
                    "UPDATE tasks SET spec=?2 WHERE id=?1",
                    params![source.id, source.spec.to_string()],
                )
                .unwrap();
        }
    }
    store
        .queue_plan(&conversation, &sources[0].plan_id)
        .unwrap();
    let outputs = if artifact {
        vec!["report.md".into()]
    } else {
        vec![]
    };
    let revision =
        git::snapshot_with_artifacts(&project, &outputs, git::CommitPurpose::TestFixture).unwrap();
    for source in &mut sources {
        source.status = "approved".into();
        source.round = 1;
        source.report = Some("historical evidence".into());
        source.feedback = Some(json!({"kept":"feedback"}));
        store.update_task(source).unwrap();
        store.test_completion(source, "historical evidence");
        store
            .db()
            .execute(
                "INSERT INTO task_source_revisions VALUES(?1,0,?2)",
                params![source.id, revision],
            )
            .unwrap();
    }
    let targets = store
        .add_plan(
            &conversation,
            &sources.iter().map(|t| t.spec.clone()).collect::<Vec<_>>(),
        )
        .unwrap();
    store
        .bind_plan_scope(&conversation, &targets[0].plan_id, &scope)
        .unwrap();
    (dir, Arc::new(store), conversation, scope, sources, targets)
}

fn assert_unpublished(store: &Store, plan: &str) {
    let db = store.db();
    for sql in [
        "SELECT count(*) FROM tasks WHERE plan_id=?1 AND status<>'proposed'",
        "SELECT count(*) FROM plan_activations WHERE plan_id=?1",
        "SELECT count(*) FROM events WHERE plan_id=?1 AND kind IN ('notice','task_reassessment_required','workflow')",
        "SELECT count(*) FROM task_resume_results WHERE task_id IN (SELECT id FROM tasks WHERE plan_id=?1)",
        "SELECT count(*) FROM task_result_sources WHERE task_id IN (SELECT id FROM tasks WHERE plan_id=?1)",
        "SELECT count(*) FROM task_source_revisions WHERE task_id IN (SELECT id FROM tasks WHERE plan_id=?1)",
    ] {
        assert_eq!(
            db.query_row(sql, [plan], |r| r.get::<_, i64>(0)).unwrap(),
            0,
            "{sql}"
        );
    }
}

#[test]
fn carry_revalidates_concurrent_inputs_without_holding_store() {
    for mutation in [
        "generation",
        "target_generation",
        "scope_binding",
        "scope_source",
        "contract",
        "checkpoint",
        "producer",
        "scope",
        "membership",
        "new_producer",
        "dependency",
        "result",
        "evidence",
        "workflow",
        "revision",
        "file",
        "artifact",
    ] {
        let (dir, store, conversation, scope, sources, targets) =
            prepared_fixture(mutation == "artifact");
        let plan = targets[0].plan_id.clone();
        let worker_store = store.clone();
        let worker_plan = plan.clone();
        let worker_conversation = conversation.clone();
        let project = dir.path().join("project");
        let source_id = sources[0].id.clone();
        let target_id = targets[0].id.clone();
        let spec = sources[0].spec.clone();
        test_observation::on_next_diff(move || {
            std::thread::spawn(move || {
                // This runs while queue_plan is suspended inside its actual Git
                // read path. A held Store mutex fails immediately, deterministically.
                let db = worker_store.connection.try_lock().expect("Store must be available during Git preparation");
                match mutation {
                    "generation" => { db.execute("UPDATE task_inputs SET generation=generation+1 WHERE task_id=?1", [&source_id]).unwrap(); }
                    "target_generation" => { db.execute("UPDATE task_inputs SET generation=generation+1 WHERE task_id=?1", [&target_id]).unwrap(); }
                    "scope_binding" => { db.execute("INSERT INTO plan_scopes SELECT plan_id,revision+1,scope_id FROM plan_scopes WHERE plan_id=?1 ORDER BY revision DESC LIMIT 1", [&worker_plan]).unwrap(); }
                    "scope_source" => {
                        db.execute("INSERT INTO conversation_sources VALUES('concurrent-source',?1,'reference','hash',0)", [&worker_conversation]).unwrap();
                        db.execute("INSERT INTO scope_documents VALUES(?1,'concurrent-source')", [&scope]).unwrap();
                    }
                    "contract" => { db.execute("UPDATE tasks SET spec=json_set(spec,'$.goal','concurrently changed') WHERE id=?1", [&target_id]).unwrap(); }
                    "checkpoint" => { db.execute("UPDATE tasks SET report='new checkpoint',round=round+1 WHERE id=?1", [&source_id]).unwrap(); }
                    "producer" => { db.execute("INSERT INTO task_result_sources VALUES(?1,0,?2)", params![source_id,target_id]).unwrap(); }
                    "scope" => { db.execute("UPDATE scopes SET summary='changed scope' WHERE id=?1", [&scope]).unwrap(); }
                    "membership" => { db.execute("UPDATE tasks SET position=position+1 WHERE id=?1", [&target_id]).unwrap(); }
                    "new_producer" => { db.execute("INSERT INTO tasks(id,conversation_id,plan_id,position,spec,status,report,created_ms) SELECT 'new-producer',conversation_id,plan_id,99,spec,'approved','newest',created_ms+1 FROM tasks WHERE id=?1", [&source_id]).unwrap(); }
                    "dependency" => { db.execute("UPDATE tasks SET feedback=json_object('carried_from',?2) WHERE id=?1", params![source_id,target_id]).unwrap(); }
                    "result" => { db.execute("UPDATE agent_results SET result=json_set(result,'$.report','changed result') WHERE turn_id=(SELECT turn_id FROM turn_scopes WHERE task_id=?1 LIMIT 1)", [&source_id]).unwrap(); }
                    "evidence" => {
                        let turn: String = db.query_row("SELECT turn_id FROM turn_scopes WHERE task_id=?1 LIMIT 1", [&source_id], |r| r.get(0)).unwrap();
                        db.execute("INSERT INTO agent_evidence VALUES('concurrent-evidence',?1,'new',1,'sha')", [&turn]).unwrap();
                        db.execute("INSERT INTO result_evidence VALUES(?1,'concurrent-evidence')", [&turn]).unwrap();
                    }
                    "workflow" => { db.execute("UPDATE session_workflow SET summary='changed workflow' WHERE conversation_id=?1", [&worker_conversation]).unwrap(); }
                    "revision" => { db.execute("UPDATE task_source_revisions SET revision='missing-revision' WHERE task_id=?1", [&source_id]).unwrap(); }
                    "file" => std::fs::write(project.join("input"), "after").unwrap(),
                    "artifact" => std::fs::write(project.join("report.md"), "after").unwrap(),
                    _ => unreachable!(),
                }
                // Exercise a successful SQLite read while the carry is paused.
                let _: String = db.query_row("SELECT spec FROM tasks WHERE id=?1", [&target_id], |r| r.get(0)).unwrap();
                drop(db);
                assert_eq!(worker_store.plan(&worker_plan).unwrap().len(), 2);
                assert!(spec.is_object());
            }).join().unwrap();
        });
        let error = store
            .queue_plan(&conversation, &plan)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("changed during preparation"),
            "{mutation}: {error}"
        );
        assert_unpublished(&store, &plan);
        eprintln!(
            "carry mutation={mutation}: Store available, stale preparation rejected, no partial publication"
        );
    }
}

#[test]
fn carry_work_is_bounded_by_demand_and_reuses_git_comparisons() {
    for artifact in [false, true] {
        let mut previous_work = None;
        let mut previous_git = None;
        for history in [0, 100, 10000] {
            let (_dir, store, conversation, _, sources, targets) = prepared_fixture(artifact);
            {
                let mut db = store.db();
                let tx = db.transaction().unwrap();
                for n in 0..history {
                    // Older rows of the *same* demanded identity, plus unrelated ids.
                    // Equal timestamps exercise the rowid tie break and reverse index walk.
                    tx.execute("INSERT INTO tasks(id,conversation_id,plan_id,position,spec,status,feedback,created_ms) SELECT ?2,conversation_id,plan_id,99,CASE WHEN ?3%2=0 THEN spec ELSE json_set(spec,'$.id',?2) END,'approved','{\"history\":true}',created_ms-1 FROM tasks WHERE id=?1", params![sources[0].id,format!("history-{n}"),n]).unwrap();
                }
                tx.commit().unwrap();
            }
            measurements::take();
            test_observation::take();
            store
                .queue_plan(&conversation, &targets[0].plan_id)
                .unwrap();
            let work = measurements::take();
            let git = test_observation::take();
            eprintln!("carry artifact={artifact} history={history} work={work:?} git={git:?}");
            assert_eq!(work.spec_decodes, 8); // two targets + two demanded sources, twice
            assert_eq!(work.feedback_cells, 8);
            assert_eq!(work.producer_lookups, 4); // two producer seeks in each capture
            assert_eq!(work.dependency_lookups, 8); // old/new identity and origin, twice
            assert_eq!(work.comparisons, 1);
            assert_eq!(work.snapshots, 2); // preparation and independent revalidation
            assert_eq!(git.probes, 1);
            assert_eq!(git.resolutions, 2); // preparation and independent ref validation
            assert_eq!(git.diffs, 2); // one shared comparison and one source revalidation
            assert_eq!(work.fullscan_steps, 0);

            if let Some(before) = &mut previous_work {
                let before: &mut measurements::Work = before;
                assert!(
                    work.vm_steps <= before.vm_steps * 2,
                    "indexed VM work must not scale with historical payloads"
                );
                before.vm_steps = work.vm_steps;
                assert_eq!(before, &work);
            }
            if let Some(before) = &previous_git {
                assert_eq!(before, &git);
            }
            previous_work = Some(work);
            previous_git = Some(git);
            for target in &targets {
                let current = store.task_by_id(&target.id).unwrap().unwrap();
                assert_eq!(current.status, "approved");
                assert_eq!(current.report, sources[0].report);
                assert_eq!(
                    store.completed_task_output(&current).unwrap()["report"],
                    "historical evidence"
                );
            }
            let events = store.latest_event_seq(&conversation).unwrap();
            let error = store
                .queue_plan(&conversation, &targets[0].plan_id)
                .unwrap_err()
                .to_string();
            assert!(error.contains("not waiting for approval"));
            assert_eq!(store.latest_event_seq(&conversation).unwrap(), events);
        }
    }
}
#[test]
fn carry_rolls_back_raw_git_and_sqlite_failures() {
    for failure in [
        "git_revision",
        "git_snapshot",
        "git_diff",
        "sqlite_carry",
        "sqlite_activation",
    ] {
        let (dir, store, conversation, _, sources, targets) = prepared_fixture(false);
        if failure == "git_revision" {
            store
                .db()
                .execute(
                    "UPDATE task_source_revisions SET revision='missing-revision' WHERE task_id=?1",
                    [&sources[0].id],
                )
                .unwrap();
        } else if failure == "git_snapshot" {
            std::fs::write(dir.path().join("project/.git/index"), "broken index").unwrap();
        } else if failure == "git_diff" {
            let revision: String = store
                .db()
                .query_row(
                    "SELECT revision FROM task_source_revisions WHERE task_id=?1",
                    [&sources[0].id],
                    |r| r.get(0),
                )
                .unwrap();
            let object = dir
                .path()
                .join("project/.git/objects")
                .join(&revision[..2])
                .join(&revision[2..]);
            test_observation::on_next_diff(move || std::fs::remove_file(object).unwrap());
        } else if failure == "sqlite_carry" {
            store.db().execute_batch(&format!("CREATE TRIGGER fail_carry BEFORE INSERT ON task_resume_results WHEN NEW.task_id='{}' BEGIN SELECT RAISE(ABORT,'injected carry SQLite failure'); END", targets[1].id)).unwrap();
        } else {
            store.db().execute_batch("CREATE TRIGGER fail_activation BEFORE INSERT ON plan_activations BEGIN SELECT RAISE(ABORT,'injected activation SQLite failure'); END").unwrap();
        }
        let error = store
            .queue_plan(&conversation, &targets[0].plan_id)
            .unwrap_err()
            .to_string();
        let expected = match failure {
            "git_revision" => "git rev-parse",
            "git_snapshot" => "index file smaller than expected",
            "git_diff" => "git diff",
            "sqlite_carry" => "injected carry SQLite failure",
            "sqlite_activation" => "injected activation SQLite failure",
            _ => unreachable!(),
        };
        assert!(error.contains(expected), "{failure}: {error}");
        assert_unpublished(&store, &targets[0].plan_id);
        eprintln!("carry failure={failure}: {error}; all queue/carry effects rolled back");
    }
}

#[test]
fn carry_indexes_are_installed_on_reopen_without_rewriting_history() {
    let (dir, store, conversation, _, _, targets) = prepared_fixture(false);
    store.db().execute_batch("DROP INDEX tasks_carry_by_identity; DROP INDEX tasks_carry_latest; DROP INDEX tasks_plan_identity;").unwrap();
    store.db().execute("INSERT INTO tasks(id,conversation_id,plan_id,position,spec,status,created_ms) VALUES('malformed-history',?1,(SELECT plan_id FROM tasks WHERE conversation_id=?1 AND plan_id<>?2 LIMIT 1),99,'not-json','working',0)", params![conversation,targets[0].plan_id]).unwrap();
    let before: Vec<Task> = store.plan(&targets[0].plan_id).unwrap();
    drop(store);
    let store = Store::open(&dir.path().join("core.db")).unwrap();
    let indexes: i64 = store.db().query_row("SELECT count(*) FROM sqlite_master WHERE type='index' AND name IN ('tasks_carry_by_identity','tasks_carry_latest','tasks_plan_identity')", [], |r| r.get(0)).unwrap();
    assert_eq!(indexes, 3);
    assert_eq!(
        store
            .db()
            .query_row(
                "SELECT spec FROM tasks WHERE id='malformed-history'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "not-json"
    );
    assert_eq!(
        store
            .plan(&targets[0].plan_id)
            .unwrap()
            .iter()
            .map(|t| (&t.id, &t.spec, &t.report, &t.feedback))
            .collect::<Vec<_>>(),
        before
            .iter()
            .map(|t| (&t.id, &t.spec, &t.report, &t.feedback))
            .collect::<Vec<_>>()
    );
    store
        .queue_plan(&conversation, &targets[0].plan_id)
        .unwrap();
}

#[test]
fn carry_cache_distinguishes_artifact_sets_and_normalizes_identical_sets() {
    for identical in [false, true] {
        let (dir, store, conversation, _, sources, targets) = prepared_fixture(true);
        let project = dir.path().join("project");
        let common_revision = if identical {
            Some(
                git::snapshot_with_artifacts(
                    &project,
                    &["other.md".into(), "report.md".into()],
                    git::CommitPurpose::TestFixture,
                )
                .unwrap(),
            )
        } else {
            None
        };
        for (index, source) in sources.iter().enumerate() {
            let outputs = if identical {
                vec!["other.md".to_owned(), "report.md".to_owned()]
            } else {
                vec![if index == 0 { "report.md" } else { "other.md" }.to_owned()]
            };
            let revision = match &common_revision {
                Some(revision) => revision.clone(),
                None => git::snapshot_with_artifacts(
                    &project,
                    &outputs,
                    git::CommitPurpose::TestFixture,
                )
                .unwrap(),
            };
            let mut spec = source.spec.clone();
            spec["outputs"] = if identical && index == 1 {
                json!(["report.md", "other.md", "report.md"])
            } else {
                json!(outputs)
            };
            store
                .db()
                .execute(
                    "UPDATE tasks SET spec=?2 WHERE id=?1 OR id=?3",
                    params![source.id, spec.to_string(), targets[index].id],
                )
                .unwrap();
            store
                .db()
                .execute(
                    "UPDATE task_source_revisions SET revision=?2 WHERE task_id=?1",
                    params![source.id, revision],
                )
                .unwrap();
        }
        if !identical {
            std::fs::write(project.join("other.md"), "after").unwrap();
        }
        measurements::take();
        test_observation::take();
        store
            .queue_plan(&conversation, &targets[0].plan_id)
            .unwrap();
        let work = measurements::take();
        assert_eq!(work.snapshots, if identical { 2 } else { 4 });
        assert_eq!(work.comparisons, if identical { 1 } else { 2 });
        assert_eq!(
            store.task_by_id(&targets[0].id).unwrap().unwrap().status,
            "approved"
        );
        assert_eq!(
            store.task_by_id(&targets[1].id).unwrap().unwrap().status,
            if identical { "approved" } else { "queued" }
        );
        eprintln!(
            "carry artifact identical_sets={identical} work={work:?} git={:?}",
            test_observation::take()
        );
    }
}

#[test]
fn carry_keeps_newest_checkpoint_exact_generation_result_and_evidence() {
    for revision_present in [false, true] {
        let (_dir, store, conversation, scope, sources, _) = prepared_fixture(false);
        let mut checkpoint = store
            .add_plan(&conversation, &[sources[0].spec.clone()])
            .unwrap()
            .remove(0);
        checkpoint.spec["goal"] = "newer checkpoint contract".into();
        checkpoint.status = "failed".into();
        checkpoint.round = 3;
        checkpoint.input_generation = 7;
        checkpoint.report = Some("latest checkpoint report".into());
        checkpoint.base = Some("checkpoint base".into());
        checkpoint.result = Some("checkpoint result".into());
        checkpoint.feedback = Some(json!({"checkpoint":"newest"}));
        store
            .db()
            .execute(
                "UPDATE task_inputs SET generation=7 WHERE task_id=?1",
                [&checkpoint.id],
            )
            .unwrap();
        store.update_task(&checkpoint).unwrap();
        store
            .db()
            .execute(
                "UPDATE tasks SET spec=?2 WHERE id=?1",
                params![checkpoint.id, checkpoint.spec.to_string()],
            )
            .unwrap();
        store
            .bind_plan_scope(&conversation, &checkpoint.plan_id, &scope)
            .unwrap();
        store
        .db()
        .execute(
            "UPDATE tasks SET created_ms=(SELECT created_ms FROM tasks WHERE id=?2) WHERE id=?1",
            params![checkpoint.id, sources[0].id],
        )
        .unwrap();
        let worker = store
            .start_turn(
                &conversation,
                &format!("worker:{}", checkpoint.id),
                "fixture",
                0,
                Some(&checkpoint.plan_id),
            )
            .unwrap();
        let evidence = store
            .record_evidence(
                &worker.id,
                "checkpoint-check",
                "command: fixture; exit: 0; latest checkpoint",
            )
            .unwrap();
        let result = crate::lean::turn_result::TurnResult::Completed {
            summary: "latest checkpoint".into(),
            report: "latest checkpoint report".into(),
            commit_message: String::new(),
            selection: None,
        };
        store
            .save_turn_result_with_evidence(&worker.id, &result, std::slice::from_ref(&evidence))
            .unwrap();
        store
            .finish_turn(&worker.id, TurnStatus::Done, None)
            .unwrap();
        // A generation-zero revision must not prove generation seven unchanged.
        store
            .db()
            .execute(
                "INSERT INTO task_source_revisions VALUES(?1,0,'old-generation')",
                [&checkpoint.id],
            )
            .unwrap();
        let next = store
            .add_plan(&conversation, &[sources[0].spec.clone()])
            .unwrap()
            .remove(0);
        store
            .bind_plan_scope(&conversation, &next.plan_id, &scope)
            .unwrap();
        if revision_present {
            store.db().execute("INSERT INTO task_source_revisions SELECT ?1,7,revision FROM task_source_revisions WHERE task_id=?2 AND generation=0", params![checkpoint.id,sources[0].id]).unwrap();
        }
        store
            .db()
            .execute(
                "UPDATE task_inputs SET generation=5 WHERE task_id=?1",
                [&next.id],
            )
            .unwrap();
        store.queue_plan(&conversation, &next.plan_id).unwrap();
        let carried = store.task_by_id(&next.id).unwrap().unwrap();
        assert_eq!(carried.status, "queued");
        assert_eq!(carried.input_generation, 5);
        assert_eq!(carried.report, checkpoint.report);
        assert_eq!(carried.base, checkpoint.base);
        assert_eq!(carried.result, checkpoint.result);
        assert_eq!(carried.round, 3);
        assert_eq!(
            carried.feedback.as_ref().unwrap()["carried_from"],
            checkpoint.id
        );
        let (producer, output) = store.completed_task_result(&carried).unwrap();
        assert_eq!(producer, checkpoint.id);
        assert_eq!(
            store
                .db()
                .query_row(
                    "SELECT result_turn FROM task_resume_results WHERE task_id=?1 AND generation=5",
                    [&carried.id],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            worker.id
        );
        assert_eq!(output["report"], "latest checkpoint report");
        assert_eq!(
            store.result_evidence(&worker.id).unwrap()[0].id,
            evidence.id
        );
        assert_eq!(
            store.evidence_text(&evidence).unwrap(),
            "command: fixture; exit: 0; latest checkpoint"
        );
        assert_eq!(
            store
                .db()
                .query_row(
                    "SELECT count(*) FROM task_source_revisions WHERE task_id=?1",
                    [&carried.id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            i64::from(revision_present)
        );
        assert_eq!(
            store.task_by_id(&sources[0].id).unwrap().unwrap().report,
            sources[0].report
        );
        let reassessment = store
            .events_since(&conversation, 0, 1000)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == "task_reassessment_required" && e.payload["task"] == carried.id)
            .unwrap();
        assert_eq!(
            reassessment.payload["source_comparison_unknown"],
            !revision_present
        );
    }
}
#[test]
fn carry_producer_queries_use_ordered_expression_indexes() {
    let (_dir, store, conversation, _, _, targets) = prepared_fixture(false);
    let db = store.db();
    for (sql, expected) in [
        (
            "EXPLAIN QUERY PLAN SELECT id,spec,feedback FROM tasks WHERE conversation_id=?1 AND CASE WHEN json_valid(spec) THEN json_extract(spec,'$.id') END='a' AND plan_id<>?2 AND (status IN ('approved','stale') OR (status IN ('coordinating','failed','cancelled','needs_user','waiting','queued') AND (report IS NOT NULL OR result IS NOT NULL))) ORDER BY created_ms DESC,rowid DESC LIMIT 1",
            "tasks_carry_by_identity",
        ),
        (
            "EXPLAIN QUERY PLAN SELECT plan_id FROM tasks WHERE conversation_id=?1 AND plan_id<>?2 AND (status IN ('approved','stale') OR (status IN ('coordinating','failed','cancelled','needs_user','waiting','queued') AND (report IS NOT NULL OR result IS NOT NULL))) ORDER BY created_ms DESC,rowid DESC LIMIT 1",
            "tasks_carry_latest",
        ),
        (
            "EXPLAIN QUERY PLAN SELECT id FROM tasks WHERE plan_id=?2 AND CASE WHEN json_valid(spec) THEN json_extract(spec,'$.id') END='a' AND status<>'superseded' AND conversation_id=?1",
            "tasks_plan_identity",
        ),
    ] {
        let mut stmt = db.prepare(sql).unwrap();
        let details = stmt
            .query_map(params![conversation, targets[0].plan_id], |r| {
                r.get::<_, String>(3)
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .join("; ");
        assert!(details.contains(expected), "{details}");
        assert!(!details.contains("TEMP B-TREE"), "{details}");
        eprintln!("carry query plan: {details}");
    }
}

#[test]
fn carry_accepts_an_unrelated_concurrent_store_write() {
    let (_dir, store, conversation, _, _, targets) = prepared_fixture(false);
    let writer = store.clone();
    let writer_conversation = conversation.clone();
    test_observation::on_next_diff(move || {
        std::thread::spawn(move || {
            drop(
                writer
                    .connection
                    .try_lock()
                    .expect("Store must be available during Git preparation"),
            );
            writer
                .append_event(
                    &writer_conversation,
                    None,
                    None,
                    "unrelated_observation",
                    &json!({"preserve":true}),
                )
                .unwrap();
        })
        .join()
        .unwrap();
    });
    store
        .queue_plan(&conversation, &targets[0].plan_id)
        .unwrap();
    assert!(
        store
            .plan(&targets[0].plan_id)
            .unwrap()
            .iter()
            .all(|t| t.status == "approved")
    );
    assert!(
        store
            .events_since(&conversation, 0, 1000)
            .unwrap()
            .iter()
            .any(|e| e.kind == "unrelated_observation")
    );
}

#[test]
fn carry_revalidates_a_legacy_result_producers_actual_generation() {
    let (_dir, store, conversation, _, sources, targets) = prepared_fixture(false);
    let mut spec = sources[0].spec.clone();
    spec["id"] = "legacy-producer".into();
    let mut producer = store.add_plan(&conversation, &[spec]).unwrap().remove(0);
    producer.status = "approved".into();
    producer.round = 1;
    producer.report = Some("legacy producer result".into());
    store.update_task(&producer).unwrap();
    store.test_completion(&producer, "legacy producer result");
    store.db().execute("DELETE FROM agent_results WHERE turn_id IN (SELECT turn_id FROM turn_scopes WHERE task_id=?1)", [&sources[0].id]).unwrap();
    store
        .db()
        .execute(
            "INSERT INTO task_result_sources VALUES(?1,0,?2)",
            params![sources[0].id, producer.id],
        )
        .unwrap();
    store
        .db()
        .execute(
            "UPDATE tasks SET feedback=json_object('carried_from',?2) WHERE id=?1",
            params![sources[0].id, producer.id],
        )
        .unwrap();
    let writer = store.clone();
    test_observation::on_next_diff(move || {
        let db = writer
            .connection
            .try_lock()
            .expect("Store available during source read");
        db.execute(
            "UPDATE task_inputs SET generation=generation+1 WHERE task_id=?1",
            [&producer.id],
        )
        .unwrap();
    });
    let error = store
        .queue_plan(&conversation, &targets[0].plan_id)
        .unwrap_err()
        .to_string();
    assert!(error.contains("changed during preparation"), "{error}");
    assert_unpublished(&store, &targets[0].plan_id);
}

#[test]
fn carry_dependency_projection_uses_the_round_that_will_be_copied() {
    let (_dir, store, conversation, _, _, targets) = prepared_fixture(false);
    // The target's existing turn becomes visible when carry copies round one.
    // Its own producer must then win over the projected carried-from identity.
    store.test_completion(&targets[0], "target's own completed result");
    store
        .db()
        .execute(
            "UPDATE turn_scopes SET round=1 WHERE task_id=?1",
            [&targets[0].id],
        )
        .unwrap();
    store
        .queue_plan(&conversation, &targets[0].plan_id)
        .unwrap();
    assert_eq!(
        store.task_by_id(&targets[0].id).unwrap().unwrap().status,
        "approved"
    );
    assert_eq!(
        store.task_by_id(&targets[1].id).unwrap().unwrap().status,
        "queued"
    );
}

#[test]
fn carry_pins_the_compared_commit_and_rejects_a_moving_git_reference() {
    for mutate in [false, true] {
        let (dir, store, conversation, _, sources, targets) = prepared_fixture(false);
        let project = dir.path().join("project");
        let revision: String = store
            .db()
            .query_row(
                "SELECT revision FROM task_source_revisions WHERE task_id=?1",
                [&sources[0].id],
                |r| r.get(0),
            )
            .unwrap();
        let reference = "refs/heads/carry-input";
        git::ok(&project, &["update-ref", reference, &revision]).unwrap();
        store
            .db()
            .execute(
                "UPDATE task_source_revisions SET revision=?1 WHERE task_id=?2 OR task_id=?3",
                params![reference, sources[0].id, sources[1].id],
            )
            .unwrap();
        if mutate {
            std::fs::write(project.join("input"), "different tree").unwrap();
            let changed = git::snapshot(&project, git::CommitPurpose::TestFixture).unwrap();
            std::fs::write(project.join("input"), "before").unwrap();
            test_observation::on_next_diff(move || {
                git::ok(&project, &["update-ref", reference, &changed]).unwrap();
            });
        }
        let result = store.queue_plan(&conversation, &targets[0].plan_id);
        if mutate {
            let error = result.unwrap_err().to_string();
            assert!(
                error.contains("carry revision changed during preparation"),
                "{error}"
            );
            assert_unpublished(&store, &targets[0].plan_id);
        } else {
            result.unwrap();
            for target in &targets {
                let pinned: String = store
                    .db()
                    .query_row(
                        "SELECT revision FROM task_source_revisions WHERE task_id=?1",
                        [&target.id],
                        |r| r.get(0),
                    )
                    .unwrap();
                assert_eq!(pinned, revision);
                assert_eq!(
                    store.task_by_id(&target.id).unwrap().unwrap().status,
                    "approved"
                );
            }
        }
    }
}
