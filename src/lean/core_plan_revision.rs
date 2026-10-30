//! Reviewed revisions are applied behind a short scheduler barrier.
use super::store::PlanRevision;
use super::*;

mod review;
mod scope;

struct PreparedRevision {
    baseline: Vec<Task>,
    specs: Vec<TaskContract>,
    requirement_revision: i64,
    receipt: serde_json::Value,
}

struct RevisionRegistration<'a> {
    _guard: std::sync::MutexGuard<'a, ()>,
    request: String,
    fresh: bool,
}

impl<'a> RevisionRegistration<'a> {
    fn fresh(guard: std::sync::MutexGuard<'a, ()>, request: String) -> Self {
        Self {
            _guard: guard,
            request,
            fresh: true,
        }
    }

    fn existing(guard: std::sync::MutexGuard<'a, ()>, request: String) -> Self {
        Self {
            _guard: guard,
            request,
            fresh: false,
        }
    }
}

impl Core {
    pub(super) fn submit_revision(
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
        require(
            self.store
                .plan_belongs_to_conversation(conversation, &revision.plan)?,
            &format!(
                "revision.plan must be the plan's full UUID from the prompt, not plan text or a shortened ID (got {:?}); put changed tasks in revision.tasks",
                revision.plan.chars().take(40).collect::<String>()
            ),
        )?;
        let guard = self.graph_updates.lock().expect("graph update mutex");
        let (request, fresh) =
            self.store
                .reserve_revision_job(conversation, request_key, revision)?;
        if !fresh {
            return Ok(RevisionRegistration::existing(guard, request));
        }
        Ok(RevisionRegistration::fresh(guard, request))
    }

    fn spawn_revision_worker(
        self: &Arc<Self>,
        conversation: &str,
        request: String,
        revision: PlanRevision,
        prepared: PreparedRevision,
        cancel: Arc<AtomicBool>,
    ) -> Result<()> {
        self.revision_jobs
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
                self.revision_jobs
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
        self.revision_jobs
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
        let specs = super::core_plan_helpers::finalize_plan(
            revision.merge(&baseline)?,
            self.config_snapshot().policy.required_gates,
            self.config_snapshot().policy.local_options,
        );
        tasks::validate(&specs)?;
        self.validate_revision_scope(conversation, &revision.plan, &specs)?;
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

    fn validate_revision_scope(
        &self,
        conversation: &str,
        plan: &str,
        specs: &[TaskContract],
    ) -> Result<()> {
        scope::validate_plan_authorization(self, conversation, plan, specs)
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
            &super::store::RevisionLease {
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
    use super::scope::{covers, validate_authorized_scope};
    use super::*;
    use std::collections::BTreeSet;

    fn contract(writes: bool, paths: &[&str]) -> TaskContract {
        serde_json::from_value(json!({"id":"local","title":"Local","goal":"Inspect local behavior","kind":"implementation","writes":writes,"scope":paths,"verifier":{"focus":"Check local behavior"}})).unwrap()
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
                    json!({"id":"a","title":"A","goal":"Inspect A","kind":"analysis","scope":["src"]}),
                    json!({"id":"b","title":"B","goal":"Inspect B","kind":"analysis","depends_on":["a"]}),
                    json!({"id":"c","title":"C","goal":"Inspect C","kind":"analysis"}),
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
        assert!(affected_tasks(&baseline, &reworded, &[]).is_empty());
        assert_eq!(
            affected_tasks(&baseline, &reworded, &["a".into()]),
            ids(&["a", "b"])
        );
        let rescoped = with(&|s| s[0].scope = vec!["src".into(), "tests".into()]);
        assert_eq!(affected_tasks(&baseline, &rescoped, &[]), ids(&["a", "b"]));
        // Work not yet approved follows its contract.
        let queued = with(&|s| s[2].goal = "Inspect C again".into());
        assert_eq!(affected_tasks(&baseline, &queued, &[]), ids(&["c"]));
    }

    #[test]
    fn revision_preserves_separate_read_and_write_authorizations() {
        let read = vec!["src".into(), "docs".into()];
        let write = vec!["src".into()];
        assert!(validate_authorized_scope(&[contract(false, &["docs"])], &read, &write).is_ok());
        assert!(
            validate_authorized_scope(&[contract(true, &["src/api.rs"])], &read, &write).is_ok()
        );
        assert!(validate_authorized_scope(&[contract(true, &["docs"])], &read, &write).is_err());
        assert!(validate_authorized_scope(&[contract(true, &["src"])], &read, &[]).is_err());
        assert!(validate_authorized_scope(&[contract(false, &[])], &read, &write).is_err());
        assert!(
            validate_authorized_scope(
                &[contract(true, &["src/../secrets"])],
                &[".".into()],
                &[".".into()]
            )
            .is_err()
        );
        assert!(
            validate_authorized_scope(
                &[contract(false, &["../service"])],
                &["../service".into()],
                &[]
            )
            .is_ok()
        );
    }
    #[test]
    fn approved_globs_cover_the_paths_they_name() {
        let read: Vec<String> = vec!["**/*".into(), "docs/**".into()];
        let write: Vec<String> = vec!["src/lean/ledger/**".into(), "tests/*.rs".into()];
        let ok =
            |scope: &[&str]| validate_authorized_scope(&[contract(true, scope)], &read, &write);
        assert!(ok(&["src/lean/ledger/storage/validation.rs"]).is_ok());
        assert!(ok(&["src/lean/ledger", "tests/lean_ledger.rs"]).is_ok());
        assert!(ok(&["src/lean/ledger/storage/**"]).is_ok());
        assert!(ok(&["src/lean/tui/graph.rs"]).is_err());
        assert!(ok(&["src/lean/**"]).is_err());
        assert!(ok(&["tests/fixtures/data.rs"]).is_err());
        assert!(covers("src/**", "src/lean/core_plan_revision.rs"));
        assert!(covers("src/lean/core*", "src/lean/core_plan_revision.rs"));
        assert!(!covers("src/lean/core*", "src/lean/store.rs"));
        assert!(!covers("docs/**", "src/lib.rs"));
    }

    #[test]
    fn preparation_failure_releases_the_durable_revision_slot() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!("../../examples/antislop.toml")).unwrap();
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
        core.store.db().execute("INSERT INTO plan_authorizations(plan_id,conversation_id,scope,write_scope) VALUES(?1,?2,'[\".\"]','[]')",rusqlite::params![plan,conversation]).unwrap();
        let revision = PlanRevision {
            plan: plan.clone(),
            base_revision: 0,
            reason: "Check missing requirement binding".into(),
            tasks: vec![],
            retire: vec![],
            redo: vec![],
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
        assert!(core.revision_jobs.lock().unwrap().is_empty());
    }
}
