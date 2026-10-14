use crate::{Result, domain::Run, hash, require};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{cell::RefCell, path::Path};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub run_id: String,
    pub kind: String,
    pub payload: serde_json::Value,
}
pub struct Store {
    path: std::path::PathBuf,
    connection: Connection,
    // One decoded snapshot; exact database bytes remain authoritative on every read.
    decoded: RefCell<Option<(String, String, Run)>>,
}
impl Store {
    pub fn data_directory(&self) -> Result<std::path::PathBuf> {
        self.path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .canonicalize()
            .map_err(Into::into)
    }
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
                statement
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?
            };
            for (id, state) in records {
                tx.execute(
                    "UPDATE runs SET state_hash=?1 WHERE id=?2",
                    params![hash(state), id],
                )?;
            }
            tx.execute_batch("PRAGMA user_version=2;")?;
            tx.commit()?;
        }
        Ok(Self {
            path: path.to_owned(),
            connection,
            decoded: RefCell::new(None),
        })
    }
    pub fn get(&self, id: &str) -> Result<Run> {
        let state: String =
            self.connection
                .query_row("SELECT state FROM runs WHERE id=?1", [id], |r| r.get(0))?;
        if let Some((cached_id, cached_state, cached_run)) = &*self.decoded.borrow()
            && cached_id == id
            && cached_state == &state
        {
            return Ok(cached_run.clone());
        }
        let run: Run = serde_json::from_str(&state)?;
        *self.decoded.borrow_mut() = Some((id.to_owned(), state, run.clone()));
        Ok(run)
    }
    /// Read only one bounded byte slice; don't deserialize/hash the full run for each page.
    pub fn run_chunk(
        &self,
        id: &str,
        offset: usize,
        count: usize,
    ) -> Result<(String, usize, String)> {
        self.run_chunk_bound(id, offset, count, None)
    }
    pub(crate) fn run_chunk_bound(
        &self,
        id: &str,
        offset: usize,
        count: usize,
        expected_hash: Option<&str>,
    ) -> Result<(String, usize, String)> {
        require(
            count > 0 && count <= crate::transport::MAX_FRAME,
            "invalid document chunk size",
        )?;
        let start = i64::try_from(offset)
            .ok()
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| crate::Error::Invalid("invalid document offset".into()))?;
        let (digest, total, mut bytes): (String, i64, Vec<u8>) = self.connection.query_row(
            "SELECT state_hash,length(CAST(state AS BLOB)),substr(CAST(state AS BLOB),?2,?3) FROM runs WHERE id=?1",
            params![id,start,(count + 4) as i64], |row| Ok((row.get(0)?,row.get(1)?,row.get(2)?)))?;
        // A saved document may shrink or move a UTF-8 boundary while the client
        // holds a cursor from the preceding version. Reject that version before
        // interpreting its cursor against the new bytes.
        require(
            expected_hash.is_none_or(|h| h == digest),
            "document changed; restart from offset zero",
        )?;
        require(offset <= total as usize, "invalid document offset")?;
        if let Err(error) = std::str::from_utf8(&bytes) {
            require(
                error.error_len().is_none() && error.valid_up_to() > 0,
                "invalid document offset",
            )?;
            bytes.truncate(error.valid_up_to());
        }
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| crate::Error::Invalid("invalid document UTF-8".into()))?;
        let mut end = count.min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
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
        let mut statement = self
            .connection
            .prepare("SELECT state FROM runs WHERE id>?1 ORDER BY id LIMIT ?2")?;
        let states = statement
            .query_map(params![after, count as i64], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        states
            .into_iter()
            .map(|text| Ok(serde_json::from_str(&text)?))
            .collect()
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
        *self.decoded.borrow_mut() = Some((run.id.clone(), state, run.clone()));
        Ok(())
    }
}

#[cfg(test)]
mod reconciliation_commit_tests {
    use super::*;
    use crate::{
        domain::Status,
        recovery::WorkspaceLease,
        runtime::{Command, Runtime},
    };
    use std::{
        cell::RefCell,
        sync::{Arc, mpsc},
        time::Duration,
    };

    // SQLite invokes its actual connection callback when the reconciliation
    // transaction cannot obtain the write lock. The callback belongs to the
    // dispatch thread and cannot intercept other tests' database operations.
    thread_local! {
        static WRITE_WAIT: RefCell<Option<(mpsc::Sender<()>, mpsc::Receiver<()>)>> = const { RefCell::new(None) };
    }
    fn wait_for_write_lock(_attempt: i32) -> bool {
        WRITE_WAIT.with(|context| {
            let Some((entered, release)) = context.borrow_mut().take() else {
                return false;
            };
            entered.send(()).is_ok() && release.recv().is_ok()
        })
    }

