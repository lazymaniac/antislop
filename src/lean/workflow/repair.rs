use super::super::now_ms;
use super::record_event;
use crate::Result;
use rusqlite::params;

/// Recover an analysis that ended without its ledger once per failure, with a bounded budget.
pub(super) fn queue_requirements_format_repair(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
) -> Result<bool> {
    use rusqlite::OptionalExtension;
    let failure: Option<i64> = tx.query_row(
        "SELECT seq FROM events WHERE conversation_id=?1 AND kind='requirements_failed' AND seq=(SELECT max(seq) FROM events WHERE conversation_id=?1 AND kind='requirements_failed') AND json_extract(payload,'$.error')=?2",
        params![conversation, crate::lean::ledger::MISSING_LEDGER], |row| row.get(0),
    ).optional()?;
    let Some(failure) = failure else {
        return Ok(false);
    };
    let handled: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM events WHERE conversation_id=?1 AND seq>?2 AND kind IN ('turn_started','requirements_format_retry'))",
        params![conversation, failure], |row| row.get(0),
    )?;
    let retries: i64 = tx.query_row(
        "SELECT count(*) FROM messages WHERE conversation_id=?1 AND author='requirements-format-repair' AND seq>(SELECT coalesce(max(seq),0) FROM messages WHERE conversation_id=?1 AND author='user')",
        [conversation], |row| row.get(0),
    )?;
    if handled || retries >= 2 {
        return Ok(false);
    }
    let text = "Resume preparation of the SAME complete task plan, preserving requirements, tasks and dependencies. Submit the complete plan again with submit_plan. The previous requirements analyst ended without submitting the requirements; the runtime will retry that analyst with role-scoped review findings. Do not extract or edit the ledger yourself.";
    tx.execute("INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'requirements-format-repair',?2,?3)", params![conversation, text, now_ms()])?;
    let seq = tx.last_insert_rowid();
    record_event(
        tx,
        conversation,
        "message",
        serde_json::json!({"seq":seq,"author":"requirements-format-repair","text":text,"internal":true}),
    )?;
    record_event(
        tx,
        conversation,
        "requirements_format_retry",
        serde_json::json!({"failure":failure}),
    )?;
    Ok(true)
}
