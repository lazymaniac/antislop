//! Turn-bound access to approved results and their same-input worker rounds.
use super::*;

impl Store {
    pub(in crate::lean) fn dependency_plan(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
    ) -> Result<String> {
        let db = self.db();
        let plan: Option<String> = db.query_row(
            "SELECT coalesce(t.plan_id,
                (SELECT p.id FROM plans p JOIN plan_authorizations a ON a.plan_id=p.id WHERE p.conversation_id=t.conversation_id ORDER BY p.created_ms DESC,p.rowid DESC LIMIT 1),
                (SELECT p.id FROM plans p WHERE p.conversation_id=t.conversation_id ORDER BY p.created_ms DESC,p.rowid DESC LIMIT 1))
             FROM turns t WHERE t.id=?1 AND t.conversation_id=?2 AND t.role=?3 AND t.status='running'",
            params![turn,conversation,role], |r| r.get(0))?;
        let plan = plan.ok_or_else(|| Error::Invalid("no plan bound to this turn".into()))?;
        crate::require(
            db.query_row(
                "SELECT EXISTS(SELECT 1 FROM plans WHERE id=?1 AND conversation_id=?2)",
                params![plan, conversation],
                |r| r.get::<_, bool>(0),
            )?,
            "turn plan belongs to another conversation",
        )?;
        // Legacy/unbound turns freeze the same accepted-plan choice on first use.
        db.execute(
            "UPDATE turns SET plan_id=?1 WHERE id=?2 AND plan_id IS NULL",
            params![plan, turn],
        )?;
        Ok(plan)
    }

    pub(in crate::lean) fn dependency_report_round(
        &self,
        task: &Task,
        requested: Option<u32>,
    ) -> Result<(String, u32, Vec<u32>, Value)> {
        let (producer, latest) = self.completed_task_result(task)?;
        let source = self
            .task_by_id(&producer)?
            .ok_or_else(|| Error::Invalid("result producer disappeared".into()))?;
        let db = self.db();
        let mut query = db.prepare("SELECT s.round,r.result FROM agent_results r JOIN turns t ON t.id=r.turn_id JOIN turn_scopes s ON s.turn_id=t.id JOIN turn_inputs x ON x.turn_id=t.id WHERE s.task_id=?1 AND s.round<=?2 AND x.generation=?3 AND t.status='done' AND t.role='worker:'||?1 AND json_extract(r.result,'$.type')='completed' ORDER BY s.round DESC,t.started_ms DESC,t.rowid DESC")?;
        let mut rows = query.query(params![source.id, source.round, source.input_generation])?;
        let mut rounds = Vec::new();
        let mut selected = None;
        while let Some(row) = rows.next()? {
            let round: u32 = row.get(0)?;
            if rounds.last() == Some(&round) {
                continue;
            }
            rounds.push(round);
            if requested == Some(round) {
                selected = Some(serde_json::from_str::<Value>(&row.get::<_, String>(1)?)?);
            }
        }
        let round = requested.unwrap_or_else(|| rounds.first().copied().unwrap_or(source.round));
        if requested.is_some() && selected.is_none() {
            crate::require(
                rounds.is_empty() && round == source.round,
                "round: no completed worker report for these dependency inputs",
            )?;
        }
        if rounds.is_empty() {
            rounds.push(source.round);
        }
        Ok((producer, round, rounds, selected.unwrap_or(latest)))
    }
}
