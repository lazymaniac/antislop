use super::super::*;

impl Store {
    pub(super) fn plan_page_rows(
        &self,
        conversation: &str,
        offset: usize,
        limit: usize,
    ) -> Result<(usize, Vec<String>, Option<String>)> {
        let db = self.db();
        let total = plan_count(&db, conversation)?;
        let plan_ids = plan_ids(&db, conversation, offset, limit)?;
        let active_plan_id = active_plan(&db, conversation)?;
        Ok((total, plan_ids, active_plan_id))
    }

    pub(super) fn load_plan_histories(
        &self,
        conversation: &str,
        plan_ids: &[String],
    ) -> Result<Vec<PlanHistory>> {
        plan_ids
            .iter()
            .map(|id| self.plan_history(conversation, id))
            .collect()
    }
}

fn plan_count(db: &rusqlite::Connection, conversation: &str) -> Result<usize> {
    Ok(db.query_row(
        "SELECT COUNT(*) FROM plans WHERE conversation_id=?1",
        [conversation],
        |row| row.get::<_, i64>(0),
    )? as usize)
}

fn plan_ids(
    db: &rusqlite::Connection,
    conversation: &str,
    offset: usize,
    limit: usize,
) -> Result<Vec<String>> {
    let mut statement = db.prepare(
        "SELECT id FROM plans WHERE conversation_id=?1 ORDER BY created_ms DESC, rowid DESC LIMIT ?2 OFFSET ?3",
    )?;
    Ok(statement
        .query_map(params![conversation, limit as i64, offset as i64], |row| {
            row.get(0)
        })?
        .collect::<std::result::Result<_, _>>()?)
}

fn active_plan(db: &rusqlite::Connection, conversation: &str) -> Result<Option<String>> {
    Ok(db
        .query_row(
            "SELECT plan_id FROM tasks WHERE conversation_id=?1 ORDER BY created_ms DESC, rowid DESC LIMIT 1",
            [conversation],
            |row| row.get(0),
        )
        .optional()?)
}
