//! Reviewed revisions are applied behind a short scheduler barrier.
use crate::lean::store::PlanRevision;
use crate::lean::*;

mod acceptance;
mod review;
mod scope;

struct PreparedRevision {
    baseline: Vec<Task>,
    specs: Vec<TaskContract>,
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
    /// Change generated checks in the existing graph without invoking planning.
    pub(in crate::lean) fn select_rigor(
        self: &Arc<Self>,
        conversation: &str,
        team: tasks::Team,
        reason: &str,
    ) -> Result<()> {
        let changing = self.locks.turn(conversation, "rigor-change");
        let _changing = changing.lock().expect("conversation rigor change");
        let (baseline, revision, specs, accepted) = {
            let _graph = self.locks.graph();
            let baseline = self.store.latest_plan(conversation)?;
            let Some(plan) = baseline.first() else {
                return self.store.select_team(conversation, team, reason);
            };
            let accepted =
                self.store.accepted_plan(conversation)?.as_deref() == Some(&plan.plan_id);
            if self.store.team(conversation)? == team
                || (!accepted && baseline.iter().any(|task| task.status != status::PROPOSED))
            {
                return self.store.select_team(conversation, team, reason);
            }
            let current = baseline
                .iter()
                .map(|task| serde_json::from_value(task.spec.clone()))
                .collect::<std::result::Result<Vec<TaskContract>, _>>()?;
            let specs = super::proposal::checked_team_plan(&current, &self.config_snapshot(), team)
                .map_err(Error::Invalid)?;
            if current == specs {
                return self.store.select_team(conversation, team, reason);
            }
            let revision = PlanRevision {
                plan: plan.plan_id.clone(),
                base_revision: self.store.plan_revision(&plan.plan_id)?,
                reason: format!("Rigor changed to `{}`: {reason}", team.name()),
                scope: None,
                tasks: Vec::new(),
                retire: Vec::new(),
                redo: Vec::new(),
                replacements: Default::default(),
            };
            (baseline, revision, specs, accepted)
        };
        self.apply_direct_revision(
            conversation,
            &revision,
            &baseline,
            &specs,
            review::DirectRevision::Rigor(team, reason),
            accepted,
        )?;
        if accepted {
            self.maybe_continue_plan(conversation, &revision.plan)?;
        }
        Ok(())
    }

    pub(in crate::lean) fn submit_revision(
        self: &Arc<Self>,
        conversation: &str,
        request_key: &str,
        mut revision: PlanRevision,
    ) -> Result<serde_json::Value> {
        self.include_scope_sources(conversation, &mut revision)?;
        self.supersede_scope_revision(conversation, &revision.plan, request_key)?;
        let registration = self.reserve_revision(conversation, request_key, &revision)?;
        if !registration.fresh {
            return self.store.revision_job(&registration.request);
        }
        self.process_fresh_revision(conversation, revision, registration)
    }

    pub(in crate::lean) fn apply_user_revision(
        self: &Arc<Self>,
        conversation: &str,
        request_key: &str,
        mut revision: PlanRevision,
        request_ai_review: bool,
    ) -> Result<serde_json::Value> {
        self.include_scope_sources(conversation, &mut revision)?;
        self.supersede_scope_revision(conversation, &revision.plan, request_key)?;
        let baseline = self.store.plan(&revision.plan)?;
        require(!baseline.is_empty(), "plan has no tasks")?;
        self.normalize_user_revision_tasks(conversation, &mut revision, &baseline)?;
        if request_ai_review || revision.scope.is_some() {
            return self.submit_revision(conversation, request_key, revision);
        }
        require(
            self.store.plan_revision(&revision.plan)? == revision.base_revision,
            "stale plan revision",
        )?;
        let is_accepted =
            acceptance::ensure_accepted_plan(self, conversation, &revision.plan).is_ok();
        if !is_accepted {
            let is_latest = self
                .store
                .latest_plan(conversation)?
                .first()
                .is_some_and(|task| task.plan_id == revision.plan);
            require(
                is_latest,
                "plan is no longer the active or proposed plan of this conversation",
            )?;
        }
        let specs = self.revised_specs(&revision, &baseline)?;
        self.store.revision_checkpoints(conversation, &revision)?;
        let registration = self.reserve_revision(conversation, request_key, &revision)?;
        if !registration.fresh {
            return self.store.revision_job(&registration.request);
        }
        let request = registration.request.clone();
        drop(registration);
        let lease = crate::lean::store::RevisionLease {
            accepted_scope: false,
            request: &request,
        };
        if let Err(error) = self.apply_direct_revision(
            conversation,
            &revision,
            &baseline,
            &specs,
            review::DirectRevision::Reviewed(&lease),
            is_accepted,
        ) {
            let _ = self
                .store
                .finish_revision_job(&request, &Err(Error::Invalid(error.to_string())));
            return Err(error);
        }
        if is_accepted {
            let _ = self.maybe_continue_plan(conversation, &revision.plan);
        }
        self.wake(conversation);
        Ok(json!({
            "status": "applied",
            "request": request,
            "revision": revision.base_revision + 1,
        }))
    }

