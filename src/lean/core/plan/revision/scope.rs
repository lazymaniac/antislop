//! A reviewed scope amendment uses the same plan acceptance surface as a new plan.
use super::*;
use std::collections::BTreeSet;

impl Core {
    pub(in crate::lean) fn stage_scope_revision(
        &self,
        conversation: &str,
        revision: &PlanRevision,
        specs: &[TaskContract],
        affected: &BTreeSet<String>,
        interrupted: &BTreeSet<String>,
        request: &str,
    ) -> Result<()> {
        let summary = revision
            .scope
            .as_deref()
            .ok_or_else(|| Error::Invalid("scope is missing".into()))?;
        let old_id = self
            .store
            .plan_scope_id(&revision.plan)?
            .ok_or_else(|| Error::Invalid("plan has no scope".into()))?;
        let old = self.store.scope(conversation, &old_id)?;
        let scope = self.store.create_scope(conversation, summary)?;
        let fingerprint = crate::hash(serde_json::to_vec(&(revision, &scope.id, &old_id))?);
        let document = tasks::render_plan_document(
            &format!(
                "# Scope change awaiting acceptance\n\n## Accepted scope\n{}\nOriginal documents: {}\n\n## Proposed scope\n{}\nOriginal documents: {}\n\n## Reason\n{}",
                old.summary,
                old.source_ids.len(),
                summary,
                scope.source_ids.len(),
                revision.reason
            ),
            specs,
        );
        let payload = json!({"plan":revision.plan,"scope_revision":request,"scope":scope,"previous_scope":old,"base_revision":revision.base_revision,"document":document,"tasks":specs,"parallel":tasks::parallel_peers(specs),"review":{"approved":true},"automatic":false,"revision_token":fingerprint});
        let mut db = self.store.db();
        let tx = db.transaction()?;
        let current: i64 = tx.query_row(
            "SELECT revision FROM plan_heads WHERE plan_id=?1",
            [&revision.plan],
            |r| r.get(0),
        )?;
        require(current == revision.base_revision, "stale plan revision")?;
        tx.execute(
            "INSERT INTO scope_revision_proposals VALUES(?1,?2,?3,?4,?5,?6,'awaiting_acceptance')",
            rusqlite::params![
                request,
                scope.id,
                old_id,
                fingerprint,
                serde_json::to_string(affected)?,
                serde_json::to_string(interrupted)?
            ],
        )?;
        let changed=tx.execute("UPDATE plan_revision_jobs SET status='awaiting_acceptance' WHERE id=?1 AND status='reviewing'",[request])?;
        require(changed == 1, "revision is no longer under review")?;
        tx.execute("INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'plan_proposed',?3,?4)",rusqlite::params![conversation,revision.plan,payload.to_string(),store::now_ms()])?;
        tx.commit()?;
        Ok(())
    }

    pub(in crate::lean) fn accept_scope_revision(
        self: &Arc<Self>,
        conversation: &str,
        plan: &str,
    ) -> Result<Vec<Task>> {
        let request = self
            .store
            .pending_scope_revision(plan)?
            .ok_or_else(|| Error::Invalid("no scope amendment awaiting acceptance".into()))?;
        let proposal: String = self.store.db().query_row(
            "SELECT proposal FROM plan_revision_jobs WHERE id=?1 AND conversation_id=?2",
            rusqlite::params![request, conversation],
            |r| r.get(0),
        )?;
        let revision: PlanRevision = serde_json::from_str(&proposal)?;
        require(
            revision.base_revision == self.store.plan_revision(plan)?,
            "stale plan revision",
        )?;
        let baseline = self.store.plan(plan)?;
        let specs = self.revised_specs(&revision, &baseline)?;
        self.apply_direct_revision(
            conversation,
            &revision,
            &baseline,
            &specs,
            review::DirectRevision::Reviewed(&crate::lean::store::RevisionLease {
                request: &request,
                accepted_scope: true,
            }),
            true,
        )?;
        self.publish_scope_resolution(conversation, plan, &request, true)?;
        if !self.activity.plan_running(conversation) {
            self.maybe_continue_plan(conversation, plan)?;
        }
        self.store.plan(plan)
    }

    pub(in crate::lean) fn reject_scope_revision(
        self: &Arc<Self>,
        conversation: &str,
        plan: &str,
        reason: Option<&str>,
    ) -> Result<bool> {
        self.discard_scope_revision(conversation, plan, reason, true)
    }

