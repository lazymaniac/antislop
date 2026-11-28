use super::*;
use crate::lean::agent_tools::turn_inputs::ReadFor;
use crate::lean::prompts;
use serde_json::Value;
use std::collections::BTreeSet;

pub(super) struct ReviewContext<'a> {
    pub(super) conversation: &'a str,
    pub(super) revision: &'a PlanRevision,
    pub(super) baseline: &'a [Task],
    pub(super) specs: &'a [TaskContract],
    pub(super) cancel: &'a AtomicBool,
    pub(super) project: PathBuf,
}

pub(super) enum DirectRevision<'a> {
    Reviewed(&'a super::store::RevisionLease<'a>),
    Rigor(tasks::Team, &'a str),
}

struct ReviewData<'a> {
    context: &'a ReviewContext<'a>,
    approved: &'a ApprovedRevisionWork,
    request: &'a str,
    checkpoints: &'a [Value],
}

impl Core {
    pub(super) fn review_and_apply_revision(
        self: &Arc<Self>,
        context: ReviewContext<'_>,
        lease: &super::store::RevisionLease<'_>,
    ) -> Result<()> {
        let affected = affected_tasks(context.baseline, context.specs, &context.revision.redo);
        let approved = approved_work(context.baseline, &affected);
        let checkpoints = self
            .store
            .revision_checkpoints(context.conversation, context.revision)?;
        // Held from the review on: a task the revision changes would start on
        // the contract it is about to lose and be cancelled when it applies.
        let _barrier = {
            let _graph = self.locks.graph();
            RevisionBarrier::new(self, affected.clone())
        };
        self.review_revision(&ReviewData {
            context: &context,
            approved: &approved,
            request: lease.request,
            checkpoints: &checkpoints,
        })?;
        require(!context.cancel.load(Ordering::SeqCst), "revision cancelled")?;
        if context.revision.scope.is_some() {
            let interrupted = self.cancel_affected_tasks(&affected);
            self.wait_for_affected_tasks(&affected, context.cancel)?;
            self.checkpoint_revision_work(&affected)?;
            return self.stage_scope_revision(
                context.conversation,
                context.revision,
                context.specs,
                &affected,
                &interrupted,
                lease.request,
            );
        }
        self.apply_after_review(&context, lease, affected, &checkpoints)
    }

    fn review_revision(self: &Arc<Self>, data: &ReviewData<'_>) -> Result<()> {
        let context = data.context;
        let previous = previous_specs(context);
        let affected = affected_tasks(context.baseline, context.specs, &context.revision.redo);
        let held: Vec<_> = context.baseline.iter()
            .filter(|task| task.status == status::COORDINATING && !affected.contains(&task.id))
            .map(|task| json!({"task":task.spec["id"],"runtime_id":task.id,"status":task.status,"input_generation":task.input_generation}))
            .collect();
        let reason = format!(
            prompts::template!(REVISION_CHECKPOINT_CONTEXT),
            context.revision.reason,
            crate::lean::prompt_context::markdown(&checkpoint_review_context(
                context.revision,
                context.baseline,
                data.checkpoints
            )),
        );
        let reason = format!(
            prompts::template!(REVISION_COORDINATING_CONTEXT),
            crate::lean::prompt_context::untrusted(
                prompts::PRESERVED_COORDINATION_LABEL,
                &crate::lean::prompt_context::markdown(&held)
            ),
            reason = reason
        );
        let scope_context =
            self.plan_scope_context(context.conversation, &context.revision.plan)?;
        let team_guidance = self.store.team_guidance(context.conversation)?;
        let review = self
            .review(
                context.conversation,
                PLAN_REVIEWER,
                Some(&context.revision.plan),
                &context.project,
                context.cancel,
                |initial, request, inputs| {
                    let previous_scope = inputs.block("Previous accepted scope", &scope_context, ReadFor::Review);
                    let candidate_scope = inputs.block("Candidate scope", context.revision.scope.as_deref().unwrap_or("unchanged"), ReadFor::Review);
                    let reason = format!("Revision {} for plan {} at base {}. {reason}\n{previous_scope}\n{candidate_scope}\nReassess preserved results against the candidate scope.", data.request, context.revision.plan, context.revision.base_revision);

                    let mut prompt = tasks::render_revision_review(
                        &context.project.to_string_lossy(),
                        request,
                        context.specs,
                        &previous,
                        tasks::ApprovedWork {
                            kept: &data.approved.kept,
                            redone: &data.approved.redone,
                        },
                        &reason,
                        initial,
                        inputs,
                    );
                    if let Some(guidance) = &team_guidance {
                        guidance.write_guidance_section(&mut prompt);
                    } else {
                        prompt.push_str(tasks::FEATURE_PLANNING_RULES);
                    }
                    prompt
                },
            )?
            .map_err(Error::Invalid)?;
        self.require_revision_review(context.revision, &review)
    }