    #[test]
    fn reconciliation_retains_workspace_until_actual_sqlite_commit_or_rollback() {
        for refuse_commit in [false, true] {
            let root = tempfile::tempdir().unwrap();
            let workspace = root.path().join("workspace");
            std::fs::create_dir(&workspace).unwrap();
            std::fs::write(workspace.join("source.txt"), "original input").unwrap();
            let database = root.path().join("state.db");
            let runtime = Arc::new(Runtime::new(Store::open(&database).unwrap()).unwrap());
            let spec = serde_json::from_value(serde_json::json!({
                "task_family":"research","objective":"Inspect the supplied source","workspace":workspace,
                "provider":"fake","model":null,"requirements":[{"id":"R1","source_ref":"user:1","original_text":"Inspect supplied source","criteria":{"verified":"Report the actual source"}}],
                "plan_items":[{"id":"P1","description":"Inspect source","requirement_ids":["R1"],"dependencies":[]}],
                "input_files":["source.txt"]
            })).unwrap();
            let mut before = runtime
                .dispatch(&crate::id(), Command::Create { spec })
                .unwrap();
            before.status = Status::Blocked;
            before.reconciliation_required = true;
            let before_sequence;
            {
                let mut store = runtime.store.lock().unwrap();
                store.save(&before, "test_interrupted", None).unwrap();
                before_sequence = store.sequence().unwrap();
            }
            std::fs::write(workspace.join("source.txt"), "observed user edit").unwrap();
            let command = Command::Reconcile {
                run_id: before.id.clone(),
                scope_hash: before.scope_hash("reconcile"),
                input_hash: crate::recovery::observe(&before).unwrap().hash,
            };
            let key = crate::id();
            let request = serde_json::to_vec(&command).unwrap();
            let blocker = Connection::open(&database).unwrap();
            if refuse_commit {
                blocker.execute_batch("CREATE TRIGGER refuse_reconciliation BEFORE INSERT ON events WHEN NEW.kind='effects_reconciled' BEGIN SELECT RAISE(ABORT, 'injected reconciliation failure'); END;").unwrap();
            }
            blocker.execute_batch("BEGIN IMMEDIATE;").unwrap();
            let (entered_tx, entered_rx) = mpsc::channel();
            let (release_tx, release_rx) = mpsc::channel();
            let worker_runtime = runtime.clone();
            let worker_key = key.clone();
            let worker = std::thread::spawn(move || {
                WRITE_WAIT.with(|context| *context.borrow_mut() = Some((entered_tx, release_rx)));
                worker_runtime
                    .store
                    .lock()
                    .unwrap()
                    .connection
                    .busy_handler(Some(wait_for_write_lock))
                    .unwrap();
                worker_runtime.dispatch(&worker_key, command)
            });
            let entered = entered_rx.recv_timeout(Duration::from_secs(5));
            let competing = WorkspaceLease::acquire(workspace.to_str().unwrap());
            let retained = competing.is_err();
            drop(competing);
            // Always unblock and reap the dispatcher before asserting, including
            // the pre-fix failure. No detached worker outlives the fixture.
            blocker.execute_batch("ROLLBACK;").unwrap();
            let _ = release_tx.send(());
            let result = worker.join().unwrap();
            assert!(
                entered.is_ok(),
                "reconciliation never reached the actual SQLite write wait: {entered:?}"
            );
            assert!(
                retained,
                "workspace became available before reconciliation transaction finished (rollback={refuse_commit})"
            );
            assert!(WorkspaceLease::acquire(workspace.to_str().unwrap()).is_ok());
            let store = runtime.store.lock().unwrap();
            if refuse_commit {
                assert!(result.is_err());
                assert_eq!(
                    serde_json::to_value(store.get(&before.id).unwrap()).unwrap(),
                    serde_json::to_value(&before).unwrap()
                );
                assert_eq!(store.sequence().unwrap(), before_sequence);
                assert!(store.cached(&key, &request).unwrap().is_none());
            } else {
                let reconciled = result.unwrap();
                assert!(!reconciled.reconciliation_required);
                assert_eq!(
                    reconciled.input_snapshot["source.txt"],
                    "observed user edit"
                );
                assert_eq!(reconciled.spec_revision, before.spec_revision + 1);
                assert_eq!(store.sequence().unwrap(), before_sequence + 1);
                assert!(store.cached(&key, &request).unwrap().is_some());
            }
            assert_eq!(
                std::fs::read_to_string(workspace.join("source.txt")).unwrap(),
                "observed user edit",
                "reconciliation must never replay a filesystem effect"
            );
        }
    }
}