    fn include_scope_sources(&self, conversation: &str, revision: &mut PlanRevision) -> Result<()> {
        if let Some(id) = self.store.plan_scope_id(&revision.plan)?
            && !self.store.scope_documents_current(conversation, &id)?
            && revision.scope.is_none()
        {
            revision.scope = Some(self.store.scope(conversation, &id)?.summary);
        }
        Ok(())
    }

    fn normalize_user_revision_tasks(
        &self,
        _conversation: &str,
        revision: &mut PlanRevision,
        baseline: &[Task],
    ) -> Result<()> {
        for spec in &mut revision.tasks {
            if let Some(old_task) = baseline.iter().find(|old| old.spec["id"] == spec.id) {
                let old_spec: TaskContract = serde_json::from_value(old_task.spec.clone())?;
                if spec
                    .reviewer
                    .summary
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                    && !old_spec
                        .reviewer
                        .summary
                        .as_deref()
                        .unwrap_or_default()
                        .trim()
                        .is_empty()
                {
                    spec.reviewer = old_spec.reviewer;
                }
                if spec.delivery.is_none() {
                    spec.delivery = old_spec.delivery;
                }
                if spec.section.is_none() {
                    spec.section = old_spec.section;
                }
                if spec.summary.is_none() {
                    spec.summary = old_spec.summary;
                }
            } else {
                if spec
                    .reviewer
                    .summary
                    .as_deref()
                    .unwrap_or_default()
                    .trim()
                    .is_empty()
                {
                    spec.reviewer.summary = Some(format!("Verify {}", spec.title));
                }
            }
        }
        Ok(())
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
        let graph_guard = self.locks.graph();
        let lifecycle_guard = self
            .locks
            .session_lifecycle
            .lock()
            .expect("session lifecycle mutex");
        self.require_accepting()?;
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
        let waiting = self
            .store
            .pending_scope_revision(&revision.plan)
            .ok()
            .flatten()
            .is_some();
        let mut text = match &outcome {
            Ok(()) if waiting => {
                format!("Plan revision {request} reviewed; scope change awaits your acceptance.")
            }
            Ok(()) => format!("Plan revision {request} applied."),
            Err(error) => format!("Plan revision {request} rejected: {error}"),
        };
        if outcome.is_ok()
            && !waiting
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
        let team = self
            .store
            .team(&self.store.plan_conversation(&revision.plan)?)?;
        let specs = crate::lean::core::plan::helpers::finalize_team_plan(
            revision.merge(baseline)?,
            config.policy.required_gates,
            config.policy.local_options,
            team,
        );
        tasks::validate(&specs)?;
        Ok(specs)
    }

    fn ensure_revision_accepted(&self, conversation: &str, revision: &PlanRevision) -> Result<()> {
        acceptance::ensure_accepted_plan(self, conversation, &revision.plan)
    }