    fn require_revision_review(&self, revision: &PlanRevision, review: &Verdict) -> Result<()> {
        if review.approved {
            return Ok(());
        }
        let previous = self.store.rejected_revisions_in_a_row(&revision.plan)?;
        Err(Error::Invalid(format!(
            "revision review rejected: {} ({} prior rejected reviews; resolve the findings before resubmitting)",
            tasks::findings_text(&review.findings),
            previous,
        )))
    }

    fn apply_after_review(
        &self,
        context: &ReviewContext<'_>,
        lease: &super::store::RevisionLease<'_>,
        affected: BTreeSet<String>,
        checkpoints: &[Value],
    ) -> Result<()> {
        self.cancel_affected_tasks(&affected);
        self.wait_for_affected_tasks(&affected, context.cancel)?;
        let source = self.locks.source(context.conversation);
        let _source = source.lock().expect("conversation source scope");
        require(!context.cancel.load(Ordering::SeqCst), "revision cancelled")?;
        require(
            self.store
                .revision_checkpoints(context.conversation, context.revision)?
                == checkpoints,
            "checkpoint source changed during revision review; review the new source before adoption",
        )?;
        self.checkpoint_revision_work(&affected)?;
        let _graph = self.locks.graph();
        let _integration = self.locks.integration();
        require(!context.cancel.load(Ordering::SeqCst), "revision cancelled")?;
        self.store.apply_plan_revision(
            context.conversation,
            context.revision,
            context.baseline,
            context.specs,
            &affected,
            lease,
        )?;
        for id in affected {
            self.queue_worktree_cleanup(&id);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply_direct_revision(
        &self,
        conversation: &str,
        revision: &PlanRevision,
        baseline: &[Task],
        specs: &[TaskContract],
        change: DirectRevision<'_>,

        is_accepted: bool,
    ) -> Result<()> {
        let operation = self.operation(conversation)?;
        let affected = affected_tasks(baseline, specs, &revision.redo);
        let cancel = &operation.cancel;
        let _barrier = {
            let _graph = self.locks.graph();
            RevisionBarrier::new(self, affected.clone())
        };
        if is_accepted {
            self.cancel_affected_tasks(&affected);
            self.wait_for_affected_tasks(&affected, cancel)?;
        }
        let source = self.locks.source(conversation);
        let _source = source.lock().expect("conversation source scope");
        require(!cancel.load(Ordering::SeqCst), "revision cancelled")?;
        if is_accepted {
            self.checkpoint_revision_work(&affected)?;
        }
        let _graph = self.locks.graph();
        let _integration = self.locks.integration();
        require(!cancel.load(Ordering::SeqCst), "revision cancelled")?;
        match change {
            DirectRevision::Reviewed(lease) => self.store.apply_plan_revision_with_options(
                conversation,
                revision,
                baseline,
                specs,
                &affected,
                lease,
                is_accepted,
            )?,
            DirectRevision::Rigor(team, reason) => self.store.apply_plan_rigor(
                conversation,
                revision,
                baseline,
                specs,
                &affected,
                (team, reason),
                is_accepted,
            )?,
        }
        for id in affected {
            self.queue_worktree_cleanup(&id);
        }
        Ok(())
    }

    fn checkpoint_revision_work(&self, affected: &BTreeSet<String>) -> Result<()> {
        for id in affected {
            let Some(mut task) = self.store.task_by_id(id)? else {
                continue;
            };
            let spec: TaskContract = serde_json::from_value(task.spec.clone())?;
            let path = crate::lean::core::worktree::paths::worktree_path(&self.state, &task)?;
            if spec.change_kind.has_delivery() && path.exists() {
                if task.base.is_none() {
                    task.base = Some(git::head(&path)?);
                }
                task.result = Some(if spec.change_kind() == tasks::ChangeKind::Artifact {
                    git::commit_artifacts(
                        &path,
                        &spec.outputs,
                        git::CommitPurpose::ArtifactCheckpoint,
                        false,
                    )?
                } else {
                    git::commit_all(&path, git::CommitPurpose::Checkpoint)?
                });
                self.store.update_task(&task)?;
            }
        }
        Ok(())
    }

    fn cancel_affected_tasks(&self, affected: &BTreeSet<String>) -> BTreeSet<String> {
        let tasks = self
            .activity
            .task_cancels
            .lock()
            .expect("task cancel mutex");
        let mut interrupted = BTreeSet::new();
        for id in affected {
            if let Some((_, cancel)) = tasks.get(id)
                && !cancel.swap(true, Ordering::SeqCst)
            {
                interrupted.insert(id.clone());
            }
        }
        interrupted
    }

    fn wait_for_affected_tasks(
        &self,
        affected: &BTreeSet<String>,
        cancel: &AtomicBool,
    ) -> Result<()> {
        while self
            .activity
            .task_cancels
            .lock()
            .expect("task cancel mutex")
            .keys()
            .any(|id| affected.contains(id))
        {
            require(!cancel.load(Ordering::SeqCst), "revision cancelled")?;
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
    let (redone, kept): (Vec<_>, Vec<_>) = baseline
        .iter()
        .filter(|task| task.status == status::APPROVED)
        .partition(|task| affected.contains(&task.id));
    let names = |tasks: Vec<&Task>| {
        tasks
            .into_iter()
            .filter_map(|task| task.spec["id"].as_str().map(str::to_owned))
            .collect()
    };
    let redone = names(redone);
    let kept = names(kept);
    ApprovedRevisionWork { kept, redone }
}

/// Tasks a revision sends back to work, plus their dependents.
pub(super) fn affected_tasks(
    baseline: &[Task],
    specs: &[TaskContract],
    redo: &[String],
) -> BTreeSet<String> {
    let mut changed = changed_tasks(baseline, specs, redo);
    // Options no longer embed owner contracts. Semantic owner changes still
    // invalidate their selection before any consumer may use it again.
    for task in baseline {
        if let Some(owner) = task.spec["options_for"].as_str()
            && changed.contains(owner)
            && let Some(id) = task.spec["id"].as_str()
        {
            changed.insert(id.to_owned());
        }
    }
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
    !serde_json::from_value::<TaskContract>(old.spec.clone())
        .is_ok_and(|old| tasks::same_contract(&old, new))
        && !reworded_only(&old.spec, new)
}

/// Approved work survives wording changes, but not changes to its contract.
fn reworded_only(old: &serde_json::Value, new: &TaskContract) -> bool {
    let Ok(old) = serde_json::from_value::<TaskContract>(old.clone()) else {
        return false;
    };
    let mut new = new.clone();
    if old.options_for.is_some()
        && old.options_for == new.options_for
        && options_goal_differs_only_by_owner_wording(&old.goal, &new.goal)
    {
        new.goal = old.goal.clone();
    }
    new.title = old.title.clone();
    new.summary = old.summary.clone();
    new.section = old.section.clone();
    new.reviewer = old.reviewer.clone();
    tasks::same_contract(&new, &old)
}

fn options_goal_differs_only_by_owner_wording(old: &str, new: &str) -> bool {
    const MARKER: &str = prompts::OPTIONS_OBJECTIVE_MARKER;
    let Some((old_prefix, old_json)) = old.split_once(MARKER) else {
        return false;
    };
    let Some((new_prefix, new_json)) = new.split_once(MARKER) else {
        return false;
    };
    if old_prefix != new_prefix {
        return false;
    }
    let (Ok(mut old), Ok(mut new)) = (
        serde_json::from_str::<serde_json::Value>(old_json),
        serde_json::from_str::<serde_json::Value>(new_json),
    ) else {
        return false;
    };
    let (Some(old_owner), Some(new_owner)) = (
        old.get_mut("owner_contract")
            .and_then(serde_json::Value::as_object_mut),
        new.get_mut("owner_contract")
            .and_then(serde_json::Value::as_object_mut),
    ) else {
        return false;
    };
    for field in ["title", "summary", "section", "reviewer"] {
        old_owner.remove(field);
        new_owner.remove(field);
    }
    old == new
}

struct RevisionBarrier<'a> {
    core: &'a Core,
    ids: BTreeSet<String>,
}

impl<'a> RevisionBarrier<'a> {
    fn new(core: &'a Core, ids: BTreeSet<String>) -> Self {
        core.revisions
            .revising
            .lock()
            .expect("revision mutex")
            .extend(ids.iter().cloned());
        Self { core, ids }
    }
}

impl Drop for RevisionBarrier<'_> {
    fn drop(&mut self) {
        self.core
            .revisions
            .revising
            .lock()
            .expect("revision mutex")
            .retain(|id| !self.ids.contains(id));
    }
}

fn checkpoint_review_context(
    revision: &PlanRevision,
    baseline: &[Task],
    transfers: &[Value],
) -> Value {
    let existing = baseline
        .iter()
        .filter_map(|task| {
            let (base, result) = task.base.as_ref().zip(task.result.as_ref())?;
            (base != result).then(|| {
                serde_json::json!({
                    "task":task.spec["id"], "runtime_id":task.id, "status":task.status,
                    "base":base, "result":result,
                    "retired":revision.retire.iter().any(|id| task.spec["id"] == *id),
                })
            })
        })
        .collect::<Vec<_>>();
    serde_json::json!({"new_transfers":transfers, "replacements":revision.replacements, "existing_checkpoints":existing})
}

fn previous_specs(context: &ReviewContext<'_>) -> Vec<serde_json::Value> {
    context
        .baseline
        .iter()
        .map(|task| task.spec.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_reworded_recovery_keeps_its_existing_checkpoint_visible_to_review() {
        let task: Task = serde_json::from_value(json!({
            "id":"runtime-writer", "plan_id":"plan", "spec":{"id":"recover"},
            "status":"queued", "round":1, "base":"old", "result":"saved-code",
            "report":"A retained report is not a replacement for this code"
        }))
        .unwrap();
        let revision: PlanRevision = serde_json::from_value(json!({
            "plan":"plan", "base_revision":1, "reason":"Clarify assessment", "tasks":[], "retire":[]
        }))
        .unwrap();
        let context = checkpoint_review_context(&revision, std::slice::from_ref(&task), &[]);
        assert_eq!(context["new_transfers"], json!([]));
        assert_eq!(context["existing_checkpoints"][0]["result"], "saved-code");
        assert_eq!(context["existing_checkpoints"][0]["retired"], false);
        assert!(!context.to_string().contains("A retained report"));
        let mut unchanged = task;
        unchanged.result = unchanged.base.clone();
        assert_eq!(
            checkpoint_review_context(&revision, &[unchanged], &[])["existing_checkpoints"],
            json!([])
        );
    }

    #[test]
    fn repeated_review_rejections_never_authorize_a_revision() {
        let directory = tempfile::tempdir().unwrap();
        let config =
            crate::config::Config::from_toml(include_str!("../../../../../examples/antislop.toml"))
                .unwrap();
        let core = Core::open(&directory.path().join("state"), config).unwrap();
        let conversation = core.store.create_conversation("fixture").unwrap();
        let task = core
            .store
            .add_plan(
                &conversation.id,
                &[json!({"id":"analysis","title":"Analysis","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Report","change_kind": "none", "kind":"analysis"})],
            )
            .unwrap()
            .remove(0);
        let revision = PlanRevision {
            scope: None,
            plan: task.plan_id.clone(),
            base_revision: 0,
            reason: "Repair findings".into(),
            tasks: vec![],
            retire: vec![],
            redo: vec![],
            replacements: Default::default(),
        };
        for attempt in 0..5 {
            for severity in ["low", "medium", "high", "critical"] {
                let review = Verdict {
                    approved: false,
                    resolved: vec![],
                    resolution_evidence: Default::default(),
                    findings: vec![
                        json!({"severity":severity,"issue":"Explicit redo is missing","where":"revision"}),
                    ],
                    coverage: serde_json::Map::new(),
                };
                let error = core
                    .require_revision_review(&revision, &review)
                    .unwrap_err()
                    .to_string();
                assert!(error.contains("Explicit redo is missing"), "{error}");
                assert!(
                    error.contains(&format!("{attempt} prior rejected reviews")),
                    "{error}"
                );
            }
            core.store.db().execute("INSERT INTO plan_revision_jobs(id,conversation_id,plan_id,request_key,proposal,status,error) VALUES(?1,?2,?3,?4,'{}','rejected','revision review rejected: Explicit redo is missing')",rusqlite::params![crate::id(),conversation.id,task.plan_id,format!("attempt-{attempt}")]).unwrap();
        }
        assert_eq!(core.store.plan_revision(&task.plan_id).unwrap(), 0);
        let approved = Verdict {
            approved: true,
            resolved: vec![],
            resolution_evidence: Default::default(),
            findings: vec![],
            coverage: serde_json::Map::new(),
        };
        core.require_revision_review(&revision, &approved).unwrap();
    }
    #[test]
    fn revision_checkpoints_unreported_work_without_changing_the_project() {
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
        std::fs::write(project.join("source"), "original").unwrap();
        let base = git::commit_all(&project, git::CommitPurpose::TestFixture).unwrap();
        let index = std::fs::read(project.join(".git/index")).unwrap();
        let config =
            crate::config::Config::from_toml(include_str!("../../../../../examples/antislop.toml"))
                .unwrap();
        let core = Core::open(&directory.path().join("state"), config).unwrap();
        let conversation = core
            .store
            .create_conversation(project.to_str().unwrap())
            .unwrap();
        let task=core.store.add_plan(&conversation.id,&[json!({"id":"work","title":"Work","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Change source","change_kind": "code", "kind":"implementation"})]).unwrap().remove(0);
        let path = crate::lean::core::worktree::paths::worktree_path(&core.state, &task).unwrap();
        let branch =
            crate::lean::core::worktree::paths::worktree_branch(&core.state, &task).unwrap();
        git::add_worktree(&project, &path, &branch, &base).unwrap();
        std::fs::write(path.join("source"), "unfinished changes").unwrap();
        core.checkpoint_revision_work(&BTreeSet::from([task.id.clone()]))
            .unwrap();
        let saved = core.store.task_by_id(&task.id).unwrap().unwrap();
        assert_eq!(saved.base.as_deref(), Some(base.as_str()));
        assert_eq!(
            git::changed_files(&project, &base, saved.result.as_ref().unwrap()).unwrap(),
            ["source"]
        );
        assert_eq!(
            std::fs::read_to_string(project.join("source")).unwrap(),
            "original"
        );
        assert_eq!(git::head(&project).unwrap(), base);
        assert_eq!(std::fs::read(project.join(".git/index")).unwrap(), index);
    }
}
