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
        tasks::TaskSpec,
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
fn entries(db: &Connection, run: &str, revision: i64) -> Result<Vec<LedgerEntry>> {
    let mut stmt = db.prepare("SELECT v.entry_id,v.version,v.spec_revision,v.source_id,v.start_byte,v.end_byte,v.original,v.interpretation,v.mandatory,v.position,v.disposition
      FROM ledger_versions v JOIN ledger_entries e ON e.run_id=v.run_id AND e.id=v.entry_id
      WHERE v.run_id=?1 AND v.spec_revision<=?2 AND v.version=(SELECT max(w.version) FROM ledger_versions w WHERE w.run_id=v.run_id AND w.entry_id=v.entry_id AND w.spec_revision<=?2) ORDER BY v.position,v.entry_id")?;
    let rows = stmt
        .query_map(params![run, revision], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)? as usize,
                r.get::<_, i64>(5)? as usize,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, bool>(8)?,
                r.get::<_, u32>(9)?,
                r.get::<_, String>(10)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut sources = HashMap::<String, String>::new();
    rows.into_iter().map(|(id,version,spec_revision,source_id,start,end,_original,interpretation,mandatory,position,disposition)| {
        let criteria = db.prepare("SELECT id,text,mandatory,integration FROM ledger_criteria WHERE run_id=?1 AND entry_id=?2 AND version=?3 ORDER BY id")?.query_map(params![run,id,version], |r| Ok(Criterion{id:r.get(0)?,text:r.get(1)?,mandatory:r.get(2)?,integration:r.get(3)?}))?.collect::<std::result::Result<_,_>>()?;
        let state=std::path::Path::new(db.path().unwrap_or("")).parent().ok_or_else(||invalid("ledger database has no source directory"))?;
        if !sources.contains_key(&source_id) {sources.insert(source_id.clone(),source_text(db,run,&source_id,state)?);}
        let original=SourceSpan{source_id:source_id.clone(),start,end}.text(&sources[&source_id])?.to_owned();
        Ok(LedgerEntry{run_id:run.into(),version,spec_revision,original,status:"identified".into(),spec:EntrySpec{
            source:SourceSpan{source_id,start,end}, interpretation,mandatory,position,
            disposition:serde_json::from_value(serde_json::Value::String(disposition))?, criteria,
            qualifiers:relations(db,run,&id,version,"qualifier")?,
            parent:relations(db,run,&id,version,"parent")?.into_iter().next(),
            depends_on:relations(db,run,&id,version,"dependency")?,
            task_ids:relations(db,run,&id,version,"task")?,
            implementation_owners:relations(db,run,&id,version,"implementer")?,
            verification_owners:relations(db,run,&id,version,"verifier")?, id,
        }})
    }).collect()
}
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
    )?;
    let text = if let Some(message) = message {
        let encoded: String =
            db.query_row("SELECT text FROM messages WHERE seq=?1", [message], |r| {
                r.get(0)
            })?;
        crate::lean::content::Content::new(&state.join("core.db")).decode(&encoded)?
    } else {
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
        std::fs::read_to_string(path)?
    };
    require(
        crate::hash(text.as_bytes()) == hash,
        "source artifact hash mismatch",
    )?;
    Ok(text)
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
    let exists: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM requirements_resumptions WHERE operation_id=?1)",
        [&change.operation_id],
        |row| row.get(0),
    )?;
    if exists {
        return Ok(());
    }
    db.execute("INSERT INTO planning_runs VALUES(?1,?2) ON CONFLICT(conversation_id) DO UPDATE SET run_id=excluded.run_id", params![change.conversation,change.run_id])?;
    let text = format!(
        "The user approved the exact requirements change for run {}. Continue independent requirements verification and prepare the plan for review using this run. This approves requirements only; implementation still requires separate plan approval.",
        change.run_id
    );
    db.execute("INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'requirements-approval',?2,?3)", params![change.conversation,text,now_ms()])?;
    let seq = db.last_insert_rowid();
    db.execute(
        "INSERT INTO requirements_resumptions VALUES(?1,?2)",
        params![change.operation_id, seq],
    )?;
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
