//! Reviewed revisions are applied behind a short scheduler barrier.
use super::store::PlanRevision;
use super::*;
use std::collections::BTreeSet;

struct PreparedRevision {
    baseline: Vec<Task>,
    specs: Vec<TaskContract>,
    requirement_revision: i64,
    receipt: serde_json::Value,
}

impl Core {
    pub(super) fn submit_revision(
        self: &Arc<Self>,
        conversation: &str,
        request_key: &str,
        revision: PlanRevision,
    ) -> Result<serde_json::Value> {
        require(
            self.store
                .plan_belongs_to_conversation(conversation, &revision.plan)?,
            &format!(
                "revision.plan must be the plan's full UUID from the prompt, not plan text or a shortened ID (got {:?}); put changed tasks in revision.tasks",
                revision.plan.chars().take(40).collect::<String>()
            ),
        )?;
        // Reservation and in-memory registration share the finalization barrier.
        // No fallible preparation may leave a durable job without an owner.
        let registration = self.graph_updates.lock().expect("graph update mutex");
        let (request, fresh) =
            self.store
                .reserve_revision_job(conversation, request_key, &revision)?;
        if !fresh {
            return self.store.revision_job(&request);
        }
        let prepared = self.prepare_revision(conversation, &request, &revision);
        let PreparedRevision {
            baseline,
            specs,
            requirement_revision,
            receipt,
        } = match prepared {
            Ok(prepared) => prepared,
            Err(error) => {
                self.store
                    .finish_revision_job(&request, &Err(Error::Invalid(error.to_string())))?;
                return Err(error);
            }
        };
        let cancel = Arc::new(AtomicBool::new(false));
        self.revision_jobs
            .lock()
            .expect("revision jobs mutex")
            .insert(conversation.to_owned(), cancel.clone());
        let core = self.clone();
        let owner = conversation.to_owned();
        let worker_request = request.clone();
        let worker = std::thread::Builder::new()
            .name("plan-revision".into())
            .spawn(move || {
                let outcome = core.review_and_apply_revision(
                    &owner,
                    &revision,
                    &baseline,
                    &specs,
                    &cancel,
                    &super::store::RevisionLease {
                        requirement_revision,
                        request: &worker_request,
                    },
                );
                if let Err(error) = core.store.finish_revision_job(&worker_request, &outcome) {
                    core.notice(
                        &owner,
                        &format!("could not record revision {worker_request} outcome: {error}"),
                    );
                }
                core.revision_jobs
                    .lock()
                    .expect("revision jobs mutex")
                    .remove(&owner);
                let mut text = match &outcome {
                    Ok(()) => format!("Plan revision {worker_request} applied."),
                    Err(error) => format!("Plan revision {worker_request} rejected: {error}"),
                };
                if outcome.is_ok()
                    && let Err(error) = core.maybe_continue_plan(&owner, &revision.plan)
                {
                    text.push_str(&format!(" Execution was not resumed: {error}"));
                }
                let _ = core.store.add_message(&owner, RUNTIME, &text);
                core.wake(&owner);
            });
        if let Err(error) = worker {
            self.revision_jobs
                .lock()
                .expect("revision jobs mutex")
                .remove(conversation);
            let error = Error::Io(error);
            self.store
                .finish_revision_job(&request, &Err(Error::Invalid(error.to_string())))?;
            return Err(error);
        }
        drop(registration);
        Ok(receipt)
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
        use rusqlite::OptionalExtension;
        let authorization: Option<(String, String)> = self.store.db().query_row(
            "SELECT scope,write_scope FROM plan_authorizations WHERE plan_id=?1 AND conversation_id=?2",
            rusqlite::params![plan, conversation],
            |r| Ok((r.get(0)?,r.get(1)?)),
        ).optional()?;
        let Some((allowed, write_allowed)) = authorization else {
            if self
                .store
                .latest_plan(conversation)?
                .first()
                .is_some_and(|task| task.plan_id == plan)
            {
                return Err(Error::Invalid(format!(
                    "plan {plan} is not accepted; correct the draft with submit_plan, not revise_plan; revise_plan only edits an accepted plan"
                )));
            }
            let current = self
                .store
                .latest_plan(conversation)?
                .first()
                .map(|task| task.plan_id.clone())
                .unwrap_or_default();
            return Err(Error::Invalid(format!(
                "plan {plan} is not the accepted plan (it was replaced or never accepted); revise the current plan {current} using its full ID"
            )));
        };
        let write_allowed: Vec<String> = serde_json::from_str(&write_allowed)?;
        let allowed: Vec<String> = serde_json::from_str(&allowed)?;
        validate_authorized_scope(specs, &allowed, &write_allowed)
    }

