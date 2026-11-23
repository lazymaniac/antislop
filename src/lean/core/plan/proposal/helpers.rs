use super::*;

#[path = "helpers/recording.rs"]
mod recording;
use recording::*;

#[path = "helpers/review.rs"]
mod review;

impl Core {
    pub(super) fn prepare_plan(
        &self,
        conversation: &str,
        specs: &[TaskContract],
        scope: Option<&str>,
        keep: &[String],
    ) -> Result<PreparedPlan> {
        let inherited = self
            .store
            .latest_plan(conversation)?
            .first()
            .map(|task| self.store.plan_scope_id(&task.plan_id))
            .transpose()?
            .flatten();
        let scope_id = match (scope, inherited) {
            (Some(summary), Some(id))
                if self.store.scope(conversation, &id)?.summary == summary
                    && self.store.scope_documents_current(conversation, &id)? =>
            {
                id
            }
            (Some(summary), _) => self.store.create_scope(conversation, summary)?.id,
            (None, Some(id))
                if self.store.scope(conversation, &id)?.summary
                    == self.store.workflow(conversation)?.summary
                    && self.store.scope_documents_current(conversation, &id)? =>
            {
                id
            }
            (None, Some(_)) => {
                self.store
                    .create_scope(conversation, &self.store.workflow(conversation)?.summary)?
                    .id
            }
            (None, None) => {
                self.store
                    .create_scope(conversation, &self.store.workflow(conversation)?.summary)?
                    .id
            }
        };
        let (tasks, id) = self.persist_proposed_tasks(conversation, specs)?;
        self.store.bind_plan_scope(conversation, &id, &scope_id)?;
        self.store.retain_plan_tasks(conversation, &id, keep)?;
        Ok(PreparedPlan {
            id,
            tasks,
            answered: self.store.last_orchestration_answered(conversation)?,
        })
    }

    fn persist_proposed_tasks(
        &self,
        conversation: &str,
        specs: &[TaskContract],
    ) -> Result<(Vec<Task>, String)> {
        let values = specs
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let tasks = self.store.add_plan(conversation, &values)?;
        let plan_id = tasks[0].plan_id.clone();
        Ok((tasks, plan_id))
    }

    pub(super) fn publish_plan(
        self: &Arc<Self>,
        conversation: &str,
        text: &str,
        addressed: Option<&[u64]>,
        plan: &PreparedPlan,
        review: &std::result::Result<Verdict, String>,
    ) -> Result<()> {
        // Review runs outside the graph lock. An accepted plan may have
        // started since this draft was prepared; never publish it over workers.
        if let Err(error) = self.ensure_replacement_plan_allowed(conversation) {
            self.cancel_proposed_plan(&plan.id)?;
            return Err(error);
        }
        // Rigor can change while review is running. The authored plan remains
        // the same; publish its current generated checks instead of old rows.
        let tasks = self.store.plan(&plan.id)?;
        if tasks.iter().any(|task| task.status != status::PROPOSED) {
            return Ok(());
        }
        let specs = tasks
            .iter()
            .map(|task| serde_json::from_value(task.spec.clone()))
            .collect::<std::result::Result<Vec<TaskContract>, _>>()?;
        let plan = &PreparedPlan {
            id: plan.id.clone(),
            tasks,
            answered: plan.answered,
        };
        let checked = review.as_ref().is_ok_and(|verdict| verdict.approved);
        self.resolve_plan_comments(conversation, addressed, plan, checked)?;
        let waiting = (!checked).then_some("the plan reviewer did not approve this plan");
        let revision_token = self.plan_revision_token(conversation, &plan.id)?;
        let scope_id = self
            .store
            .plan_scope_id(&plan.id)?
            .ok_or_else(|| Error::Invalid("plan has no scope".into()))?;
        let scope = self.store.scope(conversation, &scope_id)?;
        let previous = self
            .store
            .accepted_plan(conversation)?
            .map(|id| self.store.plan_scope_id(&id))
            .transpose()?
            .flatten()
            .map(|id| self.store.scope(conversation, &id))
            .transpose()?;
        let document = match previous.as_ref().filter(|old| old.id != scope.id) {
            Some(old) => format!(
                "# Scope change\n\n## Previously accepted\n{}\nOriginal documents: {}\n\n## Proposed scope\n{}\nOriginal documents: {}\n\n{}",
                old.summary,
                old.source_ids.len(),
                scope.summary,
                scope.source_ids.len(),
                text
            ),
            None => format!("# Scope\n\n{}\n\n{}", scope.summary, text),
        };
        let mut payload =
            plan_event_payload(&document, plan, &specs, review, waiting, revision_token)?;
        payload["retained_tasks"] = self.store.retained_plan_preview(conversation, &plan.id)?;
        payload["base_revision"] = json!(self.store.plan_revision(&plan.id)?);
        payload["scope"] = json!(scope);
        payload["previous_scope"] = json!(previous);
        self.store.append_event(
            conversation,
            None,
            Some(&plan.id),
            "plan_proposed",
            &payload,
        )?;
        Ok(())
    }

