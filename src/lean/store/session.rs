use super::super::*;

impl Store {
    pub(super) fn validate_session_identity(
        tx: &rusqlite::Transaction<'_>,
        turn: &str,
        session: &str,
    ) -> Result<()> {
        let conflicting = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM run_contexts r JOIN turns t ON t.id=r.turn_id
                WHERE t.id=?1 AND
                ((t.provider='codex' AND r.provider_thread_id IS NOT NULL AND r.provider_thread_id!=?2)
                 OR (t.provider!='codex' AND r.provider_session_id IS NOT NULL AND r.provider_session_id!=?2)))",
            params![turn, session],
            |row| row.get::<_, bool>(0),
        )?;
        crate::require(
            !conflicting,
            "run is already bound to another provider session",
        )
    }

    pub(super) fn update_session_identity(
        tx: &rusqlite::Transaction<'_>,
        turn: &str,
        session: &str,
    ) -> Result<()> {
        tx.execute(
            "UPDATE turns SET session_id=?2 WHERE id=?1",
            params![turn, session],
        )?;
        tx.execute(
            "UPDATE run_contexts SET
                provider_thread_id=CASE WHEN (SELECT provider FROM turns WHERE id=?1)='codex'
                    THEN ?2 ELSE provider_thread_id END,
                provider_session_id=CASE WHEN (SELECT provider FROM turns WHERE id=?1)!='codex'
                    THEN ?2 ELSE provider_session_id END
             WHERE turn_id=?1",
            params![turn, session],
        )?;
        Ok(())
    }

    pub(super) fn last_turn_for_roles(
        &self,
        conversation: &str,
        roles: [&str; 3],
    ) -> Result<Option<Turn>> {
        let row = self.db().query_row(
            "SELECT id,role,status,session_id,error,upto_message,plan_id FROM turns
             WHERE conversation_id=?1 AND role IN (?2,?3,?4) ORDER BY started_ms DESC, rowid DESC LIMIT 1",
            params![conversation, roles[0], roles[1], roles[2]],
            read_turn_row,
        ).optional()?;
        row.map(decode_turn_row).transpose()
    }
}

type TurnRow = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    i64,
    Option<String>,
);

fn read_turn_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<TurnRow> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ))
}

fn decode_turn_row(
    (id, role, status, session_id, error, upto_message, plan_id): TurnRow,
) -> Result<Turn> {
    Ok(Turn {
        id,
        role,
        status: TurnStatus::parse(&status)?,
        session_id,
        error,
        upto_message: upto_message as u64,
        plan_id,
    })
}