    fn review_and_apply_revision(
        self: &Arc<Self>,
        conversation: &str,
        revision: &PlanRevision,
        baseline: &[Task],
        specs: &[TaskContract],
        cancel: &AtomicBool,
        lease: &super::store::RevisionLease<'_>,
    ) -> Result<()> {
        let project = PathBuf::from(self.store.conversation(conversation)?.project);
        let previous: Vec<_> = baseline.iter().map(|task| task.spec.clone()).collect();
        let approved: Vec<String> = baseline
            .iter()
            .filter(|task| task.status == status::APPROVED)
            .filter_map(|task| task.spec["id"].as_str().map(str::to_owned))
            .filter(|id| !revision.redo.contains(id))
            .collect();
        let review = self
            .review(
                conversation,
                PLAN_VERIFIER,
                Some(&revision.plan),
                &project,
                cancel,
                |initial, request| {
                    tasks::plan_revision_review_prompt(
                        &project.to_string_lossy(),
                        request,
                        specs,
                        &previous,
                        &approved,
                        &revision.reason,
                        initial,
                    )
                },
            )?
            .map_err(Error::Invalid)?;
        if !review.approved
            && tasks::only_minor_findings(&review)
            && self.store.rejected_revisions_in_a_row(&revision.plan)?
                >= tasks::TOLERATED_PLAN_REJECTIONS
        {
            self.notice(
                conversation,
                &format!(
                    "revision applied with unresolved minor review findings after repeated rejections: {}",
                    tasks::findings_text(&review.findings)
                ),
            );
        } else {
            require(
                review.approved,
                &format!(
                    "revision review rejected: {}",
                    tasks::findings_text(&review.findings)
                ),
            )?;
        }
        require(!cancel.load(Ordering::SeqCst), "revision cancelled")?;
        let affected = affected_tasks(baseline, specs, &revision.redo);
        let barrier = {
            let _graph = self.graph_updates.lock().expect("graph update mutex");
            RevisionBarrier::new(self, affected.clone())
        };
        for id in &affected {
            if let Some((_, cancel)) = self.task_cancels.lock().expect("task cancel mutex").get(id)
            {
                cancel.store(true, Ordering::SeqCst);
            }
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while self
            .task_cancels
            .lock()
            .expect("task cancel mutex")
            .keys()
            .any(|id| affected.contains(id))
        {
            require(!cancel.load(Ordering::SeqCst), "revision cancelled")?;
            require(
                std::time::Instant::now() < deadline,
                "affected task did not stop; revision was not applied",
            )?;
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        {
            let _graph = self.graph_updates.lock().expect("graph update mutex");
            let _integration = self.integrating.lock().expect("integration mutex");
            self.store.apply_plan_revision(
                conversation,
                revision,
                baseline,
                specs,
                &affected,
                lease,
            )?;
        }
        drop(barrier);
        Ok(())
    }
}

fn validate_authorized_scope(
    specs: &[TaskContract],
    allowed: &[String],
    write_allowed: &[String],
) -> Result<()> {
    require(
        allowed.iter().any(|p| p == ".") || specs.iter().all(|s| !s.scope.is_empty()),
        "empty scope expands the approved scope",
    )?;
    for spec in specs.iter().filter(|s| s.writes) {
        require(
            !write_allowed.is_empty(),
            "revision adds write permission; user approval is required",
        )?;
        let scope = if spec.scope.is_empty() {
            vec![".".to_owned()]
        } else {
            spec.scope.clone()
        };
        require(
            scope.iter().all(|p| {
                Path::new(p).is_relative()
                    && !Path::new(p)
                        .components()
                        .any(|c| matches!(c, std::path::Component::ParentDir))
            }),
            "revision write scope must stay inside the project",
        )?;
        require(
            scope.iter().all(|p| {
                write_allowed.iter().any(|a| {
                    a == p
                        || (a == "." && Path::new(p).is_relative())
                        || Path::new(p).starts_with(a)
                })
            }),
            "revision expands approved write scope; user approval is required",
        )?;
    }
    for path in specs.iter().flat_map(|s| &s.scope) {
        let candidate = Path::new(path);
        require(
            allowed.contains(path)
                || !candidate
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir)),
            "revision scope contains parent traversal",
        )?;
        require(
            allowed.iter().any(|a| {
                a == path || (a == "." && candidate.is_relative()) || candidate.starts_with(a)
            }),
            "revision expands the approved scope; user approval is required",
        )?;
    }
    Ok(())
}

