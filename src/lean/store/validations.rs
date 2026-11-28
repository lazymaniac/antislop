//! A validation records a command observation, never task approval or build artifacts.
use super::*;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::lean) struct ValidationSpec {
    pub command: Vec<String>,
    pub directory: String,
    pub scope: String,
    pub environment: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ValidationKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_exit_code: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(in crate::lean) enum ValidationKind {
    Test,
    Inspection,
    Discovery,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(in crate::lean) struct ValidationObservation {
    pub problems: Vec<String>,
    pub limitations: Vec<String>,
    pub tests_executed: Option<u64>,
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
    pub observation: Option<ValidationObservation>,
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
            1,
            tree,
            spec,
            tree.is_none().then_some(turn),
        ))?);
        let mut db = self.db();
        let tx = db.transaction()?;
        let live: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM turns WHERE id=?1 AND conversation_id=?2 AND status='running')",
            params![turn, conversation],
            |row| row.get(0),
        )?;
        crate::require(live, "validation producer turn is no longer running")?;
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
            observation: None,
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
        if let Some(observation) = &completed.observation {
            tx.execute(
                "INSERT INTO validation_observations(validation_id,observation) VALUES(?1,?2)",
                params![completed.id, serde_json::to_string(observation)?],
            )?;
        }
        validation_event(
            &tx,
            conversation,
            turn,
            &completed.id,
            "validation_finished",
            &serde_json::json!({"status":completed.status,"exit_code":completed.exit_code,"evidence":completed.evidence,"observation":completed.observation}),
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

    pub(in crate::lean) fn recent_validation_summaries(
        &self,
        conversation: &str,
    ) -> Result<Vec<Value>> {
        let db = self.db();
        let mut query = db.prepare(
            "SELECT id,json_extract(spec,'$.command'),status FROM validation_runs WHERE conversation=?1 ORDER BY rowid DESC LIMIT 8",
        )?;
        let rows = query.query_map([conversation], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (id, command, status) = row?;
            let command: Vec<String> = serde_json::from_str(&command)?;
            Ok(serde_json::json!({
                "id":id,"command":crate::lean::tasks::bound(&command.join(" "), 160),"status":status
            }))
        })
        .collect()
    }

    pub(in crate::lean) fn validation_available_to_turn(
        &self,
        validation: &ValidationRun,
        turn: &str,
    ) -> Result<bool> {
        if validation.producer_turn == turn {
            return Ok(true);
        }
        Ok(self.db().query_row(
            "SELECT EXISTS(SELECT 1 FROM validation_uses WHERE validation_id=?1 AND turn_id=?2)",
            params![validation.id, turn],
            |row| row.get(0),
        )?)
    }

    pub(in crate::lean) fn validation_is_newer(
        &self,
        current: &str,
        earlier: &str,
    ) -> Result<bool> {
        Ok(self.db().query_row(
            "SELECT newer.rowid>older.rowid FROM validation_runs newer JOIN validation_runs older WHERE newer.id=?1 AND older.id=?2",
            params![current, earlier], |row| row.get(0),
        )?)
    }
}

pub(super) fn interrupt_validation_claims(
    tx: &rusqlite::Transaction<'_>,
    turn: Option<&str>,
) -> Result<()> {
    let claims = {
        let mut statement = tx.prepare(
            "UPDATE validation_runs SET status='interrupted',problem='producer turn ended without completing this validation'
             WHERE status='running' AND turn_id IN (SELECT id FROM turns WHERE status<>'running' AND (?1 IS NULL OR id=?1))
             RETURNING conversation,turn_id,id",
        )?;
        statement
            .query_map([turn], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    for (conversation, turn, id) in claims {
        validation_event(
            tx,
            &conversation,
            &turn,
            &id,
            "validation_interrupted",
            &serde_json::json!({"reason":"producer turn ended without completing this validation"}),
        )?;
    }
    Ok(())
}

fn validation_row(db: &Connection, conversation: &str, id: &str) -> Result<ValidationRun> {
    let encoded: Option<String> = db.query_row(
        "SELECT json_object('id',id,'producer_turn',turn_id,'tree',tree,'spec',json(spec),'status',status,'exit_code',exit_code,'problem',problem,'evidence',evidence_id,'observation',json((SELECT observation FROM validation_observations WHERE validation_id=validation_runs.id))) FROM validation_runs WHERE conversation=?1 AND id=?2",
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