    fn discard_scope_revision(
        self: &Arc<Self>,
        conversation: &str,
        plan: &str,
        reason: Option<&str>,
        resume: bool,
    ) -> Result<bool> {
        let Some(request) = self.store.pending_scope_revision(plan)? else {
            return Ok(false);
        };
        let mut db = self.store.db();
        let tx = db.transaction()?;
        tx.execute("UPDATE tasks SET status='queued' WHERE status='cancelled' AND id IN (SELECT value FROM scope_revision_proposals p,json_each(p.interrupted) WHERE p.request=?1)",[&request])?;
        tx.execute(
            "UPDATE scope_revision_proposals SET state='rejected' WHERE request=?1",
            [&request],
        )?;
        tx.execute(
            "UPDATE plan_revision_jobs SET status='rejected',error=?2 WHERE id=?1",
            rusqlite::params![
                request,
                reason.unwrap_or("scope amendment rejected by user")
            ],
        )?;
        tx.commit()?;
        drop(db);
        self.publish_scope_resolution(conversation, plan, &request, false)?;
        self.store.add_message(
            conversation,
            RUNTIME,
            &format!(
                "Scope amendment rejected: {}. The accepted scope and graph remain in force.",
                reason.unwrap_or("user rejected the proposal")
            ),
        )?;
        if resume {
            self.wake(conversation);
        }
        if resume && !self.activity.plan_running(conversation) {
            self.maybe_continue_plan(conversation, plan)?;
        }
        Ok(true)
    }
    pub(super) fn supersede_scope_revision(
        self: &Arc<Self>,
        conversation: &str,
        plan: &str,
        key: &str,
    ) -> Result<()> {
        if let Some(request) = self.store.pending_scope_revision(plan)? {
            let previous: String = self.store.db().query_row(
                "SELECT request_key FROM plan_revision_jobs WHERE id=?1",
                [request],
                |r| r.get(0),
            )?;
            if previous != key {
                self.discard_scope_revision(
                    conversation,
                    plan,
                    Some("superseded by a newer plan edit"),
                    false,
                )?;
            }
        }
        Ok(())
    }

