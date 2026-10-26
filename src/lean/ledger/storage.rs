use super::*;
mod changes;
mod evidence;
mod reviews;
mod runs;
mod sources;
mod turns;
mod validation;

use crate::{
    Error,
    lean::{
        store::{Store, now_ms},
        tasks::TaskContract,
    },
};
use rusqlite::{OptionalExtension, params};
use std::collections::{HashMap, HashSet};

fn invalid(text: &str) -> Error {
    Error::Invalid(text.into())
}
fn run(db: &Connection, conversation: &str, id: &str) -> Result<Run> {
    db.query_row("SELECT id,conversation_id,goal,spec_revision,state,start_message FROM runs WHERE id=?1 AND conversation_id=?2", params![id,conversation], |r| Ok(Run { id:r.get(0)?, conversation_id:r.get(1)?, goal:r.get(2)?, spec_revision:r.get(3)?, state:r.get(4)?,start_message:r.get::<_,i64>(5)? as u64 })).optional()?.ok_or_else(|| invalid("run does not belong to conversation"))
}
fn relations(
    db: &Connection,
    run: &str,
    id: &str,
    version: i64,
    kind: &str,
) -> Result<Vec<String>> {
    Ok(db.prepare("SELECT target FROM ledger_relations WHERE run_id=?1 AND entry_id=?2 AND version=?3 AND kind=?4 ORDER BY target")?.query_map(params![run,id,version,kind], |r| r.get(0))?.collect::<std::result::Result<_,_>>()?)
}
mod entries;
use entries::entries;

fn source_text(
    db: &Connection,
    run: &str,
    source: &str,
    state: &std::path::Path,
) -> Result<String> {
    let (message, locator, hash): (Option<i64>, Option<String>, String) = db.query_row(
        "SELECT message,locator,hash FROM ledger_sources WHERE id=?1 AND run_id=?2",
        params![source, run],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )
    .optional()?
    .ok_or_else(|| {
        Error::Invalid(format!(
            "unknown requirements source {source:?} for run {run}; copy an exact source identity from the supplied ledger inputs"
        ))
    })?;
    let text = match message {
        Some(seq) => message_source_text(db, seq, state)?,
        None => document_source_text(state, locator, &hash)?,
    };
    require(
        crate::hash(text.as_bytes()) == hash,
        "source artifact hash mismatch",
    )?;
    Ok(text)
}

fn message_source_text(db: &Connection, seq: i64, state: &std::path::Path) -> Result<String> {
    let encoded: String = db.query_row("SELECT text FROM messages WHERE seq=?1", [seq], |row| {
        row.get(0)
    })?;
    crate::lean::content::Content::new(&state.join("core.db")).decode(&encoded)
}

fn document_source_text(
    state: &std::path::Path,
    locator: Option<String>,
    hash: &str,
) -> Result<String> {
    let locator = locator.ok_or_else(|| invalid("source has no locator"))?;
    require(
        locator == format!("sources/{hash}.txt"),
        "invalid source artifact locator",
    )?;
    let path = state.join(locator);
    require(
        std::fs::symlink_metadata(&path)?.file_type().is_file(),
        "source artifact must be a regular immutable file",
    )?;
    Ok(std::fs::read_to_string(path)?)
}

fn event(
    db: &Connection,
    conversation: &str,
    kind: &str,
    payload: &serde_json::Value,
) -> Result<()> {
    db.execute(
        "INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,?2,?3,?4)",
        params![conversation, kind, payload.to_string(), now_ms()],
    )?;
    Ok(())
}

fn independent(db: &Connection, producer: &str, verifier: &str) -> Result<()> {
    let session = |id: &str| -> Result<(String, Option<String>, String, String)> {
        Ok(db.query_row(
            "SELECT role,session_id,status,conversation_id FROM turns WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )?)
    };
    let (p, ps, pstatus, pc) = session(producer)?;
    let (v, vs, vstatus, vc) = session(verifier)?;
    require(
        producer != verifier && p != v && pc == vc && (ps.is_none() || vs.is_none() || ps != vs),
        "self verification is forbidden",
    )?;
    require(
        pstatus == "done" && vstatus == "done",
        "evidence requires completed sessions",
    )?;
    Ok(())
}

fn saved_proposal_diff(db: &Connection, id: &str, change: &LedgerChange) -> Result<LedgerDiff> {
    let before = entries(db, &change.run_id, change.expected_spec_revision)?
        .into_iter()
        .filter(|e| change.entries.iter().any(|new| new.id == e.spec.id))
        .collect();
    let affected_tasks = db
        .prepare("SELECT task_id FROM ledger_proposal_tasks WHERE proposal_id=?1 ORDER BY task_id")?
        .query_map([id], |r| r.get(0))?
        .collect::<std::result::Result<_, _>>()?;
    Ok(LedgerDiff {
        run_id: change.run_id.clone(),
        from_revision: change.expected_spec_revision,
        to_revision: change.expected_spec_revision + 1,
        before,
        after: change.entries.clone(),
        affected_tasks,
    })
}

fn enqueue_requirements_resume(db: &Connection, change: &LedgerChange) -> Result<()> {
    if requirements_resume_exists(db, change)? {
        return Ok(());
    }
    db.execute("INSERT INTO planning_runs VALUES(?1,?2) ON CONFLICT(conversation_id) DO UPDATE SET run_id=excluded.run_id", params![change.conversation,change.run_id])?;
    let text = resume_message(change);
    let seq = insert_resume_message(db, change, &text)?;
    db.execute(
        "INSERT INTO requirements_resumptions VALUES(?1,?2)",
        params![change.operation_id, seq],
    )?;
    emit_resume_events(db, change, seq, &text)
}

fn requirements_resume_exists(db: &Connection, change: &LedgerChange) -> Result<bool> {
    Ok(db.query_row(
        "SELECT EXISTS(SELECT 1 FROM requirements_resumptions WHERE operation_id=?1)",
        [&change.operation_id],
        |row| row.get(0),
    )?)
}

fn resume_message(change: &LedgerChange) -> String {
    format!(
        "The user approved the exact requirements change for run {}. Continue independent requirements verification and prepare the plan for review using this run. This approves requirements only; implementation still requires separate plan approval.",
        change.run_id
    )
}

fn insert_resume_message(db: &Connection, change: &LedgerChange, text: &str) -> Result<i64> {
    db.execute("INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'requirements-approval',?2,?3)", params![change.conversation,text,now_ms()])?;
    Ok(db.last_insert_rowid())
}

fn emit_resume_events(db: &Connection, change: &LedgerChange, seq: i64, text: &str) -> Result<()> {
    event(
        db,
        &change.conversation,
        "message",
        &serde_json::json!({"seq":seq,"author":"requirements-approval","text":text}),
    )?;
    event(
        db,
        &change.conversation,
        "planning_progress",
        &serde_json::json!({"text":"Requirements approved. Resuming verification and planning."}),
    )?;
    Ok(())
}
