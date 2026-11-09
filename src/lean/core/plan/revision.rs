//! Reviewed revisions are applied behind a short scheduler barrier.
use crate::lean::store::PlanRevision;
use crate::lean::*;

mod acceptance;
mod review;

struct PreparedRevision {
    baseline: Vec<Task>,
    specs: Vec<TaskContract>,
    requirement_revision: i64,
    receipt: serde_json::Value,
}

struct RevisionRegistration<'a> {
    _graph_guard: std::sync::MutexGuard<'a, ()>,
    _lifecycle_guard: std::sync::MutexGuard<'a, ()>,
    request: String,
    fresh: bool,
}

impl<'a> RevisionRegistration<'a> {
    fn fresh(
        graph_guard: std::sync::MutexGuard<'a, ()>,
        lifecycle_guard: std::sync::MutexGuard<'a, ()>,
        request: String,
    ) -> Self {
        Self {
            _graph_guard: graph_guard,
            _lifecycle_guard: lifecycle_guard,
            request,
            fresh: true,
        }
    }

    fn existing(
        graph_guard: std::sync::MutexGuard<'a, ()>,
        lifecycle_guard: std::sync::MutexGuard<'a, ()>,
        request: String,
    ) -> Self {
        Self {
            _graph_guard: graph_guard,
            _lifecycle_guard: lifecycle_guard,
            request,
            fresh: false,
        }
    }
}

impl Core {
    pub(in crate::lean) fn submit_revision(
        self: &Arc<Self>,
        conversation: &str,
        request_key: &str,
        revision: PlanRevision,
    ) -> Result<serde_json::Value> {
        let registration = self.reserve_revision(conversation, request_key, &revision)?;
        if !registration.fresh {
            return self.store.revision_job(&registration.request);
        }
        self.process_fresh_revision(conversation, revision, registration)
    }