    fn publish_scope_resolution(
        &self,
        conversation: &str,
        plan: &str,
        request: &str,
        accepted: bool,
    ) -> Result<()> {
        let scope_id = self
            .store
            .plan_scope_id(plan)?
            .ok_or_else(|| Error::Invalid("plan has no scope".into()))?;
        let scope = self.store.scope(conversation, &scope_id)?;
        let tasks = self.store.plan(plan)?;
        let specs = tasks
            .iter()
            .map(|task| serde_json::from_value::<TaskContract>(task.spec.clone()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let document =
            tasks::render_plan_document(&format!("# Accepted scope\n\n{}", scope.summary), &specs);
        self.store.append_event(conversation,None,Some(plan),"scope_revision_resolved",&json!({"plan":plan,"request":request,"accepted":accepted,"document":document,"tasks":specs,"parallel":tasks::parallel_peers(&specs),"revision":self.store.plan_revision(plan)?}))?;
        for task in tasks {
            self.announce_task(conversation, &task, None);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_amendment_survives_restart_and_applies_with_its_reviewed_graph_only_after_acceptance()
    {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        let config =
            Config::from_toml(include_str!("../../../../../examples/antislop.toml")).unwrap();
        let core = Core::open(&state, config.clone()).unwrap();
        let conversation = core
            .store
            .create_conversation(dir.path().to_str().unwrap())
            .unwrap()
            .id;
        core.store
            .add_message(&conversation, "user", "Export CSV locally")
            .unwrap();
        let old_scope = core
            .store
            .create_scope(&conversation, "Export CSV locally")
            .unwrap();
        let baseline=core.store.add_plan(&conversation,&[
            json!({"id":"export","title":"Export","goal":"Export CSV","kind":"implementation","change_kind":"code","acceptance_criteria":[{"id":"C1","text":"CSV contains a header"}]}),
            json!({"id":"independent","title":"Inspect","goal":"Inspect docs","kind":"analysis","change_kind":"none","acceptance_criteria":[{"id":"C1","text":"Report documentation gaps"}]}),
        ]).unwrap();
        let plan = baseline[0].plan_id.clone();
        core.store
            .bind_plan_scope(&conversation, &plan, &old_scope.id)
            .unwrap();
        let mut changed: TaskContract = serde_json::from_value(baseline[0].spec.clone()).unwrap();
        changed.acceptance_criteria[0].text = "CSV and JSON exports contain all fields".into();
        let revision = PlanRevision {
            plan: plan.clone(),
            base_revision: 0,
            reason: "Add JSON export".into(),
            scope: Some("Export CSV and JSON locally".into()),
            tasks: vec![changed],
            retire: vec![],
            redo: vec![],
            replacements: Default::default(),
        };
        let specs = revision.merge(&baseline).unwrap();
        let affected = BTreeSet::from([baseline[0].id.clone()]);
        let (request, _) = core
            .store
            .reserve_revision_job(&conversation, "scope-amendment", &revision)
            .unwrap();
        core.stage_scope_revision(
            &conversation,
            &revision,
            &specs,
            &affected,
            &BTreeSet::new(),
            &request,
        )
        .unwrap();
        assert_eq!(
            core.store.conversation_goal(&conversation).unwrap(),
            old_scope.summary
        );
        let token = core.plan_revision_token(&conversation, &plan).unwrap();
        assert_eq!(
            core.store.plan_scope_id(&plan).unwrap().as_deref(),
            Some(old_scope.id.as_str())
        );
        assert_eq!(core.store.plan(&plan).unwrap()[0].spec, baseline[0].spec);
        assert!(core.store.scope_holds_task(&baseline[0].id).unwrap());
        assert!(!core.store.scope_holds_task(&baseline[1].id).unwrap());
        drop(core);
        let core = Core::open(&state, config).unwrap();
        assert_eq!(
            core.store.pending_scope_revision(&plan).unwrap().as_deref(),
            Some(request.as_str())
        );
        assert_eq!(
            core.plan_revision_token(&conversation, &plan).unwrap(),
            token
        );
        let baseline = core.store.plan(&plan).unwrap();
        let apply = |accepted_scope| {
            core.store.apply_plan_revision(
                &conversation,
                &revision,
                &baseline,
                &specs,
                &affected,
                &store::RevisionLease {
                    request: &request,
                    accepted_scope,
                },
            )
        };
        assert!(
            apply(false)
                .unwrap_err()
                .to_string()
                .contains("requires acceptance")
        );
        assert_eq!(core.store.plan_revision(&plan).unwrap(), 0);
        apply(true).unwrap();
        assert_eq!(core.store.plan_revision(&plan).unwrap(), 1);
        assert_ne!(
            core.store.plan_scope_id(&plan).unwrap().as_deref(),
            Some(old_scope.id.as_str())
        );
        assert!(!core.store.scope_holds_task(&baseline[0].id).unwrap());
        let tasks = core.store.plan(&plan).unwrap();
        assert_eq!(tasks[0].input_generation, baseline[0].input_generation + 1);
        assert_eq!(tasks[1].input_generation, baseline[1].input_generation);
        assert!(core.store.assert_task_inputs(&baseline[0]).is_err());
        assert!(apply(true).is_err());

        let accepted_goal = core.store.conversation_goal(&conversation).unwrap();
        let accepted_scope = core.store.plan_scope_id(&plan).unwrap();
        let accepted = core.store.plan(&plan).unwrap();
        let mut added: TaskContract = serde_json::from_value(accepted[0].spec.clone()).unwrap();
        added.id = "extra".into();
        let second = PlanRevision {
            base_revision: 1,
            reason: "Add an extra export".into(),
            scope: Some("Export another format".into()),
            tasks: vec![added],
            ..revision.clone()
        };
        let specs = second.merge(&accepted).unwrap();
        let (request, _) = core
            .store
            .reserve_revision_job(&conversation, "second-scope", &second)
            .unwrap();
        let held = accepted.iter().map(|task| task.id.clone()).collect();
        for task in &accepted {
            let mut cancelled = task.clone();
            cancelled.status = status::CANCELLED.into();
            core.store.update_task(&cancelled).unwrap();
        }
        core.stage_scope_revision(
            &conversation,
            &second,
            &specs,
            &held,
            &BTreeSet::from([accepted[0].id.clone()]),
            &request,
        )
        .unwrap();
        let mut view = crate::lean::tui::view::View::default();
        for event in core.store.events_since(&conversation, 0, 10_000).unwrap() {
            view.apply(&serde_json::to_value(event).unwrap());
        }
        assert!(view.tasks.iter().any(|task| task.id == "extra"));
        let candidate_token = core.plan_revision_token(&conversation, &plan).unwrap();
        core.discard_scope_revision(&conversation, &plan, Some("Keep the accepted scope"), false)
            .unwrap();
        let restored = core.store.plan(&plan).unwrap();
        assert_eq!(restored[0].status, status::QUEUED);
        assert_eq!(
            restored[1].status,
            status::CANCELLED,
            "an earlier cancellation stays authoritative"
        );
        assert_eq!(
            core.store.conversation_goal(&conversation).unwrap(),
            accepted_goal
        );
        assert_eq!(core.store.plan_scope_id(&plan).unwrap(), accepted_scope);
        assert_eq!(core.store.plan_revision(&plan).unwrap(), 1);
        assert_ne!(
            core.plan_revision_token(&conversation, &plan).unwrap(),
            candidate_token
        );
        let mut view = crate::lean::tui::view::View::default();
        for event in core.store.events_since(&conversation, 0, 10_000).unwrap() {
            view.apply(&serde_json::to_value(event).unwrap());
        }
        assert!(!view.tasks.iter().any(|task| task.id == "extra"));
        assert!(!view.plan_proposed());
    }
}
