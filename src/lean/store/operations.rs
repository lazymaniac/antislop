use super::*;

impl Store {
    /// Runs a state-changing request once per `operation_id`: a retry of the
    /// same operation returns the first result instead of acting again. A
    /// failed attempt records nothing, so its retry runs anew.
    pub(in crate::lean) fn idempotent(
        &self,
        conversation: &str,
        operation_id: &str,
        kind: &str,
        run: impl FnOnce() -> Result<Value>,
    ) -> Result<Value> {
        crate::require(
            uuid::Uuid::parse_str(operation_id).is_ok(),
            "operation_id must be a UUID",
        )?;
        let _operation = self.operations.lock().expect("operations mutex");
        let saved: Option<(String, String)> = self
            .db()
            .query_row(
                "SELECT kind,result FROM request_operations WHERE conversation_id=?1 AND operation_id=?2",
                params![conversation, operation_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((saved_kind, result)) = saved {
            crate::require(
                saved_kind == kind,
                &format!("operation_id {operation_id} was already used for {saved_kind}"),
            )?;
            return Ok(serde_json::from_str(&result)?);
        }
        let result = run()?;
        self.db().execute(
            "INSERT INTO request_operations(conversation_id,operation_id,kind,result,created_ms)
             VALUES(?1,?2,?3,?4,?5)",
            params![
                conversation,
                operation_id,
                kind,
                result.to_string(),
                now_ms()
            ],
        )?;
        Ok(result)
    }
}
