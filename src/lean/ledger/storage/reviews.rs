use super::*;

impl Store {
    pub fn task_generation(&self, task: &str) -> Result<i64> {
        Ok(self
            .db()
            .query_row(
                "SELECT generation FROM task_inputs WHERE task_id=?1",
                [task],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or(0))
    }
    pub fn assert_task_inputs(&self, task: &crate::lean::store::Task) -> Result<()> {
        require(
            self.task_generation(&task.id)? == task.input_generation,
            "stale task generation; result retained in turn history",
        )?;
        let db = self.db();
        let bound: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM task_inputs i JOIN task_ledger_inputs l ON l.task_id=i.task_id WHERE i.task_id=?1)", [&task.id], |r|r.get(0))?;
        require(bound, "integrity error: task has no requirement inputs")?;
        let stale:bool=db.query_row("SELECT EXISTS(SELECT 1 FROM task_ledger_inputs l JOIN task_inputs i ON i.task_id=l.task_id WHERE l.task_id=?1 AND l.version<>(SELECT max(v.version) FROM ledger_versions v WHERE v.run_id=i.run_id AND v.entry_id=l.entry_id))",[&task.id],|r|r.get(0))?;
        require(!stale, "task ledger inputs changed; replan before resuming")
    }
    pub fn bind_task_ledger(&self, conversation: &str, run_id: &str, plan: &str) -> Result<()> {
        let all = self.ledger_entries(conversation, run_id, None)?;
        let tasks = self.plan(plan)?;
        let mut covered = HashSet::new();
        let prior = self.run_tasks(conversation, run_id)?;
        for task in prior
            .iter()
            .filter(|t| t.plan_id != plan && t.status == "approved")
        {
            if self.assert_task_inputs(task).is_ok() {
                let spec: TaskSpec = serde_json::from_value(task.spec.clone())?;
                covered.extend(spec.criterion_ids);
            }
        }
        let mut db = self.db();
        let tx = db.transaction()?;
        for t in &tasks {
            let spec: TaskSpec = serde_json::from_value(t.spec.clone())?;
            super::validate_task_links(&spec, &all)?;
            covered.extend(spec.criterion_ids.iter().cloned());
            let ids: HashSet<_> = spec.requirement_ids.iter().collect();
            tx.execute("INSERT INTO task_inputs(task_id,run_id) VALUES(?1,?2) ON CONFLICT(task_id) DO UPDATE SET run_id=excluded.run_id",params![t.id,run_id])?;
            tx.execute("DELETE FROM task_ledger_inputs WHERE task_id=?1", [&t.id])?;
            for id in ids {
                let e = all
                    .iter()
                    .find(|e| e.spec.id == *id)
                    .ok_or_else(|| invalid("unknown ledger task input"))?;
                tx.execute(
                    "INSERT INTO task_ledger_inputs VALUES(?1,?2,?3)",
                    params![t.id, id, e.version],
                )?;
            }
        }
        for e in &all {
            if e.spec.disposition != Disposition::Active {
                continue;
            }
            for assigned in &e.spec.task_ids {
                require(
                    tasks
                        .iter()
                        .chain(prior.iter().filter(|t| t.status == "approved"))
                        .any(|t| {
                            (t.id == *assigned || t.spec["id"].as_str() == Some(assigned))
                                && serde_json::from_value::<TaskSpec>(t.spec.clone())
                                    .is_ok_and(|s| s.requirement_ids.contains(&e.spec.id))
                        }),
                    &format!(
                        "ledger assignment {} -> {assigned} is not honored by the task plan",
                        e.spec.id
                    ),
                )?;
            }
        }
        super::validate_criterion_coverage(&all, &covered)?;
        tx.commit()?;
        Ok(())
    }
    #[allow(clippy::too_many_arguments)] // Session and input identities are validated together.
    pub fn record_ledger_review(
        &self,
        conversation: &str,
        run_id: &str,
        revision: i64,
        kind: &str,
        producer: &str,
        verifier: &str,
        fingerprint: &str,
        approved: bool,
        note: &str,
    ) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let r = run(&tx, conversation, run_id)?;
        require(
            r.spec_revision == revision,
            "review refers to an old spec revision",
        )?;
        require(
            matches!(kind, "extraction" | "audit" | "options"),
            "unknown ledger review kind",
        )?;
        require(!note.trim().is_empty(), "review needs an assessment")?;
        independent(&tx, producer, verifier)?;
        let bound: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM ledger_turns p JOIN ledger_turns v ON p.run_id=v.run_id WHERE p.turn_id=?1 AND v.turn_id=?2 AND p.run_id=?3 AND v.spec_revision=?4 AND p.phase=?5 AND v.phase=?5)",params![producer,verifier,run_id,revision,kind],|r|r.get(0))?;
        require(
            bound,
            "review sessions are not bound to this run revision and phase",
        )?;
        let uncovered: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM ledger_sources s WHERE s.run_id=?1 AND (NOT EXISTS(SELECT 1 FROM ledger_source_coverage c WHERE c.turn_id=?2 AND c.source_id=s.id) OR NOT EXISTS(SELECT 1 FROM ledger_source_coverage c WHERE c.turn_id=?3 AND c.source_id=s.id)))",params![run_id,producer,verifier],|r|r.get(0))?;
        require(
            !uncovered,
            "source set changed during review; repeat against all sources",
        )?;
        let (pc, pr): (String, String) = tx.query_row(
            "SELECT conversation_id,role FROM turns WHERE id=?1",
            [producer],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let vr: String = tx.query_row("SELECT role FROM turns WHERE id=?1", [verifier], |r| {
            r.get(0)
        })?;
        require(
            pc == conversation
                && pr == format!("requirements-{kind}-analyst")
                && vr == format!("requirements-{kind}-verifier"),
            "review author is not authorized for this run and phase",
        )?;
        tx.execute(
                "INSERT INTO ledger_reviews(run_id,spec_revision,kind,producer_turn,verifier_turn,fingerprint,approved,note,created_ms) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![
                    run_id,
                    revision,
                    kind,
                    producer,
                    verifier,
                    fingerprint,
                    approved,
                    note,
                    now_ms()
                ],
            )?;
        event(
            &tx,
            conversation,
            "ledger_review",
            &serde_json::json!({"run":run_id,"revision":revision,"kind":kind,"approved":approved}),
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn extraction_current(&self, conversation: &str, run_id: &str) -> Result<bool> {
        let db = self.db();
        let r = run(&db, conversation, run_id)?;
        Ok(db.query_row("SELECT approved FROM ledger_reviews WHERE run_id=?1 AND spec_revision=?2 AND kind='extraction' AND NOT EXISTS(SELECT 1 FROM ledger_sources s WHERE s.run_id=?1 AND NOT EXISTS(SELECT 1 FROM ledger_source_coverage c WHERE c.turn_id=ledger_reviews.verifier_turn AND c.source_id=s.id)) ORDER BY created_ms DESC,rowid DESC LIMIT 1",params![run_id,r.spec_revision],|r|r.get(0)).optional()?.unwrap_or(false))
    }
}
