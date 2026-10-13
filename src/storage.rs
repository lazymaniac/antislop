use crate::{Result, domain::Run, hash, require};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub run_id: String,
    pub kind: String,
    pub payload: serde_json::Value,
}
pub struct Store {
    connection: Connection,
}
impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if path.exists() {
            require(
                !std::fs::symlink_metadata(path)?.file_type().is_symlink(),
                "database must not be a symlink",
            )?;
        }
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;",
        )?;
        let version: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        require(version <= 2, "database schema is newer than this binary")?;
        if version == 0 {
            connection.execute_batch("BEGIN IMMEDIATE;
              CREATE TABLE runs(id TEXT PRIMARY KEY, state TEXT NOT NULL);
              CREATE TABLE events(seq INTEGER PRIMARY KEY AUTOINCREMENT, run_id TEXT NOT NULL REFERENCES runs(id), kind TEXT NOT NULL, payload TEXT NOT NULL);
              CREATE TABLE requests(key TEXT PRIMARY KEY, fingerprint TEXT NOT NULL, response TEXT NOT NULL);
              PRAGMA user_version=1; COMMIT;")?;
        }
        if version <= 1 {
            let tx = connection.transaction()?;
            tx.execute_batch("ALTER TABLE runs ADD COLUMN state_hash TEXT NOT NULL DEFAULT '';")?;
            let records = {
                let mut statement = tx.prepare("SELECT id,state FROM runs")?;
                statement.query_map([], |row| Ok((row.get::<_, String>(0)?,row.get::<_, String>(1)?)))?
                    .collect::<std::result::Result<Vec<_>, _>>()?
            };
            for (id, state) in records { tx.execute("UPDATE runs SET state_hash=?1 WHERE id=?2", params![hash(state),id])?; }
            tx.execute_batch("PRAGMA user_version=2;")?;
            tx.commit()?;
        }
        Ok(Self { connection })
    }
    pub fn get(&self, id: &str) -> Result<Run> {
        let state: String =
            self.connection
                .query_row("SELECT state FROM runs WHERE id=?1", [id], |r| r.get(0))?;
        Ok(serde_json::from_str(&state)?)
    }
    /// Read only one bounded byte slice; don't deserialize/hash the full run for each page.
    pub fn run_chunk(&self, id: &str, offset: usize, count: usize) -> Result<(String, usize, String)> {
        require(count > 0 && count <= crate::transport::MAX_FRAME, "invalid document chunk size")?;
        let start = i64::try_from(offset).ok().and_then(|n| n.checked_add(1))
            .ok_or_else(|| crate::Error::Invalid("invalid document offset".into()))?;
        let (digest, total, mut bytes): (String, i64, Vec<u8>) = self.connection.query_row(
            "SELECT state_hash,length(CAST(state AS BLOB)),substr(CAST(state AS BLOB),?2,?3) FROM runs WHERE id=?1",
            params![id,start,(count + 4) as i64], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
        require(offset <= total as usize, "invalid document offset")?;
        if let Err(error) = std::str::from_utf8(&bytes) {
            require(error.error_len().is_none() && error.valid_up_to() > 0, "invalid document offset")?;
            bytes.truncate(error.valid_up_to());
        }
        let text = std::str::from_utf8(&bytes).map_err(|_| crate::Error::Invalid("invalid document UTF-8".into()))?;
        let mut end = count.min(text.len());
        while !text.is_char_boundary(end) { end -= 1; }
        Ok((digest, total as usize, text[..end].to_owned()))
    }
    pub fn all(&self) -> Result<Vec<Run>> {
        let mut statement = self
            .connection
            .prepare("SELECT state FROM runs ORDER BY id")?;
        let states = statement
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        states
            .into_iter()
            .map(|s| Ok(serde_json::from_str(&s)?))
            .collect()
    }
    pub fn page(&self, after: &str, count: usize) -> Result<Vec<Run>> {
        let mut statement = self.connection.prepare("SELECT state FROM runs WHERE id>?1 ORDER BY id LIMIT ?2")?;
        let states = statement.query_map(params![after, count as i64], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        states.into_iter().map(|text| Ok(serde_json::from_str(&text)?)).collect()
    }
    pub fn events(&self, after: u64) -> Result<Vec<Event>> {
        let after = i64::try_from(after)
            .map_err(|_| crate::Error::Invalid("invalid event cursor".into()))?;
        let mut statement = self.connection.prepare(
            "SELECT seq,run_id,kind,payload FROM events WHERE seq>?1 ORDER BY seq LIMIT 1000",
        )?;
        let rows = statement.query_map([after], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        rows.map(|row| {
            let (seq, run_id, kind, payload) = row?;
            Ok(Event {
                seq: u64::try_from(seq)
                    .map_err(|_| crate::Error::Invalid("negative event sequence".into()))?,
                run_id,
                kind,
                payload: serde_json::from_str(&payload)?,
            })
        })
        .collect()
    }
    pub fn sequence(&self) -> Result<u64> {
        Ok(self
            .connection
            .query_row("SELECT COALESCE(MAX(seq),0) FROM events", [], |r| {
                r.get::<_, i64>(0)
            })? as u64)
    }
    pub fn cached(&self, key: &str, request: &[u8]) -> Result<Option<Run>> {
        let row: Option<(String, String)> = self
            .connection
            .query_row(
                "SELECT fingerprint,response FROM requests WHERE key=?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        match row {
            Some((fingerprint, response)) => {
                require(
                    fingerprint == hash(request),
                    "idempotency key reused with different request",
                )?;
                Ok(Some(serde_json::from_str(&response)?))
            }
            None => Ok(None),
        }
    }
    /// The state, event and idempotency response commit together before a worker runs.
    pub fn save(&mut self, run: &Run, kind: &str, request: Option<(&str, &[u8])>) -> Result<()> {
        let state = serde_json::to_string(run)?;
        let tx = self.connection.transaction()?;
        tx.execute("INSERT INTO runs(id,state,state_hash) VALUES(?1,?2,?3) ON CONFLICT(id) DO UPDATE SET state=excluded.state,state_hash=excluded.state_hash", params![run.id,state,hash(&state)])?;
        let payload = serde_json::json!({"status":run.status,"stage":run.stage,"spec_revision":run.spec_revision,"attempt":run.attempt,"active_session":run.active_session,"payload_version":1,"timestamp_unix_ms":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis()});
        tx.execute(
            "INSERT INTO events(run_id,kind,payload) VALUES(?1,?2,?3)",
            params![run.id, kind, payload.to_string()],
        )?;
        if let Some((key, bytes)) = request {
            tx.execute(
                "INSERT INTO requests(key,fingerprint,response) VALUES(?1,?2,?3)",
                params![key, hash(bytes), state],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
