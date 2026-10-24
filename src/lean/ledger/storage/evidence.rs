use super::*;

impl Store {
    pub fn add_ledger_evidence(
        &self,
        conversation: &str,
        run_id: &str,
        verifier_turn: &str,
        assessment: &Assessment,
    ) -> Result<Evidence> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let r = run(&tx, conversation, run_id)?;
        let p = &assessment.evidence;
        require(
            !p.result.trim().is_empty()
                && !p.integrated_fingerprint.trim().is_empty()
                && !assessment.note.trim().is_empty(),
            "empty evidence or assessment",
        )?;
        require(
            !matches!(
                p.result.trim().to_ascii_lowercase().as_str(),
                "done" | "completed" | "ok"
            ),
            "completion assertion is not evidence",
        )?;
        independent(&tx, &p.producer_turn, verifier_turn)?;
        let bound: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM ledger_turns p JOIN ledger_turns v ON p.run_id=v.run_id WHERE p.turn_id=?1 AND v.turn_id=?2 AND p.run_id=?3 AND p.spec_revision=?4 AND v.spec_revision=?4 AND p.phase='audit' AND v.phase='audit')",params![p.producer_turn,verifier_turn,run_id,r.spec_revision],|r|r.get(0))?;
        require(
            bound,
            "evidence sessions are not bound to the active run revision",
        )?;
        let role: String = tx.query_row(
            "SELECT role FROM turns WHERE id=?1 AND conversation_id=?2",
            params![verifier_turn, conversation],
            |r| r.get(0),
        )?;
        require(
            role == "requirements-audit-verifier",
            "only integrated-result verifier may assess completion evidence",
        )?;
        let valid:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM task_inputs i JOIN tasks t ON t.id=i.task_id JOIN turns u ON u.id=?4 WHERE i.task_id=?1 AND i.run_id=?2 AND i.generation=?3 AND t.round=?5 AND t.status='approved' AND u.conversation_id=?6 AND u.role='requirements-audit-analyst')",params![p.task_id,run_id,p.generation,p.producer_turn,p.attempt,conversation],|r|r.get(0))?;
        require(
            valid,
            "evidence has stale task inputs, attempt, result, or unauthorized producer",
        )?;
        let all = entries(&tx, run_id, r.spec_revision)?;
        let e = all
            .iter()
            .find(|e| {
                e.spec.id == p.entry_id
                    && e.version == p.entry_version
                    && e.spec.disposition == Disposition::Active
            })
            .ok_or_else(|| invalid("evidence refers to an old or inactive entry"))?;
        require(
            e.spec.criteria.iter().any(|c| c.id == p.criterion_id),
            "unknown evidence criterion",
        )?;
        let linked:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM task_ledger_inputs WHERE task_id=?1 AND entry_id=?2 AND version=?3)",params![p.task_id,p.entry_id,p.entry_version],|r|r.get(0))?;
        require(linked, "evidence task does not implement this obligation")?;
        {
            let spec: String =
                tx.query_row("SELECT spec FROM tasks WHERE id=?1", [&p.task_id], |r| {
                    r.get(0)
                })?;
            require(
                serde_json::from_str::<TaskSpec>(&spec)?
                    .criterion_ids
                    .contains(&p.criterion_id),
                "evidence task does not implement this criterion",
            )?;
        }
        let evidence = Evidence {
            id: crate::id(),
            run_id: run_id.into(),
            proposal: p.clone(),
            verifier_turn: verifier_turn.into(),
            assessment: assessment.note.clone(),
            approved: assessment.approved,
        };
        tx.execute("INSERT INTO ledger_evidence VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)",params![evidence.id,run_id,p.entry_id,p.entry_version,p.criterion_id,p.task_id,p.generation,p.attempt,p.producer_turn,verifier_turn,p.result,p.artifact,p.integrated_fingerprint,assessment.note,assessment.approved,now_ms()])?;
        event(
            &tx,
            conversation,
            "ledger_evidence",
            &serde_json::json!({"run":run_id,"entry":p.entry_id,"version":p.entry_version,"criterion":p.criterion_id,"evidence":evidence.id,"approved":assessment.approved}),
        )?;
        tx.commit()?;
        Ok(evidence)
    }
    pub fn ledger_evidence(
        &self,
        conversation: &str,
        run_id: &str,
        entry: Option<&str>,
    ) -> Result<Vec<Evidence>> {
        let db = self.db();
        run(&db, conversation, run_id)?;
        Ok(db.prepare("SELECT id,entry_id,entry_version,criterion_id,task_id,generation,attempt,producer_turn,verifier_turn,result,artifact,fingerprint,assessment,approved FROM ledger_evidence WHERE run_id=?1 AND (?2 IS NULL OR entry_id=?2) ORDER BY created_ms,rowid")?.query_map(params![run_id,entry],|r|Ok(Evidence{id:r.get(0)?,run_id:run_id.into(),proposal:EvidenceProposal{entry_id:r.get(1)?,entry_version:r.get(2)?,criterion_id:r.get(3)?,task_id:r.get(4)?,generation:r.get(5)?,attempt:r.get(6)?,producer_turn:r.get(7)?,result:r.get(9)?,artifact:r.get(10)?,integrated_fingerprint:r.get(11)?},verifier_turn:r.get(8)?,assessment:r.get(12)?,approved:r.get(13)?}))?.collect::<std::result::Result<_,_>>()?)
    }
    pub fn ledger_statuses(
        &self,
        conversation: &str,
        run_id: &str,
        revision: Option<i64>,
        fingerprint: &str,
    ) -> Result<Vec<LedgerEntry>> {
        let mut all = self.ledger_entries(conversation, run_id, revision)?;
        let evidence = self.ledger_evidence(conversation, run_id, None)?;
        let db = self.db();
        let task_rows=db.prepare("SELECT t.id,t.spec,t.status,t.round,i.generation,t.plan_id FROM tasks t JOIN task_inputs i ON i.task_id=t.id WHERE i.run_id=?1 ORDER BY t.created_ms DESC,t.rowid DESC")?.query_map([run_id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,u32>(3)?,r.get::<_,i64>(4)?,r.get::<_,String>(5)?)))?.collect::<std::result::Result<Vec<_>,_>>()?;
        for e in &mut all {
            if e.spec.disposition != Disposition::Active {
                e.status = serde_json::to_value(e.spec.disposition)?
                    .as_str()
                    .unwrap()
                    .into();
                continue;
            }
            let mut linked: Vec<_> = task_rows
                .iter()
                .filter(|(_, s, _, _, _, _)| {
                    serde_json::from_str::<TaskSpec>(s)
                        .is_ok_and(|s| s.requirement_ids.contains(&e.spec.id))
                })
                .collect();
            if let Some(latest_plan) = linked.first().map(|t| t.5.clone()) {
                linked.retain(|t| t.5 == latest_plan);
            }
            let verified = e.spec.criteria.iter().any(|c| c.mandatory)
                && e.spec.criteria.iter().filter(|c| c.mandatory).all(|c| {
                    let mut current = evidence
                        .iter()
                        .filter(|ev| {
                            let p = &ev.proposal;
                            p.entry_id == e.spec.id
                                && p.entry_version == e.version
                                && p.criterion_id == c.id
                                && p.integrated_fingerprint == fingerprint
                                && task_rows
                                    .iter()
                                    .any(|(id, _, status, round, generation, _)| {
                                        *id == p.task_id
                                            && *generation == p.generation
                                            && *round == p.attempt
                                            && status == "approved"
                                    })
                        })
                        .peekable();
                    current.peek().is_some() && current.all(|ev| ev.approved)
                });
            e.status = if verified {
                "verified"
            } else if linked.iter().any(|(_, _, s, _, _, _)| s == "stale") {
                "stale"
            } else if !linked.is_empty() && linked.iter().all(|(_, _, s, _, _, _)| s == "approved")
            {
                "implemented"
            } else if linked
                .iter()
                .any(|(_, _, s, _, _, _)| matches!(s.as_str(), "working" | "verifying"))
            {
                "in_progress"
            } else if linked.iter().any(|(_, _, s, _, _, _)| {
                matches!(
                    s.as_str(),
                    "failed" | "needs_user" | "cancelled" | "waiting"
                )
            }) {
                "blocked"
            } else if !linked.is_empty() {
                "planned"
            } else if e.spec.criteria.is_empty() {
                "clarification_needed"
            } else {
                "identified"
            }
            .into();
        }
        // Fixed point: neither children nor prerequisites can be bypassed by a parent verdict.
        loop {
            let invalid: Vec<_> = all
                .iter()
                .filter(|e| {
                    e.status == "verified"
                        && all.iter().any(|c| {
                            c.spec.disposition == Disposition::Active
                                && ((c.spec.mandatory
                                    && c.spec.parent.as_deref() == Some(&e.spec.id))
                                    || e.spec.depends_on.contains(&c.spec.id))
                                && c.status != "verified"
                        })
                })
                .map(|e| e.spec.id.clone())
                .collect();
            if invalid.is_empty() {
                break;
            }
            for e in &mut all {
                if invalid.contains(&e.spec.id) {
                    e.status = "implemented".into();
                }
            }
        }
        Ok(all)
    }
    pub fn list_ledger(&self, query: &LedgerQuery, fingerprint: &str) -> Result<LedgerPage> {
        let run = self.run(&query.conversation, &query.run_id)?;
        let linked_ids: Option<HashSet<String>> = if let Some(task) = &query.task {
            Some(self.db().prepare("SELECT l.entry_id FROM task_ledger_inputs l JOIN task_inputs i ON i.task_id=l.task_id JOIN tasks t ON t.id=i.task_id WHERE i.run_id=?1 AND (t.id=?2 OR json_extract(t.spec,'$.id')=?2)")?.query_map(params![query.run_id,task],|r|r.get(0))?.collect::<std::result::Result<_,_>>()?)
        } else {
            None
        };
        let entries: Vec<_> = self
            .ledger_statuses(
                &query.conversation,
                &query.run_id,
                query.revision,
                fingerprint,
            )?
            .into_iter()
            .filter(|e| {
                query.status.as_ref().is_none_or(|s| *s == e.status)
                    && (!query.missing_coverage
                        || !matches!(e.status.as_str(), "verified" | "superseded_by_user"))
                    && linked_ids
                        .as_ref()
                        .is_none_or(|ids| ids.contains(&e.spec.id))
            })
            .collect();
        let total = entries.len();
        let limit = query.limit.unwrap_or(50).clamp(1, 200);
        Ok(LedgerPage {
            run,
            entries: entries.into_iter().skip(query.offset).take(limit).collect(),
            total,
            offset: query.offset,
            has_more: query.offset.saturating_add(limit) < total,
        })
    }
    pub fn requirements_gaps(
        &self,
        conversation: &str,
        run_id: &str,
        fingerprint: &str,
    ) -> Result<Vec<String>> {
        let r = self.run(conversation, run_id)?;
        let mut gaps = Vec::new();
        if !self.extraction_current(conversation, run_id)? {
            gaps.push(
                "source coverage has not been independently verified for this revision".into(),
            );
        }
        let all = self.ledger_statuses(conversation, run_id, None, fingerprint)?;
        if all.is_empty() {
            gaps.push("ledger is empty".into());
        }
        for e in &all {
            if e.spec.disposition == Disposition::Active
                && e.spec.mandatory
                && e.status != "verified"
            {
                gaps.push(format!(
                    "{}: {} (current integrated evidence required)",
                    e.spec.id, e.status
                ));
            }
        }
        let db = self.db();
        let audit:bool=db.query_row("SELECT approved FROM ledger_reviews WHERE run_id=?1 AND spec_revision=?2 AND kind='audit' AND fingerprint=?3 ORDER BY created_ms DESC,rowid DESC LIMIT 1",params![run_id,r.spec_revision,fingerprint],|r|r.get(0)).optional()?.unwrap_or(false);
        if !audit {
            gaps.push(
                "integrated compliance audit and independent verification are missing or stale"
                    .into(),
            );
        }
        Ok(gaps)
    }
}
