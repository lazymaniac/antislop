//! Small rows, each written on its own, plus an append-only event log. Nothing
//! here grows with the size of the project the agents work on.

use crate::{Error, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::Mutex;

pub struct Store {
    connection: Mutex<Connection>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Conversation {
    pub id: String,
    pub project: String,
    pub created_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub seq: u64,
    pub author: String,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Turn {
    pub id: String,
    pub role: String,
    pub status: TurnStatus,
    pub session_id: Option<String>,
    pub error: Option<String>,
    /// The last user message this turn answers.
    pub upto_message: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnStatus {
    Running,
    Done,
    Failed,
    Cancelled,
    Interrupted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub seq: u64,
    pub turn_id: Option<String>,
    pub kind: String,
    pub payload: Value,
}

/// One task of a plan. `spec` is what the orchestrator asked for; `report`
/// is the worker's latest answer and `verdict` its verifier's latest one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub plan_id: String,
    pub spec: Value,
    pub status: String,
    pub round: u32,
    pub report: Option<String>,
    pub verdict: Option<Value>,
    /// A writer's starting commit and its latest squashed result.
    pub base: Option<String>,
    pub result: Option<String>,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

impl TurnStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Interrupted => "interrupted",
        }
    }
    fn parse(text: &str) -> Result<Self> {
        Ok(match text {
            "running" => Self::Running,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            "interrupted" => Self::Interrupted,
            _ => return Err(Error::Invalid(format!("unknown turn status {text}"))),
        })
    }
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS conversations(
               id TEXT PRIMARY KEY, project TEXT NOT NULL, created_ms INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS messages(
               seq INTEGER PRIMARY KEY AUTOINCREMENT,
               conversation_id TEXT NOT NULL REFERENCES conversations(id),
               author TEXT NOT NULL, text TEXT NOT NULL, created_ms INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS turns(
               id TEXT PRIMARY KEY,
               conversation_id TEXT NOT NULL REFERENCES conversations(id),
               role TEXT NOT NULL, provider TEXT NOT NULL, session_id TEXT,
               status TEXT NOT NULL, error TEXT, upto_message INTEGER NOT NULL,
               started_ms INTEGER NOT NULL, finished_ms INTEGER);
             CREATE TABLE IF NOT EXISTS events(
               seq INTEGER PRIMARY KEY AUTOINCREMENT,
               conversation_id TEXT NOT NULL REFERENCES conversations(id),
               turn_id TEXT, kind TEXT NOT NULL, payload TEXT NOT NULL);
             CREATE INDEX IF NOT EXISTS messages_by_conversation ON messages(conversation_id, seq);
             CREATE INDEX IF NOT EXISTS turns_by_conversation ON turns(conversation_id, role, started_ms);
             CREATE INDEX IF NOT EXISTS events_by_conversation ON events(conversation_id, seq);
             CREATE TABLE IF NOT EXISTS tasks(
               id TEXT PRIMARY KEY,
               conversation_id TEXT NOT NULL REFERENCES conversations(id),
               plan_id TEXT NOT NULL, position INTEGER NOT NULL, spec TEXT NOT NULL,
               status TEXT NOT NULL, round INTEGER NOT NULL DEFAULT 0,
               report TEXT, verdict TEXT, base TEXT, result TEXT,
               created_ms INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS tasks_by_plan ON tasks(conversation_id, plan_id, position);",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.connection.lock().expect("store mutex")
    }

    pub fn create_conversation(&self, project: &str) -> Result<Conversation> {
        let conversation = Conversation {
            id: crate::id(),
            project: project.into(),
            created_ms: now_ms(),
        };
        self.db().execute(
            "INSERT INTO conversations(id,project,created_ms) VALUES(?1,?2,?3)",
            params![
                conversation.id,
                conversation.project,
                conversation.created_ms
            ],
        )?;
        Ok(conversation)
    }

    pub fn conversation(&self, id: &str) -> Result<Conversation> {
        self.db()
            .query_row(
                "SELECT id,project,created_ms FROM conversations WHERE id=?1",
                [id],
                |row| {
                    Ok(Conversation {
                        id: row.get(0)?,
                        project: row.get(1)?,
                        created_ms: row.get(2)?,
                    })
                },
            )
            .optional()?
            .ok_or_else(|| Error::Invalid(format!("no conversation {id}")))
    }

    pub fn conversations(&self) -> Result<Vec<Conversation>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT id,project,created_ms FROM conversations ORDER BY created_ms DESC LIMIT 200",
        )?;
        let rows = statement.query_map([], |row| {
            Ok(Conversation {
                id: row.get(0)?,
                project: row.get(1)?,
                created_ms: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Stores a message and announces it in the event log in one transaction.
    pub fn add_message(&self, conversation: &str, author: &str, text: &str) -> Result<Message> {
        let mut db = self.db();
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,?2,?3,?4)",
            params![conversation, author, text, now_ms()],
        )?;
        let seq = tx.last_insert_rowid() as u64;
        tx.execute(
            "INSERT INTO events(conversation_id,turn_id,kind,payload) VALUES(?1,NULL,'message',?2)",
            params![
                conversation,
                serde_json::json!({"seq":seq,"author":author,"text":text}).to_string()
            ],
        )?;
        tx.commit()?;
        Ok(Message {
            seq,
            author: author.into(),
            text: text.into(),
        })
    }

    pub fn messages_after(&self, conversation: &str, after: u64) -> Result<Vec<Message>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT seq,author,text FROM messages WHERE conversation_id=?1 AND seq>?2 ORDER BY seq",
        )?;
        let rows = statement.query_map(params![conversation, after as i64], |row| {
            Ok(Message {
                seq: row.get::<_, i64>(0)? as u64,
                author: row.get(1)?,
                text: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// One author's messages after `after`.
    pub fn messages_by(
        &self,
        conversation: &str,
        author: &str,
        after: u64,
    ) -> Result<Vec<Message>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT seq,author,text FROM messages
             WHERE conversation_id=?1 AND author=?2 AND seq>?3 ORDER BY seq",
        )?;
        let rows = statement.query_map(params![conversation, author, after as i64], |row| {
            Ok(Message {
                seq: row.get::<_, i64>(0)? as u64,
                author: row.get(1)?,
                text: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// How many messages `author` wrote since the user's last message.
    pub fn count_since_user(&self, conversation: &str, author: &str) -> Result<usize> {
        let count: i64 = self.db().query_row(
            "SELECT COUNT(*) FROM messages WHERE conversation_id=?1 AND author=?2
               AND seq > COALESCE((SELECT MAX(seq) FROM messages
                                   WHERE conversation_id=?1 AND author='user'), 0)",
            params![conversation, author],
            |row| row.get(0),
        )?;
        Ok(count as usize)
    }

    pub fn start_turn(
        &self,
        conversation: &str,
        role: &str,
        provider: &str,
        upto_message: u64,
    ) -> Result<Turn> {
        let turn = Turn {
            id: crate::id(),
            role: role.into(),
            status: TurnStatus::Running,
            session_id: None,
            error: None,
            upto_message,
        };
        self.db().execute(
            "INSERT INTO turns(id,conversation_id,role,provider,status,upto_message,started_ms)
             VALUES(?1,?2,?3,?4,'running',?5,?6)",
            params![
                turn.id,
                conversation,
                role,
                provider,
                upto_message as i64,
                now_ms()
            ],
        )?;
        Ok(turn)
    }

    pub fn set_session(&self, turn: &str, session: &str) -> Result<()> {
        self.db().execute(
            "UPDATE turns SET session_id=?2 WHERE id=?1",
            params![turn, session],
        )?;
        Ok(())
    }

    pub fn finish_turn(&self, turn: &str, status: TurnStatus, error: Option<&str>) -> Result<()> {
        self.db().execute(
            "UPDATE turns SET status=?2,error=?3,finished_ms=?4 WHERE id=?1",
            params![turn, status.as_str(), error, now_ms()],
        )?;
        Ok(())
    }

    /// The latest turn of `role`, which carries the session to resume and the
    /// last message it answered.
    pub fn last_turn(&self, conversation: &str, role: &str) -> Result<Option<Turn>> {
        self.db()
            .query_row(
                "SELECT id,role,status,session_id,error,upto_message FROM turns
                 WHERE conversation_id=?1 AND role=?2 ORDER BY started_ms DESC, rowid DESC LIMIT 1",
                params![conversation, role],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()?
            .map(|(id, role, status, session_id, error, upto)| {
                Ok(Turn {
                    id,
                    role,
                    status: TurnStatus::parse(&status)?,
                    session_id,
                    error,
                    upto_message: upto as u64,
                })
            })
            .transpose()
    }

    /// The last user message a successful turn of `role` answered.
    pub fn last_answered(&self, conversation: &str, role: &str) -> Result<u64> {
        Ok(self.db().query_row(
            "SELECT COALESCE(MAX(upto_message),0) FROM turns
             WHERE conversation_id=?1 AND role=?2 AND status='done'",
            params![conversation, role],
            |row| row.get::<_, i64>(0),
        )? as u64)
    }

    /// The session of the latest turn of `role` with `provider` that got one,
    /// even if that turn later failed: the CLI keeps what it already saw.
    /// Another CLI's session id would mean nothing to it.
    pub fn last_session(
        &self,
        conversation: &str,
        role: &str,
        provider: &str,
    ) -> Result<Option<String>> {
        Ok(self
            .db()
            .query_row(
                "SELECT session_id FROM turns WHERE conversation_id=?1 AND role=?2 AND provider=?3
                 AND session_id IS NOT NULL ORDER BY started_ms DESC, rowid DESC LIMIT 1",
                params![conversation, role, provider],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Turns left running by a daemon that stopped become interrupted; nothing
    /// else changes and nothing waits for a manual reconciliation.
    /// Returns the conversations whose orchestrator was cut off, so its
    /// unanswered messages can be taken up again.
    pub fn interrupt_running_turns(&self) -> Result<Vec<String>> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let conversations = {
            let mut statement = tx.prepare(
                "SELECT DISTINCT conversation_id FROM turns
                 WHERE status='running' AND role='orchestrator'",
            )?;
            statement
                .query_map([], |row| row.get(0))?
                .collect::<std::result::Result<Vec<String>, _>>()?
        };
        tx.execute(
            "UPDATE turns SET status='interrupted',finished_ms=?1 WHERE status='running'",
            [now_ms()],
        )?;
        tx.commit()?;
        Ok(conversations)
    }

    /// Unfinished tasks become cancelled with the reason. Returns each
    /// affected (conversation, plan), so the plan can be resumed.
    pub fn interrupt_unfinished_tasks(&self) -> Result<Vec<(String, String)>> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let plans = {
            let mut statement = tx.prepare(
                "SELECT DISTINCT conversation_id, plan_id FROM tasks
                 WHERE status IN ('queued','working','verifying')",
            )?;
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<std::result::Result<Vec<(String, String)>, _>>()?
        };
        tx.execute(
            "UPDATE tasks SET status='cancelled',
               report=COALESCE(report || char(10) || char(10), '') || '[interrupted] the daemon stopped while this task ran'
             WHERE status IN ('queued','working','verifying')",
            [],
        )?;
        tx.commit()?;
        Ok(plans)
    }

    /// Queues again every task of a plan that ended without approval, keeping
    /// its rounds, report, verdict and work. Returns how many.
    pub fn requeue_plan(&self, conversation: &str, plan_id: &str) -> Result<usize> {
        Ok(self.db().execute(
            "UPDATE tasks SET status='queued' WHERE conversation_id=?1 AND plan_id=?2
               AND status IN ('cancelled','failed','needs_user','skipped','waiting')",
            params![conversation, plan_id],
        )?)
    }

    /// Queues every task of a proposed plan, all or none.
    pub fn queue_plan(&self, conversation: &str, plan_id: &str) -> Result<usize> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let proposed: i64 = tx.query_row(
            "SELECT COUNT(*) FROM tasks WHERE conversation_id=?1 AND plan_id=?2 AND status='proposed'",
            params![conversation, plan_id],
            |row| row.get(0),
        )?;
        let all: i64 = tx.query_row(
            "SELECT COUNT(*) FROM tasks WHERE conversation_id=?1 AND plan_id=?2",
            params![conversation, plan_id],
            |row| row.get(0),
        )?;
        crate::require(
            all > 0 && proposed == all,
            "this plan is not waiting for approval",
        )?;
        tx.execute(
            "UPDATE tasks SET status='queued' WHERE conversation_id=?1 AND plan_id=?2",
            params![conversation, plan_id],
        )?;
        tx.commit()?;
        Ok(all as usize)
    }

    /// Moves every task of a plan that is still `from` to `to`.
    pub fn set_plan_status(&self, plan_id: &str, from: &str, to: &str) -> Result<usize> {
        Ok(self.db().execute(
            "UPDATE tasks SET status=?3 WHERE plan_id=?1 AND status=?2",
            params![plan_id, from, to],
        )?)
    }

    pub fn add_plan(&self, conversation: &str, specs: &[Value]) -> Result<Vec<Task>> {
        let plan_id = crate::id();
        let mut db = self.db();
        let tx = db.transaction()?;
        let mut tasks = vec![];
        for (position, spec) in specs.iter().enumerate() {
            let task = Task {
                id: crate::id(),
                plan_id: plan_id.clone(),
                spec: spec.clone(),
                status: "proposed".into(),
                round: 0,
                report: None,
                verdict: None,
                base: None,
                result: None,
            };
            tx.execute(
                "INSERT INTO tasks(id,conversation_id,plan_id,position,spec,status,created_ms)
                 VALUES(?1,?2,?3,?4,?5,'proposed',?6)",
                params![
                    task.id,
                    conversation,
                    plan_id,
                    position as i64,
                    spec.to_string(),
                    now_ms()
                ],
            )?;
            tasks.push(task);
        }
        tx.commit()?;
        Ok(tasks)
    }

    /// Tasks of the conversation's latest plan, in the orchestrator's order.
    pub fn latest_plan(&self, conversation: &str) -> Result<Vec<Task>> {
        let latest: Option<String> = self
            .db()
            .query_row(
                "SELECT plan_id FROM tasks WHERE conversation_id=?1
                 ORDER BY created_ms DESC, rowid DESC LIMIT 1",
                [conversation],
                |row| row.get(0),
            )
            .optional()?;
        match latest {
            Some(plan_id) => self.plan(&plan_id),
            None => Ok(vec![]),
        }
    }

    /// Tasks of one plan, in the orchestrator's order.
    pub fn plan(&self, plan_id: &str) -> Result<Vec<Task>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT id,plan_id,spec,status,round,report,verdict,base,result FROM tasks
             WHERE plan_id=?1 ORDER BY position",
        )?;
        let rows = statement.query_map([plan_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
            ))
        })?;
        rows.map(|row| {
            let (id, plan_id, spec, status, round, report, verdict, base, result) = row?;
            Ok(Task {
                id,
                plan_id,
                spec: serde_json::from_str(&spec)?,
                status,
                round: round as u32,
                report,
                verdict: verdict.map(|v| serde_json::from_str(&v)).transpose()?,
                base,
                result,
            })
        })
        .collect()
    }

    pub fn update_task(&self, task: &Task) -> Result<()> {
        self.db().execute(
            "UPDATE tasks SET status=?2,round=?3,report=?4,verdict=?5,base=?6,result=?7 WHERE id=?1",
            params![
                task.id,
                task.status,
                task.round as i64,
                task.report,
                task.verdict.as_ref().map(Value::to_string),
                task.base,
                task.result
            ],
        )?;
        Ok(())
    }

    pub fn append_event(
        &self,
        conversation: &str,
        turn: Option<&str>,
        kind: &str,
        payload: &Value,
    ) -> Result<u64> {
        let db = self.db();
        db.execute(
            "INSERT INTO events(conversation_id,turn_id,kind,payload) VALUES(?1,?2,?3,?4)",
            params![conversation, turn, kind, payload.to_string()],
        )?;
        Ok(db.last_insert_rowid() as u64)
    }

    pub fn events_since(&self, conversation: &str, after: u64, limit: usize) -> Result<Vec<Event>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT seq,turn_id,kind,payload FROM events
             WHERE conversation_id=?1 AND seq>?2 ORDER BY seq LIMIT ?3",
        )?;
        let rows =
            statement.query_map(params![conversation, after as i64, limit as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?;
        rows.map(|row| {
            let (seq, turn_id, kind, payload) = row?;
            Ok(Event {
                seq: seq as u64,
                turn_id,
                kind,
                payload: serde_json::from_str(&payload)?,
            })
        })
        .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_and_events_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("core.db")).unwrap();
        let conversation = store.create_conversation("/p").unwrap();
        let first = store
            .add_message(&conversation.id, "user", "cześć")
            .unwrap();
        let turn = store
            .start_turn(&conversation.id, "orchestrator", "codex", first.seq)
            .unwrap();
        store.set_session(&turn.id, "s1").unwrap();
        store
            .finish_turn(&turn.id, TurnStatus::Failed, Some("boom"))
            .unwrap();
        let last = store
            .last_turn(&conversation.id, "orchestrator")
            .unwrap()
            .unwrap();
        assert_eq!(last.status, TurnStatus::Failed);
        assert_eq!(last.error.as_deref(), Some("boom"));
        assert_eq!(
            store
                .last_session(&conversation.id, "orchestrator", "codex")
                .unwrap()
                .as_deref(),
            Some("s1")
        );
        // Another CLI cannot resume a codex session.
        assert_eq!(
            store
                .last_session(&conversation.id, "orchestrator", "agy")
                .unwrap(),
            None
        );
        let events = store.events_since(&conversation.id, 0, 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].payload["text"], "cześć");
        assert!(
            store
                .events_since(&conversation.id, events[0].seq, 10)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn running_turns_become_interrupted_on_restart() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("core.db")).unwrap();
        let conversation = store.create_conversation("/p").unwrap();
        store
            .start_turn(&conversation.id, "orchestrator", "agy", 0)
            .unwrap();
        drop(store);
        let store = Store::open(&dir.path().join("core.db")).unwrap();
        assert_eq!(
            store.interrupt_running_turns().unwrap(),
            [conversation.id.as_str()]
        );
        let turn = store
            .last_turn(&conversation.id, "orchestrator")
            .unwrap()
            .unwrap();
        assert_eq!(turn.status, TurnStatus::Interrupted);
    }
}
