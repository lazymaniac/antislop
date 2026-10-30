use super::*;
use std::collections::BTreeSet;

pub(super) struct ReviewContext<'a> {
    pub(super) conversation: &'a str,
    pub(super) revision: &'a PlanRevision,
    pub(super) baseline: &'a [Task],
    pub(super) specs: &'a [TaskContract],
    pub(super) cancel: &'a AtomicBool,
    pub(super) project: PathBuf,
}

struct ReviewData<'a> {
    context: &'a ReviewContext<'a>,
    approved: &'a ApprovedRevisionWork,
}

impl Core {
    pub(super) fn review_and_apply_revision(
        self: &Arc<Self>,
        context: ReviewContext<'_>,
        lease: &super::store::RevisionLease<'_>,
    ) -> Result<()> {
        let affected = affected_tasks(context.baseline, context.specs, &context.revision.redo);
        let approved = approved_work(context.baseline, &affected);
        self.review_revision(&ReviewData {
            context: &context,
            approved: &approved,
        })?;
        require(!context.cancel.load(Ordering::SeqCst), "revision cancelled")?;
        self.apply_after_review(&context, lease, affected)
    }

    fn review_revision(self: &Arc<Self>, data: &ReviewData<'_>) -> Result<()> {
        let context = data.context;
        let previous = previous_specs(context);
        let review = self
            .review(
                context.conversation,
                PLAN_VERIFIER,
                Some(&context.revision.plan),
                &context.project,
                context.cancel,
                |initial, request| {
                    tasks::plan_revision_review_prompt(
                        &context.project.to_string_lossy(),
                        request,
                        context.specs,
                        &previous,
                        tasks::ApprovedWork {
                            kept: &data.approved.kept,
                            redone: &data.approved.redone,
                        },
                        &context.revision.reason,
                        initial,
                    )
                },
            )?
            .map_err(Error::Invalid)?;
        self.require_revision_review(context.conversation, context.revision, &review)
    }

    fn require_revision_review(
        &self,
        conversation: &str,
        revision: &PlanRevision,
        review: &Verdict,
    ) -> Result<()> {
        if !review.approved
            && tasks::only_minor_findings(review)
            && self.store.rejected_revisions_in_a_row(&revision.plan)?
                >= tasks::TOLERATED_PLAN_REJECTIONS
        {
            self.notice(conversation, &format!(
                "revision applied with unresolved minor review findings after repeated rejections: {}",
                tasks::findings_text(&review.findings)));
            return Ok(());
        }
        require(
            review.approved,
            &format!(
                "revision review rejected: {}",
                tasks::findings_text(&review.findings)
            ),
        )
    }

    fn apply_after_review(
        &self,
        context: &ReviewContext<'_>,
        lease: &super::store::RevisionLease<'_>,
        affected: BTreeSet<String>,
    ) -> Result<()> {
        let barrier = {
            let _graph = self.graph_updates.lock().expect("graph update mutex");
            RevisionBarrier::new(self, affected.clone())
        };
        self.cancel_affected_tasks(&affected);
        self.wait_for_affected_tasks(&affected, context.cancel)?;
        let _graph = self.graph_updates.lock().expect("graph update mutex");
        let _integration = self.integrating.lock().expect("integration mutex");
        self.store.apply_plan_revision(
            context.conversation,
            context.revision,
            context.baseline,
            context.specs,
            &affected,
            lease,
        )?;
        drop(barrier);
        Ok(())
    }

    fn cancel_affected_tasks(&self, affected: &BTreeSet<String>) {
        let tasks = self.task_cancels.lock().expect("task cancel mutex");
        for id in affected {
            if let Some((_, cancel)) = tasks.get(id) {
                cancel.store(true, Ordering::SeqCst);
            }
        }
    }

    fn wait_for_affected_tasks(
        &self,
        affected: &BTreeSet<String>,
        cancel: &AtomicBool,
    ) -> Result<()> {
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
        Ok(())
    }
}

struct ApprovedRevisionWork {
    kept: Vec<String>,
    redone: Vec<String>,
}

fn approved_work(baseline: &[Task], affected: &BTreeSet<String>) -> ApprovedRevisionWork {
    let (redone, kept) = baseline
        .iter()
        .filter(|task| task.status == status::APPROVED)
        .filter_map(|task| task.spec["id"].as_str().map(str::to_owned))
        .partition(|id| affected.contains(id));
    ApprovedRevisionWork { kept, redone }
}

/// Tasks a revision sends back to work, plus their dependents.
pub(super) fn affected_tasks(
    baseline: &[Task],
    specs: &[TaskContract],
    redo: &[String],
) -> BTreeSet<String> {
    let changed = changed_tasks(baseline, specs, redo);
    baseline
        .iter()
        .filter(|task| {
            task.spec["id"].as_str().is_some_and(|id| {
                changed.contains(id)
                    || changed
                        .iter()
                        .any(|dep| tasks::depends_transitively(specs, id, dep))
            })
        })
        .map(|task| task.id.clone())
        .collect()
}

fn changed_tasks(baseline: &[Task], specs: &[TaskContract], redo: &[String]) -> BTreeSet<String> {
    baseline
        .iter()
        .filter(|old| contract_changed(old, specs, redo))
        .filter_map(|task| task.spec["id"].as_str().map(str::to_owned))
        .collect()
}

fn contract_changed(old: &Task, specs: &[TaskContract], redo: &[String]) -> bool {
    let id = old.spec["id"].as_str().unwrap_or_default();
    let Some(new) = specs.iter().find(|spec| spec.id == id) else {
        return true;
    };
    if redo.iter().any(|requested| requested == id) {
        return true;
    }
    serde_json::from_value::<TaskContract>(old.spec.clone())
        .ok()
        .as_ref()
        != Some(new)
        && !(old.status == status::APPROVED && reworded_only(&old.spec, new))
}

/// Approved work survives wording changes, but not changes to its contract.
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

fn previous_specs(context: &ReviewContext<'_>) -> Vec<serde_json::Value> {
    context
        .baseline
        .iter()
        .map(|task| task.spec.clone())
        .collect()
}
