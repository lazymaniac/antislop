use super::super::now_ms;
use super::record_event;
use crate::Result;
use rusqlite::params;

pub(super) fn repair_is_needed(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    turn_id: &str,
) -> Result<bool> {
    let upto = turn_message_limit(tx, conversation, turn_id)?;
    let user = latest_user_message(tx, conversation)?;
    if user > upto {
        return Ok(false);
    }
    let repairs = repair_count_after(tx, conversation, user)?;
    let queued = repair_exists_after(tx, conversation, upto)?;
    Ok(repairs < 2 && !queued)
}

fn turn_message_limit(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    turn_id: &str,
) -> Result<i64> {
    Ok(tx.query_row(
        "SELECT upto_message FROM turns WHERE id=?1 AND conversation_id=?2 AND role IN ('orchestrator','comment-orchestrator','planner') AND status='done'",
        params![turn_id, conversation],
        |row| row.get(0),
    )?)
}

fn latest_user_message(tx: &rusqlite::Transaction<'_>, conversation: &str) -> Result<i64> {
    Ok(tx.query_row(
        "SELECT coalesce(max(seq),0) FROM messages WHERE conversation_id=?1 AND author='user'",
        [conversation],
        |row| row.get(0),
    )?)
}

fn repair_count_after(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    sequence: i64,
) -> Result<i64> {
    Ok(tx.query_row(
        "SELECT count(*) FROM messages WHERE conversation_id=?1 AND author='plan-format-repair' AND seq>?2",
        params![conversation, sequence],
        |row| row.get(0),
    )?)
}

fn repair_exists_after(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    sequence: i64,
) -> Result<bool> {
    Ok(tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM messages WHERE conversation_id=?1 AND author='plan-format-repair' AND seq>?2)",
        params![conversation, sequence],
        |row| row.get(0),
    )?)
}

pub(super) fn queue_plan_repair(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    turn_id: &str,
    error: &str,
) -> Result<()> {
    let text = repair_text(turn_id, error);
    tx.execute(
        "INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'plan-format-repair',?2,?3)",
        params![conversation, text, now_ms()],
    )?;
    let seq = tx.last_insert_rowid();
    record_repair_events(tx, conversation, seq, &text)
}

fn repair_text(turn_id: &str, error: &str) -> String {
    format!(
        "Repair the plan JSON in your previous response. Parser/validation error: {error}. Return the complete corrected ```plan JSON block for the SAME plan. Preserve the agreed requirements, selected strategy, tasks and dependencies. For JSON syntax errors, repair serialization only: escape ASCII double quotes inside string values, or use typographic quotes in prose. Do not repeat repository analysis, requirements extraction or options comparison, do not create another run, and do not ask the user to repair JSON. Your previous turn was {turn_id}."
    )
}

fn record_repair_events(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    seq: i64,
    text: &str,
) -> Result<()> {
    let message =
        serde_json::json!({"seq":seq,"author":"plan-format-repair","text":text,"internal":true});
    record_event(tx, conversation, "message", message)?;
    let progress = serde_json::json!({"text":"Correcting the plan format. Requirements and solution comparison are preserved."});
    record_event(tx, conversation, "planning_progress", progress)
}

/// Recover a malformed analyst response once per failure, with a bounded budget.
pub(super) fn queue_requirements_format_repair(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
) -> Result<bool> {
    use rusqlite::OptionalExtension;
    let failure: Option<i64> = tx.query_row(
        "SELECT seq FROM events WHERE conversation_id=?1 AND kind='requirements_failed' AND seq=(SELECT max(seq) FROM events WHERE conversation_id=?1 AND kind='requirements_failed') AND json_extract(payload,'$.error')='missing ledger JSON block'",
        [conversation], |row| row.get(0),
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
    let text = "Resume preparation of the SAME complete task plan, preserving requirements, tasks and dependencies. Return the complete fenced plan block. The previous requirements analyst response had no ledger JSON block; the runtime will retry that analyst with role-scoped review findings. Do not extract or edit the ledger yourself.";
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