    fn prepared_revision(
        &self,
        _conversation: &str,
        request: &str,
        revision: &PlanRevision,
        baseline: Vec<Task>,
        specs: Vec<TaskContract>,
    ) -> Result<PreparedRevision> {
        require(
            self.store.plan_scope_id(&revision.plan)?.is_some(),
            "plan has no scope",
        )?;
        let receipt = self.store.revision_job(request)?;
        Ok(PreparedRevision {
            baseline,
            specs,
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
                accepted_scope: false,
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
        serde_json::from_value(json!({"id":"local","title":"Local","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Inspect local behavior","change_kind": if writes { "code" } else { "none" }, "kind":"implementation","reviewer":{"summary": "Check local behavior"}})).unwrap()
    }

    #[test]
    fn delayed_revision_registration_cannot_start_after_deletion() {
        let directory = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!("../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&directory.path().join("state"), config).unwrap();
        let session = core.store.create_conversation(".").unwrap();
        let revision = PlanRevision {
            scope: None,
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
                    json!({"id":"a","title":"A","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Inspect A","change_kind": "none", "kind":"analysis"}),
                    json!({"id":"b","title":"B","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Inspect B","change_kind": "none", "kind":"analysis","depends_on":["a"]}),
                    json!({"id":"c","title":"C","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Inspect C","change_kind": "none", "kind":"analysis"}),
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
            "id":"writer", "title":"Writer", "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Implement selected behavior",
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
    fn plan_revision_merge_keeps_options_off_automatically_added_gates() {
        let directory = tempfile::tempdir().unwrap();
        let store = crate::lean::store::Store::open(&directory.path().join("core.db")).unwrap();
        let conversation = store
            .create_conversation(directory.path().to_str().unwrap())
            .unwrap()
            .id;
        let owner: TaskContract = serde_json::from_value(json!({
            "id":"writer", "title":"Writer", "goal":"Implement the requested behavior",
            "change_kind":"code", "kind":"implementation",
            "acceptance_criteria":[{"id":"C1","text":"The requested behavior is implemented"}]
        }))
        .unwrap();
        let initial = tasks::with_options(tasks::with_required_agents(vec![owner]));
        let baseline = store
            .add_plan(
                &conversation,
                &initial
                    .iter()
                    .map(|spec| serde_json::to_value(spec).unwrap())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let mut revised_writer = initial
            .iter()
            .find(|spec| spec.id == "writer")
            .unwrap()
            .clone();
        revised_writer.goal = "Implement the revised requested behavior".into();
        let revision = PlanRevision {
            plan: baseline[0].plan_id.clone(),
            base_revision: 0,
            reason: "clarify the writer objective".into(),
            scope: None,
            tasks: vec![revised_writer],
            retire: vec![],
            redo: vec![],
            replacements: Default::default(),
        };

        let merged = revision.merge(&baseline).unwrap();

        tasks::validate(&merged).unwrap();
        assert!(
            merged
                .iter()
                .any(|spec| spec.options_for.as_deref() == Some("writer"))
        );
        for gate in merged.iter().filter(|spec| spec.quality_gate) {
            assert!(
                !merged
                    .iter()
                    .any(|spec| spec.options_for.as_deref() == Some(&gate.id))
            );
        }
    }

    #[test]
    fn small_change_revisions_keep_the_selected_gates_for_existing_and_new_writers() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!("../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&dir.path().join("state"), config.clone()).unwrap();
        let conversation = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        core.store
            .select_planned_team(&conversation, tasks::Team::SmallChange, "contained fix")
            .unwrap();
        let writer: TaskContract = serde_json::from_value(json!({
            "id":"writer", "title":"Writer", "goal":"Fix contained behavior", "kind":"implementation", "change_kind":"code",
            "acceptance_criteria":[{"id":"C1","text":"The behavior works"}]
        })).unwrap();
        let initial = crate::lean::core::plan::proposal::checked_team_plan(
            std::slice::from_ref(&writer),
            &config,
            tasks::Team::SmallChange,
        )
        .unwrap();
        // Publication's rigor check must be idempotent for a generated graph.
        assert_eq!(
            crate::lean::core::plan::proposal::checked_team_plan(
                &initial,
                &config,
                tasks::Team::SmallChange
            )
            .unwrap(),
            initial
        );
        let baseline = core
            .store
            .add_plan(
                &conversation,
                &initial
                    .iter()
                    .map(|spec| serde_json::to_value(spec).unwrap())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let mut new_writer = writer.clone();
        new_writer.id = "new-writer".into();
        new_writer.depends_on = vec!["writer".into()];
        let mut revision = PlanRevision {
            plan: baseline[0].plan_id.clone(),
            base_revision: 0,
            reason: "Add a follow-up fix".into(),
            scope: None,
            tasks: vec![new_writer],
            retire: vec![],
            redo: vec![],
            replacements: Default::default(),
        };
        for _ in 0..2 {
            let revised = core.revised_specs(&revision, &baseline).unwrap();
            let gates = revised
                .iter()
                .filter(|spec| spec.quality_gate)
                .map(|spec| spec.kind)
                .collect::<BTreeSet<_>>();
            assert_eq!(
                gates,
                tasks::Team::SmallChange.gates().iter().copied().collect()
            );
            assert_eq!(
                core.store.team(&conversation).unwrap(),
                tasks::Team::SmallChange
            );
            revision.tasks[0].goal.push_str(" with updated details");
        }
        let writer_id = baseline
            .iter()
            .find(|task| task.spec["id"] == "writer")
            .unwrap()
            .id
            .clone();
        for team in [
            tasks::Team::Feature,
            tasks::Team::SecurityFix,
            tasks::Team::Refactoring,
            tasks::Team::SmallChange,
        ] {
            core.select_rigor(&conversation, team, "User changed quality checks")
                .unwrap();
            let current = core.store.plan(&revision.plan).unwrap();
            let specs = current
                .iter()
                .map(|task| serde_json::from_value::<TaskContract>(task.spec.clone()).unwrap())
                .collect::<Vec<_>>();
            tasks::validate(&specs).unwrap();
            assert_eq!(
                core.store.latest_plan(&conversation).unwrap()[0].plan_id,
                revision.plan
            );
            let current_writer = current.iter().find(|task| task.id == writer_id).unwrap();
            let mut contract: TaskContract =
                serde_json::from_value(current_writer.spec.clone()).unwrap();
            contract.depends_on.clear();
            assert_eq!(contract, writer);
            assert!(current.iter().all(|task| task.status == status::PROPOSED));
            let gates = specs
                .iter()
                .filter(|spec| spec.quality_gate)
                .map(|spec| spec.kind)
                .collect::<BTreeSet<_>>();
            assert_eq!(gates, team.gates().iter().copied().collect());
            assert_eq!(
                specs
                    .iter()
                    .any(|spec| spec.options_for.as_deref() == Some("writer")),
                team.options()
            );
        }
        let snapshot = core.store.plan(&revision.plan).unwrap();
        let head = core.store.plan_revision(&revision.plan).unwrap();
        core.select_rigor(
            &conversation,
            tasks::Team::SmallChange,
            "Confirm existing rigor",
        )
        .unwrap();
        assert_eq!(core.store.plan(&revision.plan).unwrap(), snapshot);
        assert_eq!(core.store.plan_revision(&revision.plan).unwrap(), head);
        let error = core
            .select_rigor(&conversation, tasks::Team::Research, "Switch to research")
            .unwrap_err();
        assert!(error.to_string().contains("changes no code"), "{error}");
        assert_eq!(
            core.store.team(&conversation).unwrap(),
            tasks::Team::SmallChange
        );
        assert_eq!(core.store.plan(&revision.plan).unwrap(), snapshot);

        // Failed persistence rolls back both the graph and the selected rigor.
        core.store.db().execute_batch("CREATE TRIGGER fail_rigor BEFORE UPDATE ON teams BEGIN SELECT RAISE(ABORT,'rigor write failed'); END;").unwrap();
        let events_before = core
            .store
            .events_since(&conversation, 0, 1000)
            .unwrap()
            .len();
        let error = core
            .select_rigor(&conversation, tasks::Team::Feature, "Full checks")
            .unwrap_err();
        assert!(error.to_string().contains("rigor write failed"), "{error}");
        assert_eq!(
            core.store.team(&conversation).unwrap(),
            tasks::Team::SmallChange
        );
        assert_eq!(core.store.plan(&revision.plan).unwrap(), snapshot);
        assert_eq!(core.store.plan_revision(&revision.plan).unwrap(), head);
        let events = core.store.events_since(&conversation, 0, 1000).unwrap();
        assert_eq!(events.len(), events_before);
        assert!(events.iter().all(|event| !matches!(
            event.kind.as_str(),
            "plan_rejected" | "plan_invalid" | "turn_started" | "message"
        )));
        assert!(core.is_idle(&conversation));
        assert_eq!(
            core.store
                .db()
                .query_row(
                    "SELECT count(*) FROM plans WHERE conversation_id=?1",
                    [&conversation],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
    }

    #[test]
    fn revision_adds_multi_producer_writer_with_acyclic_options_dependency() {
        let dir = tempfile::tempdir().unwrap();
        let mut config =
            Config::from_toml(include_str!("../../../../examples/antislop.toml")).unwrap();
        config.policy.local_options = true;
        let core = Core::open(&dir.path().join("state"), config.clone()).unwrap();
        let conversation = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        core.store
            .select_planned_team(&conversation, tasks::Team::Feature, "feature")
            .unwrap();

        let code_task = |id: &str, depends_on: &[&str]| -> TaskContract {
            serde_json::from_value(json!({
                "id": id, "title": id, "goal": "Prepare an input", "kind": "implementation",
                "change_kind": "code", "depends_on": depends_on,
                "acceptance_criteria": [{"id":"C1","text":"Input is ready"}]
            }))
            .unwrap()
        };
        let producers = vec![
            code_task("v2-mockups", &[]),
            code_task("v2-state", &["v2-mockups"]),
            code_task("v2-wb-shell", &["v2-state"]),
            code_task("v2-wb-content", &["v2-wb-shell"]),
            code_task("v2-wb-pty", &["v2-wb-shell"]),
            code_task("v2-graph", &["v2-wb-shell", "v2-wb-pty"]),
            code_task("v2-projects", &["v2-state"]),
            code_task("v2-composer", &["v2-projects"]),
            code_task("v2-plans", &["v2-state"]),
            code_task("v2-decisions", &["v2-state"]),
            code_task("v2-recovery", &["v2-wb-content"]),
            code_task("v2-settings", &["v2-state"]),
            serde_json::from_value(json!({
                "id":"v2-readiness", "title":"Readiness", "goal":"Check readiness",
                "kind":"analysis", "change_kind":"artifact",
                "outputs":["reports/readiness.md"],
                "depends_on":["v2-mockups","v2-state","v2-wb-shell","v2-wb-content","v2-wb-pty","v2-graph","v2-projects","v2-composer","v2-plans","v2-decisions","v2-recovery","v2-settings"],
                "acceptance_criteria":[{"id":"C1","text":"Readiness is recorded"}]
            }))
            .unwrap(),
            code_task("v2-default", &["v2-readiness"]),
            serde_json::from_value(json!({
                "id":"v2-final-record", "title":"Final record", "goal":"Record the final state",
                "kind":"analysis", "change_kind":"artifact", "outputs":["reports/final.md"],
                "depends_on":["v2-default","v2-readiness"],
                "acceptance_criteria":[{"id":"C1","text":"Final state is recorded"}]
            }))
            .unwrap(),
        ];
        let mut initial = crate::lean::core::plan::proposal::checked_team_plan(
            &producers,
            &config,
            tasks::Team::Feature,
        )
        .unwrap();
        let gate_ids = initial
            .iter()
            .filter(|spec| spec.quality_gate)
            .map(|spec| spec.id.clone())
            .collect::<Vec<_>>();
        let final_record = initial
            .iter_mut()
            .find(|spec| spec.id == "v2-final-record")
            .unwrap();
        final_record.depends_on.extend(gate_ids);
        let initial = tasks::order_graph(initial).unwrap();
        tasks::validate(&initial).unwrap();
        let baseline = core
            .store
            .add_plan(
                &conversation,
                &initial
                    .iter()
                    .map(|spec| serde_json::to_value(spec).unwrap())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        assert_eq!(baseline.len(), 34);
        assert_eq!(
            baseline
                .iter()
                .map(|task| task.spec["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "v2-mockups-options",
                "v2-mockups",
                "v2-state-options",
                "v2-state",
                "v2-wb-shell-options",
                "v2-wb-shell",
                "v2-wb-content-options",
                "v2-wb-content",
                "v2-wb-pty-options",
                "v2-wb-pty",
                "v2-graph-options",
                "v2-graph",
                "v2-projects-options",
                "v2-projects",
                "v2-composer-options",
                "v2-composer",
                "v2-plans-options",
                "v2-plans",
                "v2-decisions-options",
                "v2-decisions",
                "v2-recovery-options",
                "v2-recovery",
                "v2-settings-options",
                "v2-settings",
                "v2-readiness",
                "v2-default-options",
                "v2-default",
                "quality-refactoring",
                "static-analyzer",
                "review-testing",
                "security-auditor",
                "performance-auditor",
                "integration-gate",
                "v2-final-record",
            ]
        );
        assert_eq!(
            baseline
                .iter()
                .find(|task| task.spec["id"] == "v2-graph-options")
                .unwrap()
                .spec["depends_on"],
            json!(["v2-wb-shell", "v2-wb-pty"])
        );

        let mut revision: PlanRevision =
            serde_json::from_str(include_str!("testdata/revision-279b43ec.json")).unwrap();
        revision.plan = baseline[0].plan_id.clone();

        let revised = core.revised_specs(&revision, &baseline).unwrap();
        assert_eq!(core.revised_specs(&revision, &baseline).unwrap(), revised);
        tasks::validate(&revised).unwrap();

        let options = revised
            .iter()
            .find(|spec| spec.options_for.as_deref() == Some("v2-capture-adapter"))
            .unwrap();
        let writer = revised
            .iter()
            .find(|spec| spec.id == "v2-capture-adapter")
            .unwrap();
        assert_eq!(options.id, "v2-capture-adapter-options");
        assert_eq!(options.depends_on, writer.depends_on[..11]);
        assert_eq!(writer.depends_on.last(), Some(&options.id));
        assert!(!options.depends_on.contains(&options.id));
        assert!(!options.quality_gate);
        let readiness = revised
            .iter()
            .find(|spec| spec.id == "v2-readiness")
            .unwrap();
        assert!(readiness.depends_on.contains(&writer.id));
        assert_eq!(
            revised
                .iter()
                .filter(|spec| spec.quality_gate)
                .map(|spec| spec.kind)
                .collect::<BTreeSet<_>>(),
            tasks::Team::Feature.gates().iter().copied().collect()
        );
        assert!(revised.iter().filter(|spec| spec.quality_gate).all(|gate| {
            revised
                .iter()
                .all(|spec| spec.options_for.as_deref() != Some(&gate.id))
        }));
    }

    #[test]
    fn rigor_change_preserves_completed_authored_work_in_an_accepted_plan() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!("../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&dir.path().join("state"), config.clone()).unwrap();
        let conversation = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        core.store
            .select_planned_team(&conversation, tasks::Team::SmallChange, "Contained fix")
            .unwrap();
        let specs = crate::lean::core::plan::proposal::checked_team_plan(
            &[contract(true, &[])],
            &config,
            tasks::Team::SmallChange,
        )
        .unwrap();
        let mut baseline = core
            .store
            .add_plan(
                &conversation,
                &specs
                    .iter()
                    .map(|spec| serde_json::to_value(spec).unwrap())
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        let plan = baseline[0].plan_id.clone();
        for task in &mut baseline {
            task.status = status::QUEUED.into();
            core.store.update_task(task).unwrap();
        }
        let writer = baseline
            .iter_mut()
            .find(|task| task.spec["id"] == "local")
            .unwrap();
        writer.status = status::APPROVED.into();
        writer.report = Some("Completed implementation".into());
        writer.base = Some("base-checkpoint".into());
        writer.result = Some("completed-checkpoint".into());
        core.store.update_task(writer).unwrap();
        let writer = core.store.task_by_id(&writer.id).unwrap().unwrap();
        core.store
            .db()
            .execute(
                "INSERT INTO plan_activations(plan_id,conversation_id) VALUES(?1,?2)",
                rusqlite::params![plan, conversation],
            )
            .unwrap();
        // Keep scheduling separate from this graph-update assertion.
        core.activity
            .plans
            .lock()
            .unwrap()
            .insert(conversation.clone(), Arc::new(AtomicBool::new(false)));
        core.select_rigor(&conversation, tasks::Team::Feature, "Full checks")
            .unwrap();
        assert_eq!(core.store.task_by_id(&writer.id).unwrap().unwrap(), writer);
        let current = core.store.plan(&plan).unwrap();
        assert!(
            current
                .iter()
                .any(|task| task.spec["quality_gate"] == true && task.status == status::QUEUED)
        );
        assert!(
            current
                .iter()
                .all(|task| task.status != status::PROPOSED && task.status != status::CANCELLED)
        );
        assert_eq!(core.store.accepted_plan(&conversation).unwrap(), Some(plan));
        assert!(
            core.store
                .events_since(&conversation, 0, 1000)
                .unwrap()
                .iter()
                .all(|event| !matches!(
                    event.kind.as_str(),
                    "plan_rejected" | "turn_started" | "message"
                ))
        );
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
            scope: None,
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
            assert!(error.to_string().contains("plan has no scope"), "{error}");
            let (state,reason):(String,String)=core.store.db().query_row("SELECT status,error FROM plan_revision_jobs WHERE conversation_id=?1 AND request_key=?2",rusqlite::params![conversation,key],|r|Ok((r.get(0)?,r.get(1)?))).unwrap();
            assert_eq!(state, "rejected");
            assert!(reason.contains("plan has no scope"));
        }
        assert!(core.revisions.jobs.lock().unwrap().is_empty());
    }

    #[test]
    fn user_fast_path_revision_applies_immediately_and_warns_on_uncovered_criteria() {
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
                &[
                    json!({"id":"t1","title":"Task 1","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"First step","change_kind":"none","kind":"analysis","reviewer":{"summary":"Check t1"}}),
                    json!({"id":"t2","title":"Task 2","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Second step","change_kind":"none","kind":"analysis","depends_on":["t1"],"reviewer":{"summary":"Check t2"}}),
                ],
            )
            .unwrap();
        let plan = tasks[0].plan_id.clone();

        let edited: TaskContract = serde_json::from_value(json!({
            "id": "t1",
            "title": "Updated Task 1",
            "acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal": "Refined first step",
            "change_kind": "none",
            "kind": "analysis",
            "deferred": true
        }))
        .unwrap();
        let revision = PlanRevision {
            scope: None,
            plan: plan.clone(),
            base_revision: 0,
            reason: "Hold and refine t1 from UI".into(),
            tasks: vec![edited],
            retire: vec![],
            redo: vec![],
            replacements: Default::default(),
        };
        let reply = core
            .apply_user_revision(&conversation, "ui-edit-1", revision, false)
            .unwrap();
        assert_eq!(reply["status"], "applied");
        assert_eq!(reply["revision"], 1);
        let updated = core.store.plan(&plan).unwrap();
        assert_eq!(updated[0].spec["title"], "Updated Task 1");
        assert_eq!(updated[0].spec["deferred"], true);
        assert_eq!(updated[0].status, "proposed");
    }

    #[test]
    fn direct_execution_creates_bound_single_task_plan_without_orchestrator_ceremony() {
        let dir = tempfile::tempdir().unwrap();
        let mut config =
            Config::from_toml(include_str!("../../../../examples/antislop.toml")).unwrap();
        for provider in config.providers.values_mut() {
            provider.binary = std::path::PathBuf::from("/bin/sh");
        }
        for profile in config.profiles.values_mut() {
            profile.binary = Some("/bin/sh".into());
        }
        let core = Core::open(&dir.path().join("state"), config).unwrap();
        let conversation = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;

        let reply = core
            .submit_addressed(&conversation, "/direct Fix typo in README", None)
            .unwrap();
        let plan_id = reply["direct_plan"].as_str().unwrap();
        assert_eq!(reply["task"], "direct-1");

        // Cancel the background runner immediately so the test doesn't wait on /bin/sh
        core.cancel(&conversation);

        let tasks = core.store.plan(plan_id).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].spec["id"], "direct-1");
        assert_eq!(tasks[0].spec["title"], "Fix typo in README");
        assert!(
            core.store
                .plan_review_approved(&conversation, plan_id)
                .unwrap()
        );
        assert_eq!(
            core.store.workflow(&conversation).unwrap().phase,
            "implementation"
        );
    }
}
