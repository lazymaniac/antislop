//! A validation records a command observation, never task approval or build artifacts.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::lean) struct ValidationSpec {
    pub command: Vec<String>,
    pub directory: String,
    pub scope: String,
    pub environment: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(in crate::lean) struct ValidationRun {
    pub id: String,
    pub producer_turn: String,
    pub tree: Option<String>,
    pub spec: ValidationSpec,
    pub status: String,
    pub exit_code: Option<i32>,
    pub problem: String,
    pub evidence: Option<String>,
}

impl Store {
    pub(in crate::lean) fn prepare_validation(
        &self,
        conversation: &str,
        turn: &str,
        tree: Option<&str>,
        spec: &ValidationSpec,
    ) -> Result<(ValidationRun, &'static str)> {
        // Unversioned directories cannot prove equivalence between executions.
        let identity = crate::hash(serde_json::to_vec(&(
            tree,
            spec,
            tree.is_none().then_some(turn),
        ))?);
        let mut db = self.db();
        let tx = db.transaction()?;
        let previous: Option<(String,String)> = tx.query_row(
            "SELECT v.id,t.status FROM validation_runs v JOIN turns t ON t.id=v.turn_id WHERE v.conversation=?1 AND v.identity=?2 ORDER BY v.rowid DESC LIMIT 1",
            params![conversation,identity], |row| Ok((row.get(0)?,row.get(1)?)),
        ).optional()?;
        if let Some((id, owner_status)) = previous {
            let run = validation_row(&tx, conversation, &id)?;
            if run.status == "passed"
                && run.evidence.is_some()
                && (owner_status == "done"
                    || run.producer_turn == turn && owner_status == "running")
            {
                tx.execute(
                    "INSERT OR IGNORE INTO validation_uses(turn_id,validation_id) VALUES(?1,?2)",
                    params![turn, id],
                )?;
                validation_event(
                    &tx,
                    conversation,
                    turn,
                    &id,
                    "validation_reused",
                    &serde_json::json!({"producer_turn":run.producer_turn}),
                )?;
                tx.commit()?;
                return Ok((run, "reuse"));
            }
            if owner_status == "running"
                && (run.status == "running"
                    || run.status == "passed"
                        && run.evidence.is_some()
                        && run.producer_turn != turn)
            {
                let action = if run.producer_turn == turn {
                    "run"
                } else {
                    "wait"
                };
                tx.commit()?;
                return Ok((run, action));
            }
            if run.status == "running" {
                tx.execute("UPDATE validation_runs SET status='interrupted',problem='producer turn ended without completing this validation' WHERE id=?1", [&id])?;
                validation_event(
                    &tx,
                    conversation,
                    turn,
                    &id,
                    "validation_interrupted",
                    &serde_json::json!({"producer_status":owner_status}),
                )?;
            }
        }
        let run = ValidationRun {
            id: crate::id(),
            producer_turn: turn.into(),
            tree: tree.map(str::to_owned),
            spec: spec.clone(),
            status: "running".into(),
            exit_code: None,
            problem: String::new(),
            evidence: None,
        };
        tx.execute("INSERT INTO validation_runs(id,conversation,turn_id,identity,tree,spec,status,created_ms) VALUES(?1,?2,?3,?4,?5,?6,'running',?7)",
            params![run.id,conversation,turn,identity,tree,serde_json::to_string(spec)?,now_ms()])?;
        validation_event(
            &tx,
            conversation,
            turn,
            &run.id,
            "validation_started",
            &serde_json::json!({"tree":tree}),
        )?;
        tx.commit()?;
        Ok((run, "run"))
    }

    pub(in crate::lean) fn validation(
        &self,
        conversation: &str,
        id: &str,
    ) -> Result<ValidationRun> {
        validation_row(&self.db(), conversation, id)
    }

    pub(in crate::lean) fn complete_validation(
        &self,
        conversation: &str,
        turn: &str,
        completed: &ValidationRun,
    ) -> Result<ValidationRun> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let previous = validation_row(&tx, conversation, &completed.id)?;
        crate::require(
            previous.producer_turn == turn,
            "validation belongs to another turn",
        )?;
        if previous.status != "running" {
            crate::require(
                previous.exit_code == completed.exit_code
                    && previous.evidence == completed.evidence,
                "validation already has a different immutable result",
            )?;
            return Ok(previous);
        }
        tx.execute("UPDATE validation_runs SET status=?2,exit_code=?3,problem=?4,evidence_id=?5 WHERE id=?1",
            params![completed.id,completed.status,completed.exit_code,completed.problem,completed.evidence])?;
        validation_event(
            &tx,
            conversation,
            turn,
            &completed.id,
            "validation_finished",
            &serde_json::json!({"status":completed.status,"exit_code":completed.exit_code,"evidence":completed.evidence}),
        )?;
        tx.commit()?;
        Ok(completed.clone())
    }

    pub(in crate::lean) fn invalidate_validation(
        &self,
        conversation: &str,
        turn: &str,
        id: &str,
        reason: &str,
    ) -> Result<ValidationRun> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let mut run = validation_row(&tx, conversation, id)?;
        crate::require(
            run.status != "running",
            "validation is still running; wait for its result",
        )?;
        run.status = "invalidated".into();
        run.problem = reason.into();
        tx.execute(
            "UPDATE validation_runs SET status='invalidated',problem=?2 WHERE id=?1",
            params![id, reason],
        )?;
        validation_event(
            &tx,
            conversation,
            turn,
            id,
            "validation_invalidated",
            &serde_json::json!({"reason":reason}),
        )?;
        tx.commit()?;
        Ok(run)
    }

    pub(in crate::lean) fn validation_index(
        &self,
        conversation: &str,
        before: Option<&str>,
    ) -> Result<Vec<ValidationRun>> {
        let db = self.db();
        if let Some(id) = before {
            validation_row(&db, conversation, id)?;
        }
        let ids = db.prepare("SELECT id FROM validation_runs WHERE conversation=?1 AND (?2 IS NULL OR rowid < (SELECT rowid FROM validation_runs WHERE conversation=?1 AND id=?2)) ORDER BY rowid DESC LIMIT 32")?
            .query_map(params![conversation,before],|row|row.get::<_,String>(0))?
            .collect::<std::result::Result<Vec<_>,_>>()?;
        ids.iter()
            .map(|id| validation_row(&db, conversation, id))
            .collect()
    }
}

fn validation_row(db: &Connection, conversation: &str, id: &str) -> Result<ValidationRun> {
    let encoded: Option<String> = db.query_row(
        "SELECT json_object('id',id,'producer_turn',turn_id,'tree',tree,'spec',json(spec),'status',status,'exit_code',exit_code,'problem',problem,'evidence',evidence_id) FROM validation_runs WHERE conversation=?1 AND id=?2",
        params![conversation,id], |row|row.get(0),
    ).optional()?;
    let encoded = encoded
        .ok_or_else(|| Error::Invalid("validation is unknown in this conversation".into()))?;
    Ok(serde_json::from_str(&encoded)?)
}

fn validation_event(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    turn: &str,
    id: &str,
    kind: &str,
    detail: &Value,
) -> Result<()> {
    let payload = serde_json::json!({"validation":id,"detail":detail});
    tx.execute("INSERT INTO events(conversation_id,turn_id,kind,payload,created_ms) VALUES(?1,?2,?3,?4,?5)",params![conversation,turn,kind,payload.to_string(),now_ms()])?;
    Ok(())
}