    fn resolve_plan_comments(
        &self,
        conversation: &str,
        addressed: Option<&[u64]>,
        plan: &PreparedPlan,
        checked: bool,
    ) -> Result<()> {
        if !checked {
            return Ok(());
        }
        let pending = self.store.pending_plan_comment_ids(conversation)?;
        self.store.resolve_listed_plan_comments(
            conversation,
            plan.answered,
            addressed.or(pending.as_deref()),
        )?;
        Ok(())
    }
}
pub(super) struct PreparedPlan {
    pub(super) id: String,
    tasks: Vec<Task>,
    answered: u64,
}

pub(super) fn record_addressed_comments(
    store: &Store,
    conversation: &str,
    addressed: Option<&[u64]>,
) -> Result<()> {
    if let Some(ids) = addressed {
        store.append_event(
            conversation,
            None,
            None,
            "plan_comments_addressed",
            &json!({"ids": ids, "upto_message": store.last_orchestration_answered(conversation)?}),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_plan_rejects_replacement_tools_and_late_publication_without_changing_workers() {
        let dir = tempfile::tempdir().unwrap();
        let config =
            Config::from_toml(include_str!("../../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&dir.path().join("state"), config.clone()).unwrap();
        let conversation = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        core.store
            .db()
            .execute(
                "UPDATE session_workflow SET phase='planning' WHERE conversation_id=?1",
                [&conversation],
            )
            .unwrap();
        core.store
            .select_team(&conversation, tasks::Team::NoQualityGates, "Report only")
            .unwrap();
        let task = json!({"id":"analysis","title":"Analysis","goal":"Inspect the accepted scope","kind":"analysis","change_kind":"none","acceptance_criteria":[{"id":"C1","text":"Report the observed behavior"}]});
        let specs = vec![serde_json::from_value::<TaskContract>(task.clone()).unwrap()];
        let accepted = core
            .prepare_plan(&conversation, &specs, Some("Accepted scope"), &[])
            .unwrap();
        core.store.queue_plan(&conversation, &accepted.id).unwrap();
        let mut worker = core.store.plan(&accepted.id).unwrap().remove(0);
        worker.status = status::WORKING.into();
        worker.round = 2;
        worker.report = Some("Preserved investigation".into());
        core.store.update_task(&worker).unwrap();

        // A draft can have entered review before execution starts.
        let mut writer = specs[0].clone();
        writer.kind = WorkKind::Implementation;
        writer.change_kind = tasks::ChangeKind::Code;
        let draft = core
            .prepare_plan(
                &conversation,
                &[writer],
                Some("Expanded backend scope"),
                &[],
            )
            .unwrap();
        let running = Arc::new(AtomicBool::new(false));
        core.activity
            .plans
            .lock()
            .unwrap()
            .insert(conversation.clone(), running.clone());
        let turn = core
            .store
            .start_turn(
                &conversation,
                ORCHESTRATOR,
                "fixture",
                0,
                Some(&accepted.id),
            )
            .unwrap();
        for (tool, args) in [
            ("request_replan", json!({"reason":"Add backend ownership"})),
            (
                "submit_plan",
                json!({"document":"Expanded scope","tasks":[task]}),
            ),
        ] {
            let error = core
                .agent_tool(&conversation, ORCHESTRATOR, &turn.id, tool, &args)
                .unwrap_err();
            assert!(error.to_string().contains("revise_plan"), "{tool}: {error}");
        }
        let error = core
            .propose(
                &conversation,
                dir.path(),
                "Expanded scope",
                PlanProposal {
                    keep: &[],
                    config: &config,
                    specs: &specs,
                    scope: None,
                },
                None,
                &AtomicBool::new(false),
            )
            .unwrap_err();
        assert!(error.to_string().contains("revise_plan"), "{error}");
        assert!(
            core.select_rigor(&conversation, tasks::Team::Feature, "Add specialists")
                .unwrap_err()
                .to_string()
                .contains("revise_plan")
        );
        assert_eq!(
            core.store.team(&conversation).unwrap(),
            tasks::Team::NoQualityGates
        );
        let review = serde_json::from_value(json!({"approved":true})).unwrap();
        let error = core
            .publish_plan(&conversation, "Expanded scope", None, &draft, &Ok(review))
            .unwrap_err();
        assert!(error.to_string().contains("revise_plan"), "{error}");
        assert!(
            core.store
                .plan(&draft.id)
                .unwrap()
                .iter()
                .all(|t| t.status == status::CANCELLED)
        );
        assert_eq!(
            core.store.accepted_plan(&conversation).unwrap(),
            Some(accepted.id)
        );
        let unchanged = core.store.task_by_id(&worker.id).unwrap().unwrap();
        assert_eq!(unchanged.spec, worker.spec);
        assert_eq!(unchanged.status, worker.status);
        assert_eq!(unchanged.round, worker.round);
        assert_eq!(unchanged.report, worker.report);
        assert!(!running.load(Ordering::SeqCst));
        assert!(
            core.store
                .latest_role_result(&conversation, ORCHESTRATOR)
                .unwrap()
                .is_none()
        );
        assert!(
            core.store
                .messages_by(&conversation, REPLAN, 0)
                .unwrap()
                .is_empty()
        );
        assert!(
            !core
                .store
                .events_since(&conversation, 0, 1000)
                .unwrap()
                .iter()
                .any(|e| e.kind == "plan_proposed")
        );

        core.activity.plans.lock().unwrap().remove(&conversation);
        assert_eq!(
            core.request_replan(&conversation, "Add backend ownership")
                .unwrap()["state"],
            "requested"
        );
    }

    #[test]
    fn publication_uses_the_new_rigor_when_selection_changes_during_review() {
        let dir = tempfile::tempdir().unwrap();
        let config =
            Config::from_toml(include_str!("../../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&dir.path().join("state"), config.clone()).unwrap();
        let conversation = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        let writer: TaskContract = serde_json::from_value(json!({
            "id":"writer", "title":"Contained fix", "goal":"Fix the contained behavior", "kind":"implementation", "change_kind":"code",
            "acceptance_criteria":[{"id":"C1","text":"The requested behavior works"}]
        })).unwrap();
        let specs = checked_team_plan(
            &tasks::feature_fixture(vec![writer.clone()]),
            &config,
            tasks::Team::Feature,
        )
        .unwrap();
        let original = core
            .prepare_plan(
                &conversation,
                &specs,
                Some("Fix the contained behavior"),
                &[],
            )
            .unwrap();
        let authored_id = original
            .tasks
            .iter()
            .find(|task| task.spec["id"] == "writer")
            .unwrap()
            .id
            .clone();
        core.store
            .select_planned_team(
                &conversation,
                tasks::Team::Feature,
                "Initial recommendation",
            )
            .unwrap();

        // The reviewer retains its original snapshot while the user picks a
        // different set of generated checks. Publication must use current rows.
        core.select_rigor(&conversation, tasks::Team::SmallChange, "Contained fix")
            .unwrap();
        let review: Verdict = serde_json::from_value(json!({"approved":true})).unwrap();
        core.publish_plan(
            &conversation,
            "# Contained fix",
            None,
            &original,
            &Ok(review),
        )
        .unwrap();
        let current = core.store.plan(&original.id).unwrap();
        assert!(current.iter().any(|task| task.id == authored_id));
        let events = core.store.events_since(&conversation, 0, 1000).unwrap();
        let proposal = events
            .iter()
            .find(|event| event.kind == "plan_proposed")
            .unwrap();
        assert_eq!(proposal.payload["plan"], original.id);
        assert_eq!(
            proposal.payload["tasks"],
            json!(current.iter().map(|task| &task.spec).collect::<Vec<_>>())
        );
        assert_eq!(proposal.payload["review"]["approved"], true);
        let mut view = crate::lean::tui::view::View::default();
        for event in &events {
            view.apply(&serde_json::to_value(event).unwrap());
        }
        assert_eq!(view.team, Some(tasks::Team::SmallChange));
        assert_eq!(
            view.plan_head_revision,
            core.store.plan_revision(&original.id).unwrap()
        );
        assert_eq!(
            view.tasks
                .iter()
                .map(|task| task.id.as_str())
                .collect::<Vec<_>>(),
            current
                .iter()
                .map(|task| task.spec["id"].as_str().unwrap())
                .collect::<Vec<_>>()
        );
        assert!(
            !events
                .iter()
                .any(|event| event.kind == "plan_rejected" || event.kind == "turn_started")
        );
        assert!(core.is_idle(&conversation));
    }
}

#[cfg(test)]
mod retention_tests {
    use super::*;
    use crate::lean::tasks::PlannedTask;

    #[test]
    fn planner_keep_preserves_completed_work_in_preview_and_after_acceptance() {
        let dir = tempfile::tempdir().unwrap();
        let config =
            Config::from_toml(include_str!("../../../../../examples/antislop.toml")).unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir(&project).unwrap();
        crate::lean::git::ok(&project, &["init", "-q"]).unwrap();
        std::fs::write(project.join("input.txt"), "Original behavior").unwrap();
        crate::lean::git::commit_all(&project, crate::lean::git::CommitPurpose::TestFixture)
            .unwrap();
        let revision = crate::lean::git::head(&project).unwrap();
        let core = Core::open(&dir.path().join("state"), config.clone()).unwrap();
        let conversation = core
            .store
            .create_conversation(project.to_str().unwrap())
            .unwrap()
            .id;
        let planned: Vec<PlannedTask> = serde_json::from_value(json!([
            {"id":"map","title":"Map","goal":"Inspect inputs","kind":"context","change_kind":"none","acceptance_criteria":[{"id":"C1","text":"Inputs documented"}]},
            {"id":"build","title":"Build","goal":"Implement behavior","kind":"implementation","change_kind":"code","depends_on":["map"],"acceptance_criteria":[{"id":"C1","text":"Behavior verified"}]}
        ])).unwrap();
        let first = crate::lean::core::plan::proposal::checked_team_plan(
            &tasks::planned_graph(planned).unwrap(),
            &config,
            tasks::Team::SmallChange,
        )
        .unwrap();
        let old = core
            .prepare_plan(&conversation, &first, Some("Original scope"), &[])
            .unwrap();
        core.store.queue_plan(&conversation, &old.id).unwrap();
        for mut task in core.store.plan(&old.id).unwrap() {
            task.status = status::APPROVED.into();
            task.round = 2;
            task.report = Some(format!("Accepted result of {}", task.spec["id"]));
            task.verdict = Some(json!({"approved":true,"summary":"Verified"}));
            task.feedback = Some(json!({"round":2,"effect":"Completed work"}));
            core.store.update_task(&task).unwrap();
            core.store
                .db()
                .execute(
                    "INSERT INTO task_source_revisions VALUES(?1,?2,?3)",
                    rusqlite::params![task.id, task.input_generation, revision],
                )
                .unwrap();
            core.store
                .test_completion(&task, task.report.as_deref().unwrap());
        }
        std::fs::write(
            project.join("input.txt"),
            "Changes integrated by a successor task",
        )
        .unwrap();
        crate::lean::git::commit_all(&project, crate::lean::git::CommitPurpose::TestFixture)
            .unwrap();
        // A real replan extends the scope after earlier tasks have changed files.
        // Historical completions need not describe the current whole project.
        let keep = vec!["map".into(), "build".into()];
        let added = serde_json::from_value(json!([{"id":"next","title":"Next","goal":"Extend behavior","kind":"implementation","change_kind":"code","depends_on":["build"],"acceptance_criteria":[{"id":"C1","text":"Extension verified"}]}])).unwrap();
        let graph = core.submitted_graph(&conversation, added, &keep).unwrap();
        let specs = core
            .checked_candidate_plan(&conversation, &graph, tasks::Team::SmallChange, &[])
            .unwrap();
        assert!(!specs.iter().any(|spec| spec.id == "build-options"));
        let next = core
            .prepare_plan(&conversation, &specs, Some("Expanded scope"), &keep)
            .unwrap();
        let review: Verdict = serde_json::from_value(json!({"approved":true})).unwrap();
        core.publish_plan(&conversation, "Extend behavior", None, &next, &Ok(review))
            .unwrap();
        let proposal = core
            .store
            .events_since(&conversation, 0, 1000)
            .unwrap()
            .into_iter()
            .find(|e| e.kind == "plan_proposed")
            .unwrap();
        let mut view = crate::lean::tui::view::View::default();
        view.apply(&serde_json::to_value(&proposal).unwrap());
        assert!(view.plan_proposed());
        for name in &keep {
            let row = view.tasks.iter().find(|t| &t.id == name).unwrap();
            assert_eq!((row.status.as_str(), row.round), ("approved", 2));
        }
        // Late events from the old plan cannot replace the new proposal's rows.
        view.apply(&json!({"kind":"task","plan_id":old.id,"payload":{"id":"map","task":"old","status":"queued","round":0}}));
        assert_eq!(
            view.tasks.iter().find(|t| t.id == "map").unwrap().status,
            "approved"
        );
        assert!(view.plan_proposed());
        assert!(
            core.store
                .plan(&next.id)
                .unwrap()
                .iter()
                .all(|t| t.status == status::PROPOSED)
        );

        core.store.queue_plan(&conversation, &next.id).unwrap();
        let old_tasks = core.store.plan(&old.id).unwrap();
        for task in core.store.plan(&next.id).unwrap() {
            let name = task.spec["id"].as_str().unwrap();
            if keep.iter().any(|id| id == name) {
                let source = old_tasks.iter().find(|t| t.spec["id"] == name).unwrap();
                assert_eq!(task.status, status::APPROVED);
                assert_eq!(task.round, source.round);
                assert_eq!(task.report, source.report);
                assert_eq!(task.verdict, source.verdict);
                assert_eq!(
                    core.store.completed_task_output(&task).unwrap(),
                    core.store.completed_task_output(source).unwrap()
                );
            } else {
                assert_eq!(task.status, status::QUEUED, "{name}");
            }
            view.apply(&json!({"kind":"task","plan_id":next.id,"payload":{"id":name,"task":task.id,"status":task.status,"round":task.round}}));
        }
        assert!(!view.plan_proposed());
        let mut replay = crate::lean::tui::view::View::default();
        replay.apply(&serde_json::to_value(&proposal).unwrap());
        assert!(replay.plan_proposed());
        assert_eq!(
            replay
                .tasks
                .iter()
                .filter(|t| t.status == "approved")
                .count(),
            2
        );
    }
}