    fn process_fresh_revision(
        self: &Arc<Self>,
        conversation: &str,
        revision: PlanRevision,
        registration: RevisionRegistration<'_>,
    ) -> Result<serde_json::Value> {
        let request = registration.request.clone();
        let prepared = self.prepare_revision(conversation, &request, &revision);
        let prepared = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                self.store
                    .finish_revision_job(&request, &Err(Error::Invalid(error.to_string())))?;
                return Err(error);
            }
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let receipt = prepared.receipt.clone();
        let worker =
            self.spawn_revision_worker(conversation, request.clone(), revision, prepared, cancel);
        drop(registration);
        if let Err(error) = worker {
            self.store
                .finish_revision_job(&request, &Err(Error::Invalid(error.to_string())))?;
            return Err(error);
        }
        Ok(receipt)
    }

    fn reserve_revision(
        &self,
        conversation: &str,
        request_key: &str,
        revision: &PlanRevision,
    ) -> Result<RevisionRegistration<'_>> {
        // Keep the graph lock order used by plan execution, then serialize
        // durable reservation and worker registration against session deletion.
        let graph_guard = self.locks.graph_updates.lock().expect("graph update mutex");
        let lifecycle_guard = self
            .locks
            .session_lifecycle
            .lock()
            .expect("session lifecycle mutex");
        self.store.conversation(conversation)?;
        require(
            self.store
                .plan_belongs_to_conversation(conversation, &revision.plan)?,
            &format!(
                "revision.plan must be the plan's full UUID from the prompt, not plan text or a shortened ID (got {:?}); put changed tasks in revision.tasks",
                revision.plan.chars().take(40).collect::<String>()
            ),
        )?;
        let (request, fresh) =
            self.store
                .reserve_revision_job(conversation, request_key, revision)?;
        if !fresh {
            return Ok(RevisionRegistration::existing(
                graph_guard,
                lifecycle_guard,
                request,
            ));
        }
        Ok(RevisionRegistration::fresh(
            graph_guard,
            lifecycle_guard,
            request,
        ))
    }

    fn spawn_revision_worker(
        self: &Arc<Self>,
        conversation: &str,
        request: String,
        revision: PlanRevision,
        prepared: PreparedRevision,
        cancel: Arc<AtomicBool>,
    ) -> Result<()> {
        self.revisions
            .jobs
            .lock()
            .expect("revision jobs mutex")
            .insert(conversation.to_owned(), cancel.clone());
        let core = self.clone();
        let owner = conversation.to_owned();
        let worker_request = request.clone();
        std::thread::Builder::new()
            .name("plan-revision".into())
            .spawn(move || {
                run_revision_worker(core, owner, revision, prepared, cancel, worker_request)
            })
            .map(|_| ())
            .map_err(|error| {
                self.revisions
                    .jobs
                    .lock()
                    .expect("revision jobs mutex")
                    .remove(conversation);
                Error::Io(error)
            })
    }

    fn finish_revision_worker(
        self: &Arc<Self>,
        owner: &str,
        revision: &PlanRevision,
        request: &str,
        outcome: Result<()>,
    ) {
        if let Err(error) = self.store.finish_revision_job(request, &outcome) {
            self.notice(
                owner,
                &format!("could not record revision {request} outcome: {error}"),
            );
        }
        self.revisions
            .jobs
            .lock()
            .expect("revision jobs mutex")
            .remove(owner);
        let mut text = match &outcome {
            Ok(()) => format!("Plan revision {request} applied."),
            Err(error) => format!("Plan revision {request} rejected: {error}"),
        };
        if outcome.is_ok()
            && let Err(error) = self.maybe_continue_plan(owner, &revision.plan)
        {
            text.push_str(&format!(" Execution was not resumed: {error}"));
        }
        let _ = self.store.add_message(owner, RUNTIME, &text);
        self.wake(owner);
    }

    fn prepare_revision(
        &self,
        conversation: &str,
        request: &str,
        revision: &PlanRevision,
    ) -> Result<PreparedRevision> {
        require(
            self.store.plan_revision(&revision.plan)? == revision.base_revision,
            "stale plan revision",
        )?;
        self.prepare_revision_contents(conversation, request, revision)
    }

    fn prepare_revision_contents(
        &self,
        conversation: &str,
        request: &str,
        revision: &PlanRevision,
    ) -> Result<PreparedRevision> {
        let baseline = self.store.plan(&revision.plan)?;
        let specs = self.revised_specs(revision, &baseline)?;
        self.store.revision_checkpoints(conversation, revision)?;
        self.ensure_revision_accepted(conversation, revision)?;
        self.prepared_revision(conversation, request, revision, baseline, specs)
    }

    fn revised_specs(
        &self,
        revision: &PlanRevision,
        baseline: &[Task],
    ) -> Result<Vec<TaskContract>> {
        let config = self.config_snapshot();
        let specs = crate::lean::core::plan::helpers::finalize_plan(
            revision.merge(baseline)?,
            config.policy.required_gates,
            config.policy.local_options,
        );
        tasks::validate(&specs)?;
        Ok(specs)
    }

    fn ensure_revision_accepted(&self, conversation: &str, revision: &PlanRevision) -> Result<()> {
        acceptance::ensure_accepted_plan(self, conversation, &revision.plan)
    }

    fn prepared_revision(
        &self,
        conversation: &str,
        request: &str,
        revision: &PlanRevision,
        baseline: Vec<Task>,
        specs: Vec<TaskContract>,
    ) -> Result<PreparedRevision> {
        let run = self
            .store
            .plan_run(&revision.plan)?
            .ok_or_else(|| Error::Invalid("plan has no requirements".into()))?;
        let requirement_revision = self.store.run(conversation, &run)?.spec_revision;
        let receipt = self.store.revision_job(request)?;
        Ok(PreparedRevision {
            baseline,
            specs,
            requirement_revision,
            receipt,
        })
    }
}

