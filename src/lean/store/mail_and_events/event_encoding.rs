use super::super::*;

impl Store {
    pub(super) fn encode_event_payload(&self, kind: &str, payload: &Value) -> Result<Value> {
        let mut payload = payload.clone();
        if is_agent_event(kind) {
            encode_session_fields(self, &mut payload)?;
        }
        Ok(payload)
    }

    pub(super) fn resolve_event_plan(
        &self,
        db: &rusqlite::Connection,
        turn: Option<&str>,
        plan_id: Option<&str>,
        payload: &Value,
    ) -> Result<Option<String>> {
        plan_id.map(str::to_owned).map_or_else(
            || infer_event_plan(db, turn, payload),
            |plan| Ok(Some(plan)),
        )
    }
}

fn is_agent_event(kind: &str) -> bool {
    matches!(kind, "agent" | "agent_final")
}

fn encode_session_fields(store: &Store, payload: &mut Value) -> Result<()> {
    for field in ["text", "delta"] {
        if let Some(text) = payload[field].as_str() {
            payload[field] = serde_json::json!({"session_content": store.content.write(text)?});
        }
    }
    Ok(())
}

fn infer_event_plan(
    db: &rusqlite::Connection,
    turn: Option<&str>,
    payload: &Value,
) -> Result<Option<String>> {
    if let Some(turn_id) = turn {
        return query_turn_plan(db, turn_id);
    }
    if let Some(plan_id) = payload.get("plan").and_then(Value::as_str) {
        return Ok(Some(plan_id.to_owned()));
    }
    if let Some(task_id) = payload.get("task").and_then(Value::as_str) {
        return query_task_plan(db, task_id);
    }
    Ok(payload
        .get("plan_id")
        .and_then(Value::as_str)
        .map(str::to_owned))
}

fn query_turn_plan(db: &rusqlite::Connection, turn_id: &str) -> Result<Option<String>> {
    Ok(db
        .query_row("SELECT plan_id FROM turns WHERE id=?1", [turn_id], |row| {
            row.get::<_, Option<String>>(0)
        })
        .optional()?
        .flatten())
}

fn query_task_plan(db: &rusqlite::Connection, task_id: &str) -> Result<Option<String>> {
    Ok(db
        .query_row("SELECT plan_id FROM tasks WHERE id=?1", [task_id], |row| {
            row.get::<_, Option<String>>(0)
        })
        .optional()?
        .flatten())
}