/// Tasks a revision sends back to work: those whose contract changes, those
/// it names in `redo`, and everything depending on them. An approved task
/// changed only in wording keeps its result, and so do its dependents.
fn affected_tasks(baseline: &[Task], specs: &[TaskContract], redo: &[String]) -> BTreeSet<String> {
    let changed: BTreeSet<_> = baseline
        .iter()
        .filter(|old| {
            let id = old.spec["id"].as_str().unwrap_or_default();
            let Some(new) = specs.iter().find(|s| s.id == id) else {
                return true;
            };
            if redo.iter().any(|r| r == id) {
                return true;
            }
            serde_json::from_value::<TaskContract>(old.spec.clone())
                .ok()
                .as_ref()
                != Some(new)
                && !(old.status == status::APPROVED && reworded_only(&old.spec, new))
        })
        .filter_map(|t| t.spec["id"].as_str().map(str::to_owned))
        .collect();
    baseline
        .iter()
        .filter(|t| {
            t.spec["id"].as_str().is_some_and(|id| {
                changed.contains(id)
                    || changed
                        .iter()
                        .any(|dep| tasks::depends_transitively(specs, id, dep))
            })
        })
        .map(|t| t.id.clone())
        .collect()
}

/// Whether a contract differs from the one an approved result was produced
/// for only in how it is worded: its title, goal, summaries, section or the
/// requirement ids the planner cited. Its kind, scope, permissions,
/// dependencies and bound requirements and criteria are what the work was.
fn reworded_only(old: &serde_json::Value, new: &TaskContract) -> bool {
    let Ok(old) = serde_json::from_value::<TaskContract>(old.clone()) else {
        return false;
    };
    let mut new = new.clone();
    new.title = old.title.clone();
    new.goal = old.goal.clone();
    new.summary = old.summary.clone();
    new.section = old.section.clone();
    new.verifier = old.verifier.clone();
    new.covers = old.covers.clone();
    new == old
}

struct RevisionBarrier<'a> {
    core: &'a Core,
    ids: BTreeSet<String>,
}
impl<'a> RevisionBarrier<'a> {
    fn new(core: &'a Core, ids: BTreeSet<String>) -> Self {
        core.revising
            .lock()
            .expect("revision mutex")
            .extend(ids.iter().cloned());
        Self { core, ids }
    }
}
impl Drop for RevisionBarrier<'_> {
    fn drop(&mut self) {
        self.core
            .revising
            .lock()
            .expect("revision mutex")
            .retain(|id| !self.ids.contains(id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