fn run_revision_worker(
    core: Arc<Core>,
    owner: String,
    revision: PlanRevision,
    prepared: PreparedRevision,
    cancel: Arc<AtomicBool>,
    request: String,
) {
    let outcome = core.store.conversation(&owner).and_then(|conversation| {
        let context = review::ReviewContext {
            conversation: &owner,
            revision: &revision,
            baseline: &prepared.baseline,
            specs: &prepared.specs,
            cancel: &cancel,
            project: PathBuf::from(conversation.project),
        };
        core.review_and_apply_revision(
            context,
            &crate::lean::store::RevisionLease {
                requirement_revision: prepared.requirement_revision,
                request: &request,
            },
        )
    });
    core.finish_revision_worker(&owner, &revision, &request, outcome);
}

#[cfg(test)]
mod tests {
    use super::review::affected_tasks;
    use super::*;
    use std::collections::BTreeSet;

    fn contract(writes: bool, _paths: &[&str]) -> TaskContract {
        serde_json::from_value(json!({"id":"local","title":"Local","goal":"Inspect local behavior","change_kind": if writes { "code" } else { "none" }, "kind":"implementation","reviewer":{"summary": "Check local behavior"}})).unwrap()
    }

    #[test]
    fn delayed_revision_registration_cannot_start_after_deletion() {
        let directory = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!("../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&directory.path().join("state"), config).unwrap();
        let session = core.store.create_conversation(".").unwrap();
        let revision = PlanRevision {
            plan: crate::id(),
            base_revision: 0,
            reason: "race with session deletion".into(),
            tasks: vec![],
            retire: vec![],
            redo: vec![],
            replacements: Default::default(),
        };

        // Hold deletion's lifecycle gate until submit_revision has acquired
        // the graph lock and is waiting to register its durable job.
        let lifecycle = core
            .locks
            .session_lifecycle
            .lock()
            .expect("session lifecycle mutex");
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let submit_core = Arc::clone(&core);
        let conversation = session.id.clone();
        let submit = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            submit_core.submit_revision(&conversation, "delayed", revision)
        });
        started_rx.recv().unwrap();

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match core.locks.graph_updates.try_lock() {
                Ok(guard) => drop(guard),
                Err(std::sync::TryLockError::WouldBlock) => break,
                Err(std::sync::TryLockError::Poisoned(error)) => {
                    panic!("graph update mutex poisoned: {error}");
                }
            }
            assert!(
                std::time::Instant::now() < deadline,
                "revision submission did not acquire graph lock"
            );
            std::thread::yield_now();
        }

        core.store.delete_session(&session.id).unwrap();
        drop(lifecycle);
        let error = submit.join().unwrap().unwrap_err().to_string();

        assert!(error.contains("no conversation"), "{error}");
        assert!(core.store.conversation(&session.id).is_err());
        assert!(core.revisions.jobs.lock().unwrap().is_empty());
        let durable_jobs: i64 = core
            .store
            .db()
            .query_row(
                "SELECT count(*) FROM plan_revision_jobs WHERE conversation_id=?1",
                [&session.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(durable_jobs, 0);
    }

    #[test]
    fn rewording_an_approved_task_keeps_its_result_and_its_dependents() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::lean::store::Store::open(&dir.path().join("core.db")).unwrap();
        let conversation = store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        let mut baseline = store
            .add_plan(
                &conversation,
                &[
                    json!({"id":"a","title":"A","goal":"Inspect A","change_kind": "none", "kind":"analysis"}),
                    json!({"id":"b","title":"B","goal":"Inspect B","change_kind": "none", "kind":"analysis","depends_on":["a"]}),
                    json!({"id":"c","title":"C","goal":"Inspect C","change_kind": "none", "kind":"analysis"}),
                ],
            )
            .unwrap();
        for task in &mut baseline[..2] {
            task.status = status::APPROVED.into();
        }
        let specs: Vec<TaskContract> = baseline
            .iter()
            .map(|t| serde_json::from_value(t.spec.clone()).unwrap())
            .collect();
        let ids = |names: &[&str]| -> BTreeSet<String> {
            baseline
                .iter()
                .filter(|t| names.contains(&t.spec["id"].as_str().unwrap()))
                .map(|t| t.id.clone())
                .collect()
        };
        let with = |change: &dyn Fn(&mut Vec<TaskContract>)| {
            let mut specs = specs.clone();
            change(&mut specs);
            specs
        };

        let reworded = with(&|s| s[0].goal = "Inspect A, allowing the /tmp socket".into());
        assert_eq!(affected_tasks(&baseline, &reworded, &[]), ids(&["a", "b"]));
        let cosmetic = with(&|s| s[0].title = "Clearer title".into());
        assert!(affected_tasks(&baseline, &cosmetic, &[]).is_empty());
        assert_eq!(
            affected_tasks(&baseline, &reworded, &["a".into()]),
            ids(&["a", "b"])
        );
        // Work not yet approved follows its contract.
        let queued = with(&|s| s[2].goal = "Inspect C again".into());
        assert_eq!(affected_tasks(&baseline, &queued, &[]), ids(&["c"]));
    }

    #[test]
    fn changing_an_omitted_owner_contract_field_requires_a_fresh_options_selection() {
        let directory = tempfile::tempdir().unwrap();
        let store = crate::lean::store::Store::open(&directory.path().join("core.db")).unwrap();
        let conversation = store
            .create_conversation(directory.path().to_str().unwrap())
            .unwrap()
            .id;
        let owner: TaskContract = serde_json::from_value(json!({
            "id":"writer", "title":"Writer", "goal":"Implement selected behavior",
            "change_kind":"code", "kind":"implementation", "outputs":["old.txt"]
        }))
        .unwrap();
        let initial = tasks::with_options(vec![owner]);
        let mut baseline = store
            .add_plan(
                &conversation,
                &initial
                    .iter()
                    .map(|spec| serde_json::to_value(spec).unwrap())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        for task in &mut baseline {
            task.status = status::APPROVED.into();
        }
        let mut revised = initial;
        let writer = revised.iter_mut().find(|spec| spec.id == "writer").unwrap();
        writer.outputs = vec!["new.txt".into()];
        writer.delivery = Some(tasks::DeliveryMode::Untracked);
        let revised = tasks::refresh_existing_options(revised);

        let affected = affected_tasks(&baseline, &revised, &[]);
        let affected_specs: BTreeSet<_> = baseline
            .iter()
            .filter(|task| affected.contains(&task.id))
            .filter_map(|task| task.spec["id"].as_str().map(str::to_owned))
            .collect();

        assert_eq!(
            affected_specs,
            BTreeSet::from(["writer".into(), "writer-options".into()])
        );
        let options = revised
            .iter()
            .find(|spec| spec.options_for.as_deref() == Some("writer"))
            .unwrap();
        assert!(options.goal.contains("new.txt"));
        assert!(options.goal.contains("untracked"));
    }

    #[test]
    fn preparation_failure_releases_the_durable_revision_slot() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!("../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&dir.path().join("state"), config).unwrap();
        let conversation = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        let tasks = core
            .store
            .add_plan(
                &conversation,
                &[serde_json::to_value(contract(false, &[])).unwrap()],
            )
            .unwrap();
        let plan = &tasks[0].plan_id;
        core.store
            .db()
            .execute(
                "INSERT INTO plan_activations(plan_id,conversation_id) VALUES(?1,?2)",
                rusqlite::params![plan, conversation],
            )
            .unwrap();
        let revision = PlanRevision {
            plan: plan.clone(),
            base_revision: 0,
            reason: "Check missing requirement binding".into(),
            tasks: vec![],
            retire: vec![],
            redo: vec![],
            replacements: Default::default(),
        };
        for key in ["first", "second"] {
            let error = core
                .submit_revision(&conversation, key, revision.clone())
                .unwrap_err();
            assert!(
                error.to_string().contains("plan has no requirements"),
                "{error}"
            );
            let (state,reason):(String,String)=core.store.db().query_row("SELECT status,error FROM plan_revision_jobs WHERE conversation_id=?1 AND request_key=?2",rusqlite::params![conversation,key],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
            assert_eq!(state, "rejected");
            assert!(reason.contains("plan has no requirements"));
        }
        assert!(core.revisions.jobs.lock().unwrap().is_empty());
    }
}
