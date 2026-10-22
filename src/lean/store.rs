//! Small rows, each written on its own, plus an append-only event log. Nothing
//! here grows with the size of the project the agents work on.

#[path = "history.rs"]
mod history;
#[path = "workflow.rs"]
mod workflow;
pub use history::{AgentMessage, ContentOwner, HistoryScope};
pub use workflow::Workflow;

use crate::{Error, Result};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::Mutex;

pub struct Store {
    connection: Mutex<Connection>,
    content: super::content::Content,
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
    #[serde(default)]
    pub plan_id: Option<String>,
}

/// One normalized mailbox address with pending mail and its owning task, if any.
pub struct MailTarget {
    pub role: String,
    pub newest: u64,
    pub task_id: Option<String>,
    pub plan_id: Option<String>,
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
    #[serde(default)]
    pub plan_id: Option<String>,
    pub kind: String,
    pub payload: Value,
    /// When it was recorded; 0 for events from before schema 2.
    pub created_ms: i64,
}

/// A message to one agent, delivered into its next turn. `sender` and
/// `recipient` are display names (`user`, `orchestrator`, a task id such as
/// `t2`); `role` is the recipient's session role.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mail {
    pub id: u64,
    pub sender: String,
    pub recipient: String,
    pub kind: String,
    pub text: String,
    #[serde(default)]
    pub created_ms: i64,
    #[serde(default)]
    pub delivered_turn: Option<String>,
    #[serde(default)]
    pub delivered_ms: Option<i64>,
    #[serde(default = "default_mail_state")]
    pub state: String,
}

fn default_mail_state() -> String {
    "queued".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub turn_id: String,
    #[serde(default)]
    pub plan_id: Option<String>,
    pub role: String,
    pub provider: String,
    pub session_id: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub upto_message: u64,
    pub started_ms: i64,
    pub finished_ms: Option<i64>,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanAgentInfo {
    pub role: String,
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
    #[serde(default)]
    pub task_label: Option<String>,
    pub turns_count: usize,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskReportInfo {
    pub task_id: String,
    pub task_label: String,
    pub round: u32,
    pub report: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskVerdictInfo {
    pub task_id: String,
    pub task_label: String,
    pub round: u32,
    pub verdict: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanHistory {
    pub ledger_mode: String,
    pub run_id: Option<String>,
    pub id: String,
    pub conversation_id: String,
    pub created_ms: i64,
    pub status: String,
    pub is_active: bool,
    pub tasks: Vec<Task>,
    pub agents: Vec<PlanAgentInfo>,
    pub turns: Vec<SessionInfo>,
    pub reports: Vec<TaskReportInfo>,
    pub verdicts: Vec<TaskVerdictInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlansPage {
    pub plans: Vec<PlanHistory>,
    pub offset: usize,
    pub limit: usize,
    pub total: usize,
    pub has_more: bool,
    pub active_plan_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryPage {
    pub events: Vec<Event>,
    pub after: u64,
    pub next_after: Option<u64>,
    pub has_more: bool,
    pub total: usize,
    pub oldest_seq: u64,
    pub latest_seq: u64,
    pub history_unavailable: bool,
}

/// The user's comment on one element of the team panel or of a diff.
/// `target` is the element's key (`worker:t2`, `diff:t2:src/a.rs:14`), `label`
/// how the orchestrator reads it, `plan_id` the plan it was made on (a later
/// plan makes it stale), `message` the conversation message that carried it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub id: u64,
    pub plan_id: Option<String>,
    pub target: String,
    pub label: String,
    pub text: String,
    /// `open`, `resolved` or `deleted`.
    pub status: String,
    pub message: u64,
}

/// One task of a plan. `spec` is what the orchestrator asked for; `report`
/// is the worker's latest answer and `verdict` its verifier's latest one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    #[serde(default)]
    pub input_generation: i64,
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
    /// The compact feedback a card shows for the current round: a bounded
    /// effect summary, coverage of the real obligations, and one unambiguous
    /// integration outcome. `None` until a round produces any of it; a new
    /// round or retry clears it, so a card never shows an earlier round's data.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feedback: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_token: Option<String>,
    /// What the verifier has settled so far in the current round, one entry
    /// per check (`id`, `state`, `status`, `note`). Filled when a task is
    /// read for a frontend, never stored on the task row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checks: Option<Value>,
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

/// The layout of `core.db` this build writes (`PRAGMA user_version`).
/// Tables only ever get added; a change to an existing one raises it and
/// migrates from the version found.
const SCHEMA: u32 = 9;

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let found: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        crate::require(
            found <= SCHEMA,
            &format!(
                "{} was written by a newer antislop (schema {found}, this one knows {SCHEMA})",
                path.display()
            ),
        )?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;
             CREATE TABLE IF NOT EXISTS conversations(
               id TEXT PRIMARY KEY, project TEXT NOT NULL, created_ms INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS session_workflow(
               conversation_id TEXT PRIMARY KEY REFERENCES conversations(id),
               phase TEXT NOT NULL, summary TEXT NOT NULL DEFAULT '', upto_message INTEGER NOT NULL DEFAULT 0);
             CREATE TABLE IF NOT EXISTS messages(
               seq INTEGER PRIMARY KEY AUTOINCREMENT,
               conversation_id TEXT NOT NULL REFERENCES conversations(id),
               author TEXT NOT NULL, text TEXT NOT NULL, created_ms INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS turns(
               id TEXT PRIMARY KEY,
               conversation_id TEXT NOT NULL REFERENCES conversations(id),
               plan_id TEXT,
               role TEXT NOT NULL, provider TEXT NOT NULL, session_id TEXT,
               status TEXT NOT NULL, error TEXT, upto_message INTEGER NOT NULL,
               started_ms INTEGER NOT NULL, finished_ms INTEGER, pid INTEGER);
             CREATE TABLE IF NOT EXISTS events(
               seq INTEGER PRIMARY KEY AUTOINCREMENT,
               conversation_id TEXT NOT NULL REFERENCES conversations(id),
               turn_id TEXT, plan_id TEXT, kind TEXT NOT NULL, payload TEXT NOT NULL,
               created_ms INTEGER NOT NULL DEFAULT 0);
             CREATE INDEX IF NOT EXISTS messages_by_conversation ON messages(conversation_id, seq);
             CREATE INDEX IF NOT EXISTS turns_by_conversation ON turns(conversation_id, role, started_ms);
             CREATE INDEX IF NOT EXISTS events_by_conversation ON events(conversation_id, seq);
             CREATE TABLE IF NOT EXISTS tasks(
                id TEXT PRIMARY KEY,
                conversation_id TEXT NOT NULL REFERENCES conversations(id),
                plan_id TEXT NOT NULL, position INTEGER NOT NULL, spec TEXT NOT NULL,
                status TEXT NOT NULL, round INTEGER NOT NULL DEFAULT 0,
                report TEXT, verdict TEXT, base TEXT, result TEXT, feedback TEXT,
                created_ms INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS tasks_by_plan ON tasks(conversation_id, plan_id, position);
             CREATE INDEX IF NOT EXISTS tasks_by_plan_position ON tasks(plan_id, position);
             CREATE TABLE IF NOT EXISTS mail(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               conversation_id TEXT NOT NULL REFERENCES conversations(id),
               sender TEXT NOT NULL, recipient TEXT NOT NULL, kind TEXT NOT NULL,
               text TEXT NOT NULL, created_ms INTEGER NOT NULL,
               delivered_turn TEXT, delivered_ms INTEGER);
             CREATE INDEX IF NOT EXISTS mail_by_recipient
               ON mail(conversation_id, recipient, delivered_turn);
             CREATE TABLE IF NOT EXISTS comments(
               id INTEGER PRIMARY KEY AUTOINCREMENT,
               conversation_id TEXT NOT NULL REFERENCES conversations(id),
               plan_id TEXT, target TEXT NOT NULL, label TEXT NOT NULL, text TEXT NOT NULL,
               status TEXT NOT NULL, message INTEGER NOT NULL,
               created_ms INTEGER NOT NULL, updated_ms INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS checks(
               task_id TEXT NOT NULL, round INTEGER NOT NULL, check_id TEXT NOT NULL,
               state TEXT NOT NULL, status TEXT, note TEXT NOT NULL DEFAULT '',
               updated_ms INTEGER NOT NULL,
               PRIMARY KEY(task_id, round, check_id));
             CREATE TABLE IF NOT EXISTS pauses(
               conversation_id TEXT PRIMARY KEY REFERENCES conversations(id));
             CREATE TABLE IF NOT EXISTS modes(
               conversation_id TEXT PRIMARY KEY REFERENCES conversations(id),
               autonomous INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS trusted_projects(
               project TEXT PRIMARY KEY,
               trusted_ms INTEGER NOT NULL);",
        )?;
        // 1 → 2: events got their time; earlier ones keep 0, unknown.
        let timed: bool = connection.query_row(
            "SELECT count(*) FROM pragma_table_info('events') WHERE name='created_ms'",
            [],
            |row| row.get(0),
        )?;
        if !timed {
            connection.execute_batch(
                "ALTER TABLE events ADD COLUMN created_ms INTEGER NOT NULL DEFAULT 0",
            )?;
        }
        // 2 → 3: turns record their CLI's process group.
        let tracked: bool = connection.query_row(
            "SELECT count(*) FROM pragma_table_info('turns') WHERE name='pid'",
            [],
            |row| row.get(0),
        )?;
        if !tracked {
            connection.execute_batch("ALTER TABLE turns ADD COLUMN pid INTEGER")?;
        }
        // 3 → 4: events, turns and comments associate with plan; plans table backfilled.
        let event_plan: bool = connection.query_row(
            "SELECT count(*) FROM pragma_table_info('events') WHERE name='plan_id'",
            [],
            |row| row.get(0),
        )?;
        if !event_plan {
            connection.execute_batch("ALTER TABLE events ADD COLUMN plan_id TEXT")?;
        }
        let turn_plan: bool = connection.query_row(
            "SELECT count(*) FROM pragma_table_info('turns') WHERE name='plan_id'",
            [],
            |row| row.get(0),
        )?;
        if !turn_plan {
            connection.execute_batch("ALTER TABLE turns ADD COLUMN plan_id TEXT")?;
        }
        let comment_plan: bool = connection.query_row(
            "SELECT count(*) FROM pragma_table_info('comments') WHERE name='plan_id'",
            [],
            |row| row.get(0),
        )?;
        if !comment_plan {
            connection.execute_batch("ALTER TABLE comments ADD COLUMN plan_id TEXT")?;
        }
        // 4 → 5: tasks carry compact card feedback, tied to its round.
        // Older rows keep `None`: their cards show no result, not a wrong one.
        let card_feedback: bool = connection.query_row(
            "SELECT count(*) FROM pragma_table_info('tasks') WHERE name='feedback'",
            [],
            |row| row.get(0),
        )?;
        if !card_feedback {
            connection.execute_batch("ALTER TABLE tasks ADD COLUMN feedback TEXT")?;
        }
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS plans(
               id TEXT PRIMARY KEY,
               conversation_id TEXT NOT NULL REFERENCES conversations(id),
               created_ms INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS plans_by_conversation ON plans(conversation_id, created_ms);
             CREATE INDEX IF NOT EXISTS events_by_plan ON events(conversation_id, plan_id, seq);
             CREATE INDEX IF NOT EXISTS turns_by_plan ON turns(conversation_id, plan_id, started_ms);
             INSERT OR IGNORE INTO plans(id, conversation_id, created_ms)
               SELECT plan_id, conversation_id, MIN(created_ms) FROM tasks GROUP BY plan_id, conversation_id;
             UPDATE turns SET plan_id = (
               SELECT plan_id FROM tasks WHERE tasks.id = substr(turns.role, 8)
             ) WHERE turns.plan_id IS NULL AND turns.role LIKE 'worker:%';
             UPDATE turns SET plan_id = (
               SELECT plan_id FROM tasks WHERE tasks.id = substr(turns.role, 10)
             ) WHERE turns.plan_id IS NULL AND turns.role LIKE 'verifier:%';
             UPDATE events SET plan_id = (
               SELECT plan_id FROM turns WHERE turns.id = events.turn_id AND turns.plan_id IS NOT NULL
             ) WHERE events.plan_id IS NULL AND events.turn_id IS NOT NULL;
             UPDATE events SET plan_id = (
               SELECT plan_id FROM tasks WHERE tasks.id = json_extract(events.payload, '$.task')
             ) WHERE events.plan_id IS NULL AND events.kind = 'task' AND json_extract(events.payload, '$.task') IS NOT NULL;
             UPDATE events SET plan_id = json_extract(events.payload, '$.plan')
             WHERE events.plan_id IS NULL AND json_extract(events.payload, '$.plan') IS NOT NULL;",
        )?;
        // 5 → 6: global daemon events share the readable event stream.
        if found < 6 {
            let transaction = connection.unchecked_transaction()?;
            transaction.execute_batch("CREATE TABLE events_v6(seq INTEGER PRIMARY KEY AUTOINCREMENT,
                    conversation_id TEXT REFERENCES conversations(id), turn_id TEXT, plan_id TEXT,
                    kind TEXT NOT NULL, payload TEXT NOT NULL, created_ms INTEGER NOT NULL DEFAULT 0);
                INSERT INTO events_v6 SELECT seq,conversation_id,turn_id,plan_id,kind,payload,created_ms FROM events;
                DROP TABLE events;
                ALTER TABLE events_v6 RENAME TO events;
                CREATE INDEX events_by_conversation ON events(conversation_id,seq);
                CREATE INDEX events_by_plan ON events(conversation_id,plan_id,seq);")?;
            transaction.pragma_update(None, "user_version", 6)?;
            transaction.commit()?;
        }
        super::ledger::migrate(&connection)?;
        Self::migrate_history(&connection)?;
        connection.pragma_update(None, "user_version", SCHEMA)?;
        Ok(Self {
            connection: Mutex::new(connection),
            content: super::content::Content::new(path),
        })
    }

    pub(super) fn decode_content(&self, text: &str) -> Result<String> {
        self.content.decode(text)
    }

    pub(super) fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
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

    pub fn is_trusted(&self, project: &str) -> Result<bool> {
        let count: i64 = self.db().query_row(
            "SELECT count(*) FROM trusted_projects WHERE project=?1",
            [project],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    pub fn trust_project(&self, project: &str) -> Result<()> {
        self.db().execute(
            "INSERT OR REPLACE INTO trusted_projects(project, trusted_ms) VALUES(?1, ?2)",
            params![project, now_ms()],
        )?;
        Ok(())
    }

    pub fn untrust_project(&self, project: &str) -> Result<()> {
        self.db()
            .execute("DELETE FROM trusted_projects WHERE project=?1", [project])?;
        Ok(())
    }

    pub fn trusted_projects(&self) -> Result<Vec<String>> {
        let db = self.db();
        let mut statement =
            db.prepare("SELECT project FROM trusted_projects ORDER BY trusted_ms DESC")?;
        let rows = statement.query_map([], |row| row.get(0))?;
        let mut projects = Vec::new();
        for row in rows {
            projects.push(row?);
        }
        Ok(projects)
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
        let stored_text = self.content.encode(text)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,?2,?3,?4)",
            params![conversation, author, stored_text, now_ms()],
        )?;
        let seq = tx.last_insert_rowid() as u64;
        tx.execute(
            "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,NULL,NULL,'message',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
            params![
                conversation,
                serde_json::json!({"seq":seq,"author":author,"text":stored_text}).to_string()
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
                text: self.content.sql_text(row, 2)?,
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
                text: self.content.sql_text(row, 2)?,
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
        plan_id: Option<&str>,
    ) -> Result<Turn> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let resolved_plan: Option<String> = match plan_id {
            Some(pid) => Some(pid.to_string()),
            None => {
                if let Some(task_uuid) = super::task_role(role).map(|(id, _)| id) {
                    tx.query_row(
                        "SELECT plan_id FROM tasks WHERE id=?1",
                        [task_uuid],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .optional()?
                    .flatten()
                } else {
                    None
                }
            }
        };
        let turn = Turn {
            id: crate::id(),
            role: role.into(),
            status: TurnStatus::Running,
            session_id: None,
            error: None,
            upto_message,
            plan_id: resolved_plan.clone(),
        };
        tx.execute(
            "INSERT INTO turns(id,conversation_id,plan_id,role,provider,status,upto_message,started_ms)
             VALUES(?1,?2,?3,?4,?5,'running',?6,?7)",
            params![
                turn.id,
                conversation,
                resolved_plan,
                role,
                provider,
                upto_message as i64,
                now_ms()
            ],
        )?;
        let scope = super::task_role(role).map(|(task, _)| task);
        tx.execute(
            "INSERT INTO turn_scopes(turn_id,task_id,round)
            SELECT ?1,id,round FROM tasks WHERE id=?2 AND conversation_id=?3 AND plan_id=?4",
            params![turn.id, scope, conversation, resolved_plan],
        )?;
        tx.commit()?;
        Ok(turn)
    }

    pub fn set_session(&self, turn: &str, session: &str) -> Result<()> {
        self.db().execute(
            "UPDATE turns SET session_id=?2 WHERE id=?1",
            params![turn, session],
        )?;
        Ok(())
    }

    pub fn set_turn_pid(&self, turn: &str, pid: u32) -> Result<()> {
        self.db()
            .execute("UPDATE turns SET pid=?2 WHERE id=?1", params![turn, pid])?;
        Ok(())
    }

    /// The CLIs of turns still marked running: (process group, turn start,
    /// provider). Read before those turns become interrupted.
    pub fn running_processes(&self) -> Result<Vec<(u32, i64, String)>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT pid, started_ms, provider FROM turns
             WHERE status='running' AND pid IS NOT NULL",
        )?;
        let rows = statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
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
        self.last_turn_for_roles(conversation, role, role)
    }

    pub fn last_orchestration_turn(&self, conversation: &str) -> Result<Option<Turn>> {
        self.last_turn_for_roles(
            conversation,
            super::ORCHESTRATOR,
            super::COMMENT_ORCHESTRATOR,
        )
    }

    fn last_turn_for_roles(
        &self,
        conversation: &str,
        role: &str,
        other: &str,
    ) -> Result<Option<Turn>> {
        self.db()
            .query_row(
                "SELECT id,role,status,session_id,error,upto_message,plan_id FROM turns
                 WHERE conversation_id=?1 AND role IN (?2,?3) ORDER BY started_ms DESC, rowid DESC LIMIT 1",
                params![conversation, role, other],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, Option<String>>(6)?,
                    ))
                },
            )
            .optional()?
            .map(|(id, role, status, session_id, error, upto, plan_id)| {
                Ok(Turn {
                    id,
                    role,
                    status: TurnStatus::parse(&status)?,
                    session_id,
                    error,
                    upto_message: upto as u64,
                    plan_id,
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

    pub fn last_orchestration_answered(&self, conversation: &str) -> Result<u64> {
        Ok(self
            .last_answered(conversation, super::ORCHESTRATOR)?
            .max(self.last_answered(conversation, super::COMMENT_ORCHESTRATOR)?))
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
                 WHERE status='running' AND role IN ('orchestrator','comment-orchestrator')",
            )?;
            statement
                .query_map([], |row| row.get(0))?
                .collect::<std::result::Result<Vec<String>, _>>()?
        };
        let interrupted = {
            let mut statement = tx.prepare(
                "SELECT id,conversation_id,plan_id,role FROM turns WHERE status='running'",
            )?;
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (turn, conversation, plan, role) in interrupted {
            tx.execute(
                "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,?3,'turn_finished',?4,?5)",
                params![conversation, turn, plan, serde_json::json!({"role": role, "status": "interrupted", "error": "the daemon stopped while this turn ran"}).to_string(), now_ms()],
            )?;
        }
        tx.execute(
            "UPDATE turns SET status='interrupted',finished_ms=?1 WHERE status='running'",
            [now_ms()],
        )?;
        tx.commit()?;
        Ok(conversations)
    }

    /// Unfinished tasks stay queued with the interruption reason. Returns each
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
        let interrupted = {
            let mut statement = tx.prepare(
                "SELECT id,report FROM tasks WHERE status IN ('queued','working','verifying')",
            )?;
            statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (id, report) in interrupted {
            let mut text = report
                .as_deref()
                .map(|text| self.content.decode(text))
                .transpose()?
                .unwrap_or_default();
            if !text.is_empty() {
                text.push_str("\n\n");
            }
            text.push_str("[interrupted] the daemon stopped while this task ran");
            tx.execute(
                "UPDATE tasks SET status='queued',report=?2 WHERE id=?1",
                params![id, self.content.encode(&text)?],
            )?;
        }
        tx.commit()?;
        Ok(plans)
    }

    /// Queues again every task of a plan that ended without approval, keeping
    /// its rounds, report, verdict and work. Returns how many.
    pub fn requeue_plan(&self, conversation: &str, plan_id: &str) -> Result<usize> {
        Ok(self.db().execute(
            "UPDATE tasks SET status='queued' WHERE conversation_id=?1 AND plan_id=?2
               AND (status IN ('cancelled','failed','needs_user','skipped','waiting','queued','working','verifying')
                    OR (status='approved' AND round=0))",
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
        let changed = tx.execute("UPDATE session_workflow SET phase='implementation' WHERE conversation_id=?1 AND phase<>'interview'", [conversation])?;
        if changed > 0 {
            tx.execute("INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'workflow',?3,?4)", params![conversation,plan_id,serde_json::json!({"phase":"implementation"}).to_string(),now_ms()])?;
        }
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
        let now = now_ms();
        tx.execute(
            "INSERT INTO plans(id,conversation_id,created_ms) VALUES(?1,?2,?3)",
            params![plan_id, conversation, now],
        )?;
        let mut tasks = vec![];
        for (position, spec) in specs.iter().enumerate() {
            let task = Task {
                input_generation: 0,
                id: crate::id(),
                plan_id: plan_id.clone(),
                spec: spec.clone(),
                status: "proposed".into(),
                round: 0,
                report: None,
                verdict: None,
                base: None,
                result: None,
                feedback: None,
                freshness_token: None,
                checks: None,
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
                    now
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

    /// The latest plan of the conversation other than `except` that was
    /// accepted and ran (not only proposed or sent back).
    pub fn last_plan_that_ran(
        &self,
        conversation: &str,
        except: &str,
    ) -> Result<Option<Vec<Task>>> {
        let plan_id: Option<String> = self
            .db()
            .query_row(
                "SELECT plan_id FROM tasks WHERE conversation_id=?1 AND plan_id<>?2
                   AND status NOT IN ('proposed','revised')
                 ORDER BY created_ms DESC, rowid DESC LIMIT 1",
                params![conversation, except],
                |row| row.get(0),
            )
            .optional()?;
        plan_id.map(|plan_id| self.plan(&plan_id)).transpose()
    }

    /// Tasks of one plan, in the orchestrator's order.
    pub fn plan(&self, plan_id: &str) -> Result<Vec<Task>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT id,plan_id,spec,status,round,report,verdict,base,result,feedback FROM tasks
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
                row.get::<_, Option<String>>(9)?,
            ))
        })?;
        rows.map(|row| {
            let (id, plan_id, spec, status, round, report, verdict, base, result, feedback) = row?;
            Ok(Task {
                input_generation: db
                    .query_row(
                        "SELECT generation FROM task_inputs WHERE task_id=?1",
                        [&id],
                        |r| r.get(0),
                    )
                    .optional()?
                    .unwrap_or(0),
                id,
                plan_id,
                spec: serde_json::from_str(&spec)?,
                status,
                round: round as u32,
                report: report
                    .as_deref()
                    .map(|text| self.content.decode(text))
                    .transpose()?,
                verdict: verdict
                    .map(|v| {
                        self.content
                            .decode(&v)
                            .and_then(|text| Ok(serde_json::from_str(&text)?))
                    })
                    .transpose()?,
                base,
                result,
                feedback: feedback.map(|v| serde_json::from_str(&v)).transpose()?,
                freshness_token: None,
                checks: None,
            })
        })
        .collect()
    }

    /// Records how far a verifier is with one check of a round: `checking`
    /// while it works on it, `settled` with its status and evidence. A check
    /// already settled does not go back to `checking`; a note stays unless a
    /// new one replaces it. Returns whether the row changed.
    pub fn set_check(
        &self,
        task: &str,
        round: u32,
        check: &str,
        state: &str,
        status: Option<&str>,
        note: &str,
    ) -> Result<bool> {
        let db = self.db();
        let before: Option<(String, Option<String>, String)> = db
            .query_row(
                "SELECT state,status,note FROM checks WHERE task_id=?1 AND round=?2 AND check_id=?3",
                params![task, round, check],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let (state, status, note) = match before {
            Some((old_state, ..)) if old_state == "settled" && state == "checking" => {
                return Ok(false);
            }
            Some((_, old_status, old_note)) => (
                state,
                status.map(str::to_owned).or(old_status),
                if note.is_empty() {
                    old_note
                } else {
                    note.to_owned()
                },
            ),
            None => (state, status.map(str::to_owned), note.to_owned()),
        };
        let changed = db.execute(
            "INSERT INTO checks(task_id,round,check_id,state,status,note,updated_ms)
             VALUES(?1,?2,?3,?4,?5,?6,?7)
             ON CONFLICT(task_id,round,check_id) DO UPDATE SET
               state=?4,status=?5,note=?6,updated_ms=?7
             WHERE state IS NOT ?4 OR status IS NOT ?5 OR note IS NOT ?6",
            params![task, round, check, state, status, note, now_ms()],
        )?;
        Ok(changed > 0)
    }

    /// The checks of one round of a task, as JSON entries.
    pub fn checks(&self, task: &str, round: u32) -> Result<Vec<Value>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT check_id,state,status,note FROM checks
             WHERE task_id=?1 AND round=?2 ORDER BY updated_ms, check_id",
        )?;
        let rows = statement.query_map(params![task, round], |row| {
            Ok(serde_json::json!({
                "id": row.get::<_, String>(0)?, "state": row.get::<_, String>(1)?,
                "status": row.get::<_, Option<String>>(2)?, "note": row.get::<_, String>(3)?,
            }))
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn update_task(&self, task: &Task) -> Result<()> {
        let stored_report = task
            .report
            .as_deref()
            .map(|text| self.content.encode(text))
            .transpose()?;
        let stored_verdict = task
            .verdict
            .as_ref()
            .map(|value| self.content.encode(&value.to_string()))
            .transpose()?;
        let mut snapshot = serde_json::to_value(task)?;
        for (field, encoded) in [("report", &stored_report), ("verdict", &stored_verdict)] {
            if let Some(encoded) = encoded {
                let reference: Value = serde_json::from_str(
                    encoded
                        .strip_prefix("\0antislop-content:")
                        .expect("encoded content"),
                )?;
                snapshot[field] = serde_json::json!({"session_content":reference});
            }
        }
        let content = self.content.write(&serde_json::to_string(&snapshot)?)?;
        let mut db = self.db();
        let transaction = db.transaction()?;
        transaction.execute("UPDATE task_rounds SET status='continued' WHERE task_id=?1 AND round=(SELECT round FROM tasks WHERE id=?1) AND round<?2 AND status IN ('working','verifying','queued')",params![task.id,task.round])?;
        let changed = transaction.execute(
            "UPDATE tasks SET status=?2,round=?3,report=?4,verdict=?5,base=?6,result=?7,feedback=?8 WHERE id=?1 AND coalesce((SELECT generation FROM task_inputs WHERE task_id=?1),0)=?9",
            params![
                task.id,
                task.status,
                task.round as i64,
                stored_report,
                stored_verdict,
                task.base,
                task.result,
                task.feedback.as_ref().map(Value::to_string),
                task.input_generation
            ],
        )?;
        crate::require(
            changed == 1,
            "stale task generation; result retained in turn history",
        )?;
        transaction.execute("INSERT INTO task_rounds(task_id,round,status,content) VALUES(?1,?2,?3,?4)
            ON CONFLICT(task_id,round) DO UPDATE SET status=excluded.status,content=excluded.content",
            params![task.id, task.round, task.status, serde_json::to_string(&content)?])?;
        transaction.commit()?;
        Ok(())
    }

    /// Approved writers for this project, most recently approved first.
    /// Task events give completion order, including writers from older plans
    /// and conversations; creation order need not match execution order.
    pub fn approved_writer_results(&self, project: &str) -> Result<Vec<(String, String)>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT t.base,t.result FROM events e
             JOIN tasks t ON t.id=json_extract(e.payload,'$.task')
             JOIN conversations c ON c.id=t.conversation_id
             WHERE c.project=?1 AND e.conversation_id=t.conversation_id
               AND e.kind='task' AND json_extract(e.payload,'$.status')='approved'
               AND t.status='approved' AND t.base IS NOT NULL AND t.result IS NOT NULL
             GROUP BY t.id ORDER BY max(e.seq) DESC",
        )?;
        let rows = statement.query_map([project], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Records a message for `role` and announces it.
    pub fn add_mail(&self, conversation: &str, role: &str, mail: &Mail) -> Result<Mail> {
        self.add_mail_from(conversation, role, mail, None)
    }

    pub fn add_mail_from(
        &self,
        conversation: &str,
        role: &str,
        mail: &Mail,
        source_turn: Option<&str>,
    ) -> Result<Mail> {
        let stored_text = self.content.encode(&mail.text)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let now = now_ms();
        tx.execute(
            "INSERT INTO mail(conversation_id,sender,recipient,kind,text,created_ms)
             VALUES(?1,?2,?3,?4,?5,?6)",
            params![conversation, mail.sender, role, mail.kind, stored_text, now],
        )?;
        let mail = Mail {
            id: tx.last_insert_rowid() as u64,
            created_ms: now,
            delivered_turn: None,
            delivered_ms: None,
            state: "queued".into(),
            ..mail.clone()
        };
        if let Some(turn) = source_turn {
            tx.execute("INSERT INTO mail_sources(mail_id,turn_id) SELECT ?1,id FROM turns WHERE id=?2 AND conversation_id=?3",params![mail.id as i64,turn,conversation])?;
        }
        let mut payload = serde_json::to_value(&mail)?;
        payload["text"] = stored_text.into();
        tx.execute(
            "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,NULL,NULL,'mail',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
            params![conversation, payload.to_string()],
        )?;
        tx.commit()?;
        Ok(mail)
    }

    /// Stores a comment with the message that tells the orchestrator about
    /// it (`text` gets the comment's id), in one transaction.
    pub fn add_comment(
        &self,
        conversation: &str,
        comment: &Comment,
        text: impl Fn(u64) -> String,
    ) -> Result<Comment> {
        self.add_comment_at(conversation, comment, None, None, text)
    }

    pub fn add_comment_at(
        &self,
        conversation: &str,
        comment: &Comment,
        round: Option<u32>,
        run: Option<&str>,
        text: impl Fn(u64) -> String,
    ) -> Result<Comment> {
        let stored_text = self.content.encode(&comment.text)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let now = now_ms();
        tx.execute(
            "INSERT INTO comments(conversation_id,plan_id,target,label,text,status,message,
                                  created_ms,updated_ms)
             VALUES(?1,?2,?3,?4,?5,'open',0,?6,?6)",
            params![
                conversation,
                comment.plan_id,
                comment.target,
                comment.label,
                stored_text,
                now
            ],
        )?;
        let id = tx.last_insert_rowid() as u64;
        let text = self.content.encode(&text(id))?;
        tx.execute(
            "INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'user',?2,?3)",
            params![conversation, text, now],
        )?;
        let message = tx.last_insert_rowid() as u64;
        tx.execute(
            "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms)
             VALUES(?1,NULL,?3,'message',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
            params![
                conversation,
                serde_json::json!({"seq": message, "author": "user", "text": text}).to_string(),
                comment.plan_id
            ],
        )?;
        tx.execute(
            "UPDATE comments SET message=?1 WHERE id=?2",
            params![message as i64, id as i64],
        )?;
        let comment = Comment {
            id,
            status: "open".into(),
            message,
            ..comment.clone()
        };
        tx.execute(
            "INSERT INTO comment_scopes(comment_id,round,run) VALUES(?1,?2,?3)",
            params![comment.id as i64, round, run],
        )?;
        let mut payload = self.comment_payload(&comment)?;
        payload["round"] = serde_json::to_value(round)?;
        payload["run"] = serde_json::to_value(run)?;
        tx.execute(
            "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,NULL,?3,'comment',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
            params![conversation, payload.to_string(), comment.plan_id],
        )?;
        tx.commit()?;
        Ok(comment)
    }

    /// Stores a comment on the plan as a draft: the user is still writing,
    /// and the orchestrator hears of it only when the user sends the review.
    pub fn add_draft_comment(&self, conversation: &str, comment: &Comment) -> Result<Comment> {
        let stored_text = self.content.encode(&comment.text)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let now = now_ms();
        tx.execute(
            "INSERT INTO comments(conversation_id,plan_id,target,label,text,status,message,
                                  created_ms,updated_ms)
             VALUES(?1,?2,?3,?4,?5,'draft',0,?6,?6)",
            params![
                conversation,
                comment.plan_id,
                comment.target,
                comment.label,
                stored_text,
                now
            ],
        )?;
        let comment = Comment {
            id: tx.last_insert_rowid() as u64,
            status: "draft".into(),
            message: 0,
            ..comment.clone()
        };
        tx.execute(
            "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,NULL,?3,'comment',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
            params![conversation, self.comment_payload(&comment)?.to_string(), comment.plan_id],
        )?;
        tx.commit()?;
        Ok(comment)
    }

    /// Hands every draft comment on `plan_id` to the orchestrator as one
    /// message, so that they are answered together in one turn.
    pub fn send_draft_comments(&self, conversation: &str, plan_id: &str) -> Result<Vec<Comment>> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let drafts = self.comment_rows(
            &tx,
            "conversation_id=?1 AND status='draft' AND (plan_id=?2 OR plan_id IS NULL)",
            params![conversation, plan_id],
        )?;
        if drafts.is_empty() {
            return Err(Error::Invalid("there are no draft comments to send".into()));
        }
        let now = now_ms();
        let text = drafts
            .iter()
            .map(|c| {
                format!(
                    "{}{} on {}] {}",
                    super::COMMENT_PREFIX,
                    c.id,
                    c.label,
                    c.text
                )
            })
            .collect::<Vec<_>>()
            .join("\n\n");
        let text = self.content.encode(&text)?;
        tx.execute(
            "INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'user',?2,?3)",
            params![conversation, text, now],
        )?;
        let message = tx.last_insert_rowid() as u64;
        tx.execute(
            "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms)
             VALUES(?1,NULL,?3,'message',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
            params![
                conversation,
                serde_json::json!({"seq": message, "author": "user", "text": text}).to_string(),
                plan_id
            ],
        )?;
        let mut sent = Vec::new();
        for mut comment in drafts {
            tx.execute(
                "UPDATE comments SET status='open', message=?1, updated_ms=?2 WHERE id=?3",
                params![message as i64, now, comment.id as i64],
            )?;
            comment.status = "open".into();
            comment.message = message;
            tx.execute(
                "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,NULL,?3,'comment',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
                params![conversation, self.comment_payload(&comment)?.to_string(), comment.plan_id],
            )?;
            sent.push(comment);
        }
        tx.commit()?;
        Ok(sent)
    }

    pub fn set_comment_status(&self, conversation: &str, id: u64, status: &str) -> Result<Comment> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let changed = tx.execute(
            "UPDATE comments SET status=?1, updated_ms=?2 WHERE id=?3 AND conversation_id=?4",
            params![status, now_ms(), id as i64, conversation],
        )?;
        if changed == 0 {
            return Err(Error::Invalid(format!("no comment {id}")));
        }
        let mut comment = self
            .comment_rows(&tx, "id=?1", params![id as i64])?
            .pop()
            .ok_or_else(|| Error::Invalid(format!("no comment {id}")))?;
        if comment.message == 0 && status != "deleted" {
            // A draft has not been sent: it can be deleted, and only the
            // review sends it.
            return Err(Error::Invalid(
                "this comment is a draft: delete it, or send it with Review".into(),
            ));
        }
        if status == "open" {
            let text = format!(
                "{}{} on {}] {}",
                super::COMMENT_PREFIX,
                id,
                comment.label,
                comment.text
            );
            let text = self.content.encode(&text)?;
            tx.execute("INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,'user',?2,?3)", params![conversation, text, now_ms()])?;
            comment.message = tx.last_insert_rowid() as u64;
            tx.execute(
                "UPDATE comments SET message=?1 WHERE id=?2",
                params![comment.message as i64, id as i64],
            )?;
            tx.execute(
                "INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'message',?3,?4)",
                params![conversation, comment.plan_id, serde_json::json!({"seq": comment.message, "author": "user", "text": text}).to_string(), now_ms()],
            )?;
        }
        tx.execute(
            "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,NULL,?3,'comment',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
            params![conversation, self.comment_payload(&comment)?.to_string(), comment.plan_id],
        )?;
        tx.commit()?;
        Ok(comment)
    }

    /// Resolve only plan feedback included in the successful revision.
    pub fn resolve_plan_comments(&self, conversation: &str, upto: u64) -> Result<()> {
        self.resolve_listed_plan_comments(conversation, upto, None)
    }

    /// Like [`Self::resolve_plan_comments`], but only the comments in `only`
    /// when given: the ones the comment agent said its plan took care of.
    pub fn resolve_listed_plan_comments(
        &self,
        conversation: &str,
        upto: u64,
        only: Option<&[u64]>,
    ) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let comments = self.comment_rows(
            &tx,
            "conversation_id=?1 AND status='open' AND message<=?2 AND (target='plan' OR target LIKE 'plan:%')",
            params![conversation, upto as i64],
        )?;
        for mut comment in comments {
            if only.is_some_and(|ids| !ids.contains(&comment.id)) {
                continue;
            }
            tx.execute(
                "UPDATE comments SET status='resolved',updated_ms=?1 WHERE id=?2",
                params![now_ms(), comment.id as i64],
            )?;
            comment.status = "resolved".into();
            tx.execute("INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'comment',?3,?4)",
                params![conversation, comment.plan_id, serde_json::to_string(&comment)?, now_ms()])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// A selective comment claim survives deferred options/requirements planning,
    /// but never a published plan or a subsequent user message.
    pub fn pending_plan_comment_ids(&self, conversation: &str) -> Result<Option<Vec<u64>>> {
        let encoded: Option<String> = self.db().query_row(
            "SELECT json_extract(e.payload,'$.ids') FROM events e
             WHERE e.conversation_id=?1 AND e.kind='plan_comments_addressed'
             AND NOT EXISTS(SELECT 1 FROM events p WHERE p.conversation_id=?1 AND p.kind='plan_proposed' AND p.seq>e.seq)
             AND NOT EXISTS(SELECT 1 FROM messages m WHERE m.conversation_id=?1 AND m.author='user' AND m.seq>json_extract(e.payload,'$.upto_message'))
             ORDER BY e.seq DESC LIMIT 1",
            params![conversation], |row| row.get(0),
        ).optional()?;
        encoded
            .map(|text| serde_json::from_str(&text).map_err(Error::from))
            .transpose()
    }

    /// The rendered plan document is runtime context, never repository contents.
    pub fn plan_document(&self, conversation: &str, plan: &str) -> Result<Option<String>> {
        let document = self.db().query_row(
            "SELECT json_extract(payload,'$.document') FROM events WHERE conversation_id=?1 AND plan_id=?2 AND kind='plan_proposed' ORDER BY seq DESC LIMIT 1",
            params![conversation, plan], |row| row.get::<_, Option<String>>(0),
        ).optional()?.flatten();
        if document.is_some() {
            return Ok(document);
        }
        // A rejected candidate has task rows but no published document yet.
        // Both orchestration sessions need its specs when revising it.
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT spec FROM tasks WHERE conversation_id=?1 AND plan_id=?2 ORDER BY position",
        )?;
        let specs = statement
            .query_map(params![conversation, plan], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if specs.is_empty() {
            Ok(None)
        } else {
            Ok(Some(format!("[{}]", specs.join(","))))
        }
    }

    /// Pauses or continues the conversation's plan work, with its event.
    pub fn set_paused(&self, conversation: &str, paused: bool) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let changed = if paused {
            tx.execute(
                "INSERT OR IGNORE INTO pauses(conversation_id) VALUES(?1)",
                [conversation],
            )?
        } else {
            tx.execute(
                "DELETE FROM pauses WHERE conversation_id=?1",
                [conversation],
            )?
        };
        if changed > 0 {
            tx.execute(
                "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,NULL,NULL,'paused',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
                params![conversation, serde_json::json!({"paused": paused}).to_string()],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Sets the conversation's mode for what it does from now on, with its
    /// event when it changes.
    pub fn set_mode(&self, conversation: &str, autonomous: bool) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let before: Option<bool> = tx
            .query_row(
                "SELECT autonomous FROM modes WHERE conversation_id=?1",
                [conversation],
                |row| row.get(0),
            )
            .optional()?;
        if before != Some(autonomous) {
            tx.execute(
                "INSERT INTO modes(conversation_id,autonomous) VALUES(?1,?2)
                 ON CONFLICT(conversation_id) DO UPDATE SET autonomous=excluded.autonomous",
                params![conversation, autonomous],
            )?;
            tx.execute(
                "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,NULL,NULL,'mode',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
                params![
                    conversation,
                    serde_json::json!({"autonomous": autonomous}).to_string()
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The mode the user set for the conversation, if any.
    pub fn mode(&self, conversation: &str) -> Result<Option<bool>> {
        Ok(self
            .db()
            .query_row(
                "SELECT autonomous FROM modes WHERE conversation_id=?1",
                [conversation],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn paused(&self, conversation: &str) -> Result<bool> {
        Ok(self
            .db()
            .query_row(
                "SELECT 1 FROM pauses WHERE conversation_id=?1",
                [conversation],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }

    /// The conversation's comments with that status, oldest first.
    pub fn comments(&self, conversation: &str, status: &str) -> Result<Vec<Comment>> {
        self.comment_rows(
            &self.db(),
            "conversation_id=?1 AND status=?2",
            params![conversation, status],
        )
    }

    fn comment_payload(&self, comment: &Comment) -> Result<Value> {
        let mut payload = serde_json::to_value(comment)?;
        payload["text"] = self.content.encode(&comment.text)?.into();
        Ok(payload)
    }

    fn comment_rows(
        &self,
        db: &Connection,
        filter: &str,
        values: impl rusqlite::Params,
    ) -> Result<Vec<Comment>> {
        let mut statement = db.prepare(&format!(
            "SELECT id,plan_id,target,label,text,status,message FROM comments
             WHERE {filter} ORDER BY id"
        ))?;
        let rows = statement.query_map(values, |row| {
            Ok(Comment {
                id: row.get::<_, i64>(0)? as u64,
                plan_id: row.get(1)?,
                target: row.get(2)?,
                label: row.get(3)?,
                text: self.content.sql_text(row, 4)?,
                status: row.get(5)?,
                message: row.get::<_, i64>(6)? as u64,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Messages for `role` not yet delivered, oldest first.
    pub fn pending_mail(&self, conversation: &str, role: &str) -> Result<Vec<Mail>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT id,sender,recipient,kind,text,created_ms FROM mail
             WHERE conversation_id=?1 AND recipient=?2 AND delivered_turn IS NULL ORDER BY id",
        )?;
        let rows = statement.query_map(params![conversation, role], |row| {
            Ok(Mail {
                id: row.get::<_, i64>(0)? as u64,
                sender: row.get(1)?,
                recipient: row.get(2)?,
                kind: row.get(3)?,
                text: self.content.sql_text(row, 4)?,
                created_ms: row.get(5)?,
                delivered_turn: None,
                delivered_ms: None,
                state: "queued".into(),
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn pending_mail_conversations(&self) -> Result<Vec<String>> {
        let db = self.db();
        let mut statement = db.prepare("SELECT DISTINCT conversation_id FROM mail WHERE delivered_turn IS NULL AND kind!='copy'")?;
        Ok(statement
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn pending_mail_targets(&self, conversation: &str) -> Result<Vec<MailTarget>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT m.recipient,MAX(m.id),t.id,t.plan_id FROM mail m LEFT JOIN tasks t
             ON t.conversation_id=m.conversation_id AND m.recipient IN ('worker:' || t.id,'verifier:' || t.id,'code-review:' || t.id)
             WHERE m.conversation_id=?1 AND m.delivered_turn IS NULL AND m.kind!='copy'
             GROUP BY m.recipient ORDER BY MIN(m.id)")?;
        Ok(statement
            .query_map([conversation], |row| {
                Ok(MailTarget {
                    role: row.get(0)?,
                    newest: row.get::<_, i64>(1)? as u64,
                    task_id: row.get(2)?,
                    plan_id: row.get(3)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Plans with durable task mail that still needs a successful recipient turn.
    pub fn pending_mail_plans(&self) -> Result<Vec<(String, String)>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT DISTINCT t.conversation_id,t.plan_id FROM tasks t JOIN mail m
             ON m.conversation_id=t.conversation_id
             AND m.recipient IN ('worker:' || t.id, 'verifier:' || t.id, 'code-review:' || t.id)
             WHERE m.delivered_turn IS NULL",
        )?;
        Ok(statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// All mail in the conversation with queue and delivery states, oldest first.
    pub fn mail(&self, conversation: &str) -> Result<Vec<Mail>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT id,sender,recipient,kind,text,created_ms,delivered_turn,delivered_ms FROM mail
             WHERE conversation_id=?1 ORDER BY id",
        )?;
        let rows = statement.query_map([conversation], |row| {
            let delivered_turn: Option<String> = row.get(6)?;
            let state = if delivered_turn.is_some() {
                "delivered"
            } else {
                "queued"
            };
            Ok(Mail {
                id: row.get::<_, i64>(0)? as u64,
                sender: row.get(1)?,
                recipient: row.get(2)?,
                kind: row.get(3)?,
                text: self.content.sql_text(row, 4)?,
                created_ms: row.get(5)?,
                delivered_turn,
                delivered_ms: row.get(7)?,
                state: state.into(),
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// Marks messages delivered by a turn that completed, and announces it.
    pub fn mark_delivered(&self, conversation: &str, ids: &[u64], turn: &str) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut db = self.db();
        let tx = db.transaction()?;
        let now = now_ms();
        for id in ids {
            tx.execute(
                "UPDATE mail SET delivered_turn=?2,delivered_ms=?3 WHERE id=?1",
                params![*id as i64, turn, now],
            )?;
        }
        let recipient: Option<String> = tx
            .query_row("SELECT role FROM turns WHERE id=?1", [turn], |row| {
                row.get(0)
            })
            .optional()?;
        tx.execute(
            "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,(SELECT plan_id FROM turns WHERE id=?2),'mail_delivered',?3,CAST(unixepoch('subsec')*1000 AS INTEGER))",
            params![conversation, turn, serde_json::json!({"ids": ids, "delivered_ms": now, "state": "delivered","recipient":recipient}).to_string()],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn sessions(&self, conversation: &str) -> Result<Vec<SessionInfo>> {
        self.conversation(conversation)?;
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT id, plan_id, role, provider, session_id, status, error, upto_message, started_ms, finished_ms, pid
             FROM turns WHERE conversation_id=?1 ORDER BY started_ms ASC, rowid ASC",
        )?;
        let rows = statement.query_map([conversation], |row| {
            Ok(SessionInfo {
                turn_id: row.get(0)?,
                plan_id: row.get(1)?,
                role: row.get(2)?,
                provider: row.get(3)?,
                session_id: row.get(4)?,
                status: row.get(5)?,
                error: row.get(6)?,
                upto_message: row.get::<_, i64>(7)? as u64,
                started_ms: row.get(8)?,
                finished_ms: row.get(9)?,
                pid: row.get(10)?,
            })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn latest_event_seq(&self, conversation: &str) -> Result<u64> {
        let db = self.db();
        let seq: Option<i64> = db
            .query_row(
                "SELECT seq FROM events WHERE (conversation_id=?1 OR conversation_id IS NULL) ORDER BY seq DESC LIMIT 1",
                [conversation],
                |row| row.get(0),
            )
            .optional()?;
        Ok(seq.unwrap_or(0) as u64)
    }

    pub fn event_bounds(&self, conversation: &str) -> Result<(u64, u64, usize)> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT COALESCE(MIN(seq), 0), COALESCE(MAX(seq), 0), COUNT(*)
             FROM events WHERE (conversation_id=?1 OR conversation_id IS NULL)",
        )?;
        let row = statement.query_row([conversation], |row| {
            Ok((
                row.get::<_, i64>(0)? as u64,
                row.get::<_, i64>(1)? as u64,
                row.get::<_, i64>(2)? as usize,
            ))
        })?;
        Ok(row)
    }

    pub fn history_page(
        &self,
        conversation: &str,
        after: u64,
        limit: usize,
    ) -> Result<HistoryPage> {
        self.conversation(conversation)?;
        let (oldest_seq, latest_seq, total) = self.event_bounds(conversation)?;
        let events = self.events_since_budget(conversation, after, limit, 1024 * 1024)?;
        let next_after = events.last().map(|e| e.seq);
        let has_more = match next_after {
            Some(last_seq) => last_seq < latest_seq,
            None => false,
        };
        let history_unavailable = total > 0
            && (after > latest_seq || (oldest_seq > 1 && after < oldest_seq.saturating_sub(1)));
        Ok(HistoryPage {
            events,
            after,
            next_after,
            has_more,
            total,
            oldest_seq,
            latest_seq,
            history_unavailable,
        })
    }

    /// Reserve the event before publishing an already prepared file. The callback
    /// must only rename the file and swap the snapshot; writes and fsync happen outside this lock.
    pub fn publish_config_with_event(
        &self,
        path: &Path,
        publish: impl FnOnce() -> Result<()>,
    ) -> Result<()> {
        let db = self.db();
        let transaction = db.unchecked_transaction()?;
        let payload = serde_json::json!({"path": path});
        transaction.execute("INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(NULL,'config_updated',?1,CAST(unixepoch('subsec')*1000 AS INTEGER))", [payload.to_string()])?;
        publish()?;
        transaction.commit()?;
        Ok(())
    }

    pub fn append_event(
        &self,
        conversation: &str,
        turn: Option<&str>,
        plan_id: Option<&str>,
        kind: &str,
        payload: &Value,
    ) -> Result<u64> {
        let mut payload = payload.clone();
        if kind == "agent" || kind == "agent_final" {
            for field in ["text", "delta"] {
                if let Some(text) = payload[field].as_str() {
                    let reference = self.content.write(text)?;
                    payload[field] = serde_json::json!({"session_content": reference});
                }
            }
        }
        let db = self.db();
        let resolved_plan: Option<String> = match plan_id {
            Some(pid) => Some(pid.to_string()),
            None => {
                if let Some(tid) = turn {
                    db.query_row("SELECT plan_id FROM turns WHERE id=?1", [tid], |row| {
                        row.get::<_, Option<String>>(0)
                    })
                    .optional()?
                    .flatten()
                } else if let Some(pid) = payload.get("plan").and_then(|v| v.as_str()) {
                    Some(pid.to_string())
                } else if let Some(task_uuid) = payload.get("task").and_then(|v| v.as_str()) {
                    db.query_row(
                        "SELECT plan_id FROM tasks WHERE id=?1",
                        [task_uuid],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .optional()?
                    .flatten()
                } else {
                    payload
                        .get("plan_id")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                }
            }
        };
        db.execute(
            "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,?3,?4,?5,CAST(unixepoch('subsec')*1000 AS INTEGER))",
            params![conversation, turn, resolved_plan, kind, payload.to_string()],
        )?;
        Ok(db.last_insert_rowid() as u64)
    }

    pub fn events_since(&self, conversation: &str, after: u64, limit: usize) -> Result<Vec<Event>> {
        self.events_since_budget(conversation, after, limit, usize::MAX)
    }

    /// The first event is always returned whole, even when it is larger than
    /// the page budget. No later event is discarded from the durable stream.
    pub fn events_since_budget(
        &self,
        conversation: &str,
        after: u64,
        limit: usize,
        budget: usize,
    ) -> Result<Vec<Event>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT seq,turn_id,plan_id,kind,payload,created_ms FROM events
             WHERE (conversation_id=?1 OR conversation_id IS NULL) AND seq>?2 ORDER BY seq LIMIT ?3",
        )?;
        let rows =
            statement.query_map(params![conversation, after as i64, limit as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            })?;
        let mut events = Vec::new();
        let mut bytes = 0usize;
        for row in rows {
            let (seq, turn_id, plan_id, kind, payload, created_ms) = row?;
            let payload = self.hydrate_event_payload(&kind, &payload)?;
            let weight = payload.to_string().len();
            if !events.is_empty() && bytes.saturating_add(weight) > budget {
                break;
            }
            bytes = bytes.saturating_add(weight);
            events.push(Event {
                seq: seq as u64,
                turn_id,
                plan_id,
                kind,
                payload,
                created_ms,
            });
            if bytes >= budget {
                break;
            }
        }
        Ok(events)
    }

    pub fn turn_final_answer(&self, conversation: &str, turn: &str) -> Result<Option<String>> {
        let encoded:Option<String>=self.db().query_row("SELECT payload FROM events WHERE conversation_id=?1 AND turn_id=?2 AND kind='agent_final' ORDER BY seq DESC LIMIT 1",params![conversation,turn],|row|row.get(0)).optional()?;
        encoded
            .map(|encoded| {
                self.hydrate_event_payload("agent_final", &encoded)
                    .map(|payload| payload["text"].as_str().unwrap_or("").to_owned())
            })
            .transpose()
    }

    pub(super) fn hydrate_event_payload(&self, kind: &str, encoded: &str) -> Result<Value> {
        let mut payload: Value = serde_json::from_str(encoded)?;
        for field in ["text", "delta", "report"] {
            if let Some(text) = payload[field].as_str() {
                payload[field] = self.content.decode(text)?.into();
            }
        }
        if kind == "agent" || kind == "agent_final" {
            for field in ["text", "delta"] {
                if let Some(reference) = payload[field].get("session_content") {
                    let reference = serde_json::from_value(reference.clone())?;
                    payload[field] = self.content.read(&reference)?.into();
                }
            }
        }
        Ok(payload)
    }

    /// Reads only one bounded piece of an event's text after checking ownership.
    pub fn session_content(
        &self,
        conversation: &str,
        event: u64,
        field: &str,
        offset: u64,
    ) -> Result<super::content::Chunk> {
        crate::require(
            matches!(field, "text" | "delta"),
            "invalid session content field",
        )?;
        let encoded: String = self.db().query_row(
            "SELECT payload FROM events WHERE conversation_id=?1 AND seq=?2 AND kind IN ('agent','agent_final')",
            params![conversation, event as i64], |row| row.get(0))?;
        let payload: Value = serde_json::from_str(&encoded)?;
        let reference = payload[field]
            .get("session_content")
            .ok_or_else(|| Error::Invalid("historical event has no content reference".into()))?;
        self.content
            .chunk(&serde_json::from_value(reference.clone())?, offset)
    }

    /// Finds one task within a plan by UUID or the orchestrator's local id.
    pub fn task_in_plan_by_id(&self, plan_id: &str, task_id: &str) -> Result<Option<Task>> {
        // Return just one id, rather than decoding every task's reports and
        // specs. Preserve the same first-in-plan resolution as plan().
        let id: Option<String> = self
            .db()
            .query_row(
                "SELECT id FROM tasks WHERE plan_id=?1 AND (id=?2 OR json_extract(spec, '$.id')=?2)
             ORDER BY position LIMIT 1",
                params![plan_id, task_id],
                |row| row.get(0),
            )
            .optional()?;
        id.map(|id| self.task_by_id(&id))
            .transpose()
            .map(Option::flatten)
    }

    /// Finds a task by its globally unique UUID across all plans.
    pub fn task_by_id(&self, task_id: &str) -> Result<Option<Task>> {
        let db = self.db();
        let mut statement = db.prepare(
            "SELECT id,plan_id,spec,status,round,report,verdict,base,result,feedback FROM tasks
             WHERE id=?1 LIMIT 1",
        )?;
        let task = statement
            .query_row([task_id], |row| {
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
                    row.get::<_, Option<String>>(9)?,
                ))
            })
            .optional()?;
        task.map(
            |(id, plan_id, spec, status, round, report, verdict, base, result, feedback)| {
                Ok(Task {
                    input_generation: db
                        .query_row(
                            "SELECT generation FROM task_inputs WHERE task_id=?1",
                            [&id],
                            |r| r.get(0),
                        )
                        .optional()?
                        .unwrap_or(0),
                    id,
                    plan_id,
                    spec: serde_json::from_str(&spec)?,
                    status,
                    round: round as u32,
                    report: report
                        .as_deref()
                        .map(|text| self.content.decode(text))
                        .transpose()?,
                    verdict: verdict
                        .map(|v| {
                            self.content
                                .decode(&v)
                                .and_then(|text| Ok(serde_json::from_str(&text)?))
                        })
                        .transpose()?,
                    base,
                    result,
                    feedback: feedback.map(|v| serde_json::from_str(&v)).transpose()?,
                    freshness_token: None,
                    checks: None,
                })
            },
        )
        .transpose()
    }

    /// Checks if a plan belongs to the conversation.
    pub fn plan_belongs_to_conversation(&self, conversation: &str, plan_id: &str) -> Result<bool> {
        let db = self.db();
        let in_tasks: bool = db
            .query_row(
                "SELECT 1 FROM tasks WHERE plan_id=?1 AND conversation_id=?2 LIMIT 1",
                params![plan_id, conversation],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if in_tasks {
            return Ok(true);
        }
        let in_plans: bool = db
            .query_row(
                "SELECT 1 FROM plans WHERE id=?1 AND conversation_id=?2 LIMIT 1",
                params![plan_id, conversation],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        Ok(in_plans)
    }

    /// Finds the conversation_id for a plan.
    pub fn plan_conversation(&self, plan_id: &str) -> Result<String> {
        let db = self.db();
        let conv: Option<String> = db
            .query_row(
                "SELECT conversation_id FROM tasks WHERE plan_id=?1 LIMIT 1",
                [plan_id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(c) = conv {
            return Ok(c);
        }
        let conv2: Option<String> = db
            .query_row(
                "SELECT conversation_id FROM plans WHERE id=?1 LIMIT 1",
                [plan_id],
                |row| row.get(0),
            )
            .optional()?;
        conv2.ok_or_else(|| Error::Invalid(format!("no plan {plan_id} found")))
    }

    /// Checks if a task belongs to a plan (by UUID or spec id).
    pub fn task_in_plan(&self, plan_id: &str, task_id: &str) -> Result<bool> {
        let db = self.db();
        let found: bool = db
            .query_row(
                "SELECT 1 FROM tasks WHERE plan_id=?1 AND (id=?2 OR json_extract(spec, '$.id')=?2) LIMIT 1",
                params![plan_id, task_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        Ok(found)
    }

    /// All comments in the conversation, optionally filtered by status.
    pub fn comments_all(&self, conversation: &str, status: Option<&str>) -> Result<Vec<Comment>> {
        if let Some(st) = status {
            self.comment_rows(
                &self.db(),
                "conversation_id=?1 AND status=?2",
                params![conversation, st],
            )
        } else {
            self.comment_rows(&self.db(), "conversation_id=?1", params![conversation])
        }
    }

    /// Comments for a specific plan in the conversation, optionally filtered by status.
    pub fn comments_by_plan(
        &self,
        conversation: &str,
        plan_id: &str,
        status: Option<&str>,
    ) -> Result<Vec<Comment>> {
        if let Some(st) = status {
            self.comment_rows(
                &self.db(),
                "conversation_id=?1 AND plan_id=?2 AND status=?3",
                params![conversation, plan_id, st],
            )
        } else {
            self.comment_rows(
                &self.db(),
                "conversation_id=?1 AND plan_id=?2",
                params![conversation, plan_id],
            )
        }
    }

    /// Paginated list of all stored plans for a conversation.
    pub fn plans_page(&self, conversation: &str, offset: usize, limit: usize) -> Result<PlansPage> {
        self.conversation(conversation)?;
        let (total, plan_ids, active_plan_id) = {
            let db = self.db();
            let total: i64 = db.query_row(
                "SELECT COUNT(*) FROM plans WHERE conversation_id=?1",
                [conversation],
                |row| row.get(0),
            )?;
            let total = total as usize;
            let mut stmt = db.prepare(
                "SELECT id FROM plans WHERE conversation_id=?1 ORDER BY created_ms DESC, rowid DESC LIMIT ?2 OFFSET ?3",
            )?;
            let plan_ids = stmt
                .query_map(params![conversation, limit as i64, offset as i64], |row| {
                    row.get(0)
                })?
                .collect::<std::result::Result<Vec<String>, _>>()?;

            let active_plan_id: Option<String> = db
                .query_row(
                    "SELECT plan_id FROM tasks WHERE conversation_id=?1 ORDER BY created_ms DESC, rowid DESC LIMIT 1",
                    [conversation],
                    |row| row.get(0),
                )
                .optional()?;
            (total, plan_ids, active_plan_id)
        };

        let mut plans = Vec::with_capacity(plan_ids.len());
        for pid in plan_ids {
            plans.push(self.plan_history(conversation, &pid)?);
        }

        let has_more = offset + plans.len() < total;
        Ok(PlansPage {
            plans,
            offset,
            limit,
            total,
            has_more,
            active_plan_id,
        })
    }

    /// Detailed history of one plan: its tasks, agents, turns, reports and verdicts.
    pub fn plan_history(&self, conversation: &str, plan_id: &str) -> Result<PlanHistory> {
        self.conversation(conversation)?;
        let tasks = self.plan(plan_id)?;
        crate::require(
            !tasks.is_empty(),
            &format!("no plan {plan_id} in conversation {conversation}"),
        )?;

        let db = self.db();
        let created_ms: i64 = db.query_row(
            "SELECT COALESCE(
                (SELECT created_ms FROM plans WHERE id=?1 AND conversation_id=?2),
                (SELECT MIN(created_ms) FROM tasks WHERE plan_id=?1 AND conversation_id=?2),
                0
            )",
            params![plan_id, conversation],
            |row| row.get(0),
        )?;

        let active_plan_id: Option<String> = db
            .query_row(
                "SELECT plan_id FROM tasks WHERE conversation_id=?1 ORDER BY created_ms DESC, rowid DESC LIMIT 1",
                [conversation],
                |row| row.get(0),
            )
            .optional()?;
        let is_active = active_plan_id.as_deref() == Some(plan_id);

        let status = if tasks.iter().all(|t| t.status == "approved") {
            "approved".to_string()
        } else if tasks
            .iter()
            .any(|t| t.status == "working" || t.status == "verifying" || t.status == "queued")
        {
            "active".to_string()
        } else if tasks.iter().all(|t| t.status == "proposed") {
            "proposed".to_string()
        } else if tasks.iter().any(|t| t.status == "failed") {
            "failed".to_string()
        } else if tasks.iter().any(|t| t.status == "cancelled") {
            "cancelled".to_string()
        } else if tasks.iter().any(|t| t.status == "needs_user") {
            "needs_user".to_string()
        } else if tasks.iter().any(|t| t.status == "revised") {
            "revised".to_string()
        } else {
            "done".to_string()
        };

        let task_ids: std::collections::HashSet<String> =
            tasks.iter().map(|t| t.id.clone()).collect();
        let mut turn_stmt = db.prepare(
            "SELECT id, plan_id, role, provider, session_id, status, error, upto_message, started_ms, finished_ms, pid
             FROM turns WHERE conversation_id=?1 ORDER BY started_ms ASC, rowid ASC",
        )?;
        let all_turns = turn_stmt
            .query_map([conversation], |row| {
                Ok(SessionInfo {
                    turn_id: row.get(0)?,
                    plan_id: row.get(1)?,
                    role: row.get(2)?,
                    provider: row.get(3)?,
                    session_id: row.get(4)?,
                    status: row.get(5)?,
                    error: row.get(6)?,
                    upto_message: row.get::<_, i64>(7)? as u64,
                    started_ms: row.get(8)?,
                    finished_ms: row.get(9)?,
                    pid: row.get(10)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let turns: Vec<SessionInfo> = all_turns
            .into_iter()
            .filter(|turn| {
                if turn.plan_id.as_deref() == Some(plan_id) {
                    return true;
                }
                if let Some(tid) = super::task_role(&turn.role).map(|(id, _)| id)
                    && task_ids.contains(tid)
                {
                    return true;
                }
                false
            })
            .collect();

        let mut agents_map: std::collections::BTreeMap<String, PlanAgentInfo> =
            std::collections::BTreeMap::new();
        for turn in &turns {
            let (task_id, task_label) =
                if let Some(tid) = super::task_role(&turn.role).map(|(id, _)| id) {
                    let label = tasks
                        .iter()
                        .find(|t| t.id == tid)
                        .and_then(|t| t.spec["id"].as_str())
                        .map(|s| s.to_string());
                    (Some(tid.to_string()), label)
                } else {
                    (None, None)
                };
            let entry = agents_map
                .entry(turn.role.clone())
                .or_insert_with(|| PlanAgentInfo {
                    role: turn.role.clone(),
                    provider: turn.provider.clone(),
                    model: None,
                    task_id,
                    task_label,
                    turns_count: 0,
                    status: turn.status.clone(),
                });
            entry.turns_count += 1;
            entry.status = turn.status.clone();
            entry.provider = turn.provider.clone();
        }
        let agents: Vec<PlanAgentInfo> = agents_map.into_values().collect();

        let mut reports = Vec::new();
        let mut verdicts = Vec::new();
        for task in &tasks {
            let label = task.spec["id"].as_str().unwrap_or(&task.id).to_string();
            if let Some(report) = &task.report {
                reports.push(TaskReportInfo {
                    task_id: task.id.clone(),
                    task_label: label.clone(),
                    round: task.round,
                    report: report.clone(),
                });
            }
            if let Some(verdict) = &task.verdict {
                verdicts.push(TaskVerdictInfo {
                    task_id: task.id.clone(),
                    task_label: label,
                    round: task.round,
                    verdict: verdict.clone(),
                });
            }
        }

        let run_id: Option<String> = db
            .query_row(
                "SELECT run_id FROM plan_runs WHERE plan_id=?1",
                [plan_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(PlanHistory {
            ledger_mode: if run_id.is_some() { "ledger" } else { "legacy" }.into(),
            run_id,
            id: plan_id.to_string(),
            conversation_id: conversation.to_string(),
            created_ms,
            status,
            is_active,
            tasks,
            agents,
            turns,
            reports,
            verdicts,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deferred_plan_comment_claims_expire_after_user_input_or_plan_publication() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("core.db")).unwrap();
        let conversation = store.create_conversation("/project").unwrap();
        let id = &conversation.id;
        let user = store
            .add_message(id, "user", "revise only the first comment")
            .unwrap();
        assert_eq!(store.pending_plan_comment_ids(id).unwrap(), None);
        store
            .append_event(
                id,
                None,
                None,
                "plan_comments_addressed",
                &serde_json::json!({"ids":[1],"upto_message":user.seq}),
            )
            .unwrap();
        store
            .add_message(id, "options-planner", "regenerate the plan")
            .unwrap();
        assert_eq!(store.pending_plan_comment_ids(id).unwrap(), Some(vec![1]));
        let next = store.add_message(id, "user", "new scope").unwrap();
        assert_eq!(store.pending_plan_comment_ids(id).unwrap(), None);
        store
            .append_event(
                id,
                None,
                None,
                "plan_comments_addressed",
                &serde_json::json!({"ids":[],"upto_message":next.seq}),
            )
            .unwrap();
        assert_eq!(store.pending_plan_comment_ids(id).unwrap(), Some(vec![]));
        store
            .append_event(id, None, None, "plan_proposed", &serde_json::json!({}))
            .unwrap();
        assert_eq!(store.pending_plan_comment_ids(id).unwrap(), None);
    }

    #[test]
    fn schema_five_events_migrate_without_losing_history_or_plan_scope() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.db");
        let store = Store::open(&path).unwrap();
        let conversation = store.create_conversation("/project").unwrap();
        let other = store.create_conversation("/project").unwrap();
        store
            .append_event(
                &conversation.id,
                None,
                Some("p1"),
                "notice",
                &serde_json::json!({"text": "preserved"}),
            )
            .unwrap();
        let before = store.events_since(&conversation.id, 0, 100).unwrap();
        drop(store);
        let db = Connection::open(&path).unwrap();
        db.execute_batch("BEGIN;
            CREATE TABLE events_v5(seq INTEGER PRIMARY KEY AUTOINCREMENT,
                conversation_id TEXT NOT NULL REFERENCES conversations(id),
                turn_id TEXT, plan_id TEXT, kind TEXT NOT NULL, payload TEXT NOT NULL,
                created_ms INTEGER NOT NULL DEFAULT 0);
            INSERT INTO events_v5 SELECT seq,conversation_id,turn_id,plan_id,kind,payload,created_ms FROM events;
            DROP TABLE events;
            ALTER TABLE events_v5 RENAME TO events;
            PRAGMA user_version=5;
            COMMIT;").unwrap();
        drop(db);
        let store = Store::open(&path).unwrap();
        let after = store.events_since(&conversation.id, 0, 100).unwrap();
        assert_eq!(
            serde_json::to_value(&before).unwrap(),
            serde_json::to_value(&after).unwrap()
        );
        assert!(store.events_since(&other.id, 0, 100).unwrap().is_empty());
        store
            .publish_config_with_event(Path::new("config.toml"), || Ok(()))
            .unwrap();
        let own = store.events_since(&conversation.id, 0, 100).unwrap();
        let shared = store.events_since(&other.id, 0, 100).unwrap();
        assert_eq!(shared.len(), 1);
        assert_eq!(shared[0].kind, "config_updated");
        assert_eq!(own.last().unwrap().seq, shared[0].seq);
        assert!(shared[0].seq > before.last().unwrap().seq);
        let db = Connection::open(&path).unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
                .unwrap(),
            SCHEMA
        );
        assert_eq!(db.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name IN ('events_by_plan','events_by_conversation')", [], |row| row.get::<_, u32>(0)).unwrap(), 2);
    }

    #[test]
    fn approved_outputs_follow_completion_order_and_stay_with_their_project() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("core.db")).unwrap();
        let conversation = store.create_conversation("/project").unwrap();
        let specs = [
            serde_json::json!({"id": "t1"}),
            serde_json::json!({"id": "t2"}),
        ];
        let mut tasks = store.add_plan(&conversation.id, &specs).unwrap();
        for index in [1, 0] {
            let task = &mut tasks[index];
            task.status = "approved".into();
            task.base = Some(format!("base-{index}"));
            task.result = Some(format!("result-{index}"));
            store.update_task(task).unwrap();
            store
                .append_event(
                    &conversation.id,
                    None,
                    None,
                    "task",
                    &serde_json::json!({"task": task.id, "status": "approved"}),
                )
                .unwrap();
        }
        let other = store.create_conversation("/other-project").unwrap();
        let mut task = store.add_plan(&other.id, &specs[..1]).unwrap().remove(0);
        task.status = "approved".into();
        task.base = Some("other-base".into());
        task.result = Some("other-result".into());
        store.update_task(&task).unwrap();
        store
            .append_event(
                &other.id,
                None,
                None,
                "task",
                &serde_json::json!({"task": task.id, "status": "approved"}),
            )
            .unwrap();
        assert_eq!(
            store.approved_writer_results("/project").unwrap(),
            [
                ("base-0".into(), "result-0".into()),
                ("base-1".into(), "result-1".into()),
            ]
        );
        // An old approval cannot qualify a task that now needs a decision.
        tasks[0].status = "needs_user".into();
        store.update_task(&tasks[0]).unwrap();
        assert_eq!(
            store.approved_writer_results("/project").unwrap(),
            [("base-1".into(), "result-1".into()),]
        );
    }

    #[test]
    fn failed_event_migration_rolls_back_the_schema_and_releases_the_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.db");
        drop(Store::open(&path).unwrap());
        let db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA user_version=5; CREATE VIEW event_view AS SELECT * FROM events;")
            .unwrap();
        assert!(Store::open(&path).is_err());
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |row| row.get::<_, u32>(0))
                .unwrap(),
            5
        );
        let tables: i64 = db
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='events_v6'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0);
        db.execute_batch("DROP VIEW event_view;").unwrap();
        drop(db);
        assert!(Store::open(&path).is_ok());
    }

    #[test]
    fn a_newer_schema_is_refused_not_rewritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.db");
        drop(Store::open(&path).unwrap());
        let connection = Connection::open(&path).unwrap();
        let version: u32 = connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA);
        connection
            .pragma_update(None, "user_version", SCHEMA + 1)
            .unwrap();
        drop(connection);
        let error = Store::open(&path).err().unwrap();
        assert!(error.to_string().contains("newer antislop"), "{error}");
    }

    #[test]
    fn a_schema_1_state_gains_event_times() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.db");
        let old = Connection::open(&path).unwrap();
        old.execute_batch(
            "CREATE TABLE conversations(id TEXT PRIMARY KEY, project TEXT NOT NULL, created_ms INTEGER NOT NULL);
             CREATE TABLE events(seq INTEGER PRIMARY KEY AUTOINCREMENT,
               conversation_id TEXT NOT NULL REFERENCES conversations(id),
               turn_id TEXT, kind TEXT NOT NULL, payload TEXT NOT NULL);
             INSERT INTO conversations VALUES('c', '/p', 0);
             INSERT INTO events(conversation_id,turn_id,kind,payload) VALUES('c',NULL,'old','{}');
             PRAGMA user_version=1;",
        )
        .unwrap();
        drop(old);
        let store = Store::open(&path).unwrap();
        store
            .append_event("c", None, None, "new", &serde_json::json!({}))
            .unwrap();
        let events = store.events_since("c", 0, 10).unwrap();
        assert_eq!(events[0].created_ms, 0, "unknown, not invented");
        assert!(events[1].created_ms > 1_700_000_000_000);
    }

    #[test]
    fn checks_keep_their_evidence_and_never_go_back_to_checking() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("core.db")).unwrap();
        assert!(
            store
                .set_check("u1", 1, "R1", "checking", None, "")
                .unwrap()
        );
        assert!(
            !store
                .set_check("u1", 1, "R1", "checking", None, "")
                .unwrap()
        );
        assert!(
            store
                .set_check("u1", 1, "R1", "settled", Some("ok"), "a.rs:3")
                .unwrap()
        );
        assert!(
            !store
                .set_check("u1", 1, "R1", "checking", None, "")
                .unwrap()
        );
        // The verdict repeats the check without a note: the evidence stays.
        store
            .set_check("u1", 1, "R1", "settled", Some("ok"), "")
            .unwrap();
        let rows = store.checks("u1", 1).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["note"], "a.rs:3");
        assert_eq!(rows[0]["status"], "ok");
        assert!(store.checks("u1", 2).unwrap().is_empty());
    }

    #[test]
    fn card_feedback_round_trips_and_rows_without_it_stay_readable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.db");
        // A database from before tasks carried card feedback.
        let old = Connection::open(&path).unwrap();
        old.execute_batch(
            "CREATE TABLE conversations(id TEXT PRIMARY KEY, project TEXT NOT NULL, created_ms INTEGER NOT NULL);
             CREATE TABLE tasks(id TEXT PRIMARY KEY,
               conversation_id TEXT NOT NULL REFERENCES conversations(id),
               plan_id TEXT NOT NULL, position INTEGER NOT NULL, spec TEXT NOT NULL,
               status TEXT NOT NULL, round INTEGER NOT NULL DEFAULT 0,
               report TEXT, verdict TEXT, base TEXT, result TEXT,
               created_ms INTEGER NOT NULL);
             INSERT INTO conversations VALUES('c', '/p', 0);
             INSERT INTO tasks(id,conversation_id,plan_id,position,spec,status,created_ms)
               VALUES('u1','c','p1',0,'{\"id\":\"t1\"}','queued',0);
             PRAGMA user_version=4;",
        )
        .unwrap();
        drop(old);
        let store = Store::open(&path).unwrap();
        let before = store.plan("p1").unwrap().remove(0);
        assert_eq!(before.status, "queued");
        assert_eq!(before.feedback, None, "an old row shows no card result");
        assert_eq!(store.task_by_id("u1").unwrap().unwrap().feedback, None);

        // The new column round-trips through both read paths.
        let feedback = serde_json::json!({"round": 2, "integration": {"outcome": "applied"}});
        store
            .update_task(&Task {
                feedback: Some(feedback.clone()),
                ..before.clone()
            })
            .unwrap();
        assert_eq!(
            store.plan("p1").unwrap()[0].feedback.as_ref(),
            Some(&feedback)
        );
        assert_eq!(
            store.task_by_id("u1").unwrap().unwrap().feedback.as_ref(),
            Some(&feedback)
        );
        // Clearing a retry's card really clears the stored row.
        store
            .update_task(&Task {
                feedback: None,
                ..before.clone()
            })
            .unwrap();
        assert_eq!(store.plan("p1").unwrap()[0].feedback, None);
        // The task row serializes without the key when there is no card data.
        assert!(
            serde_json::to_value(&before)
                .unwrap()
                .get("feedback")
                .is_none()
        );
    }

    #[test]
    fn rows_and_events_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("core.db")).unwrap();
        let conversation = store.create_conversation("/p").unwrap();
        let first = store
            .add_message(&conversation.id, "user", "hello")
            .unwrap();
        let turn = store
            .start_turn(&conversation.id, "orchestrator", "codex", first.seq, None)
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
        assert_eq!(events[0].payload["text"], "hello");
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
            .start_turn(&conversation.id, "orchestrator", "agy", 0, None)
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

    #[test]
    fn workspace_trust_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("core.db")).unwrap();
        assert!(!store.is_trusted("/workspace/proj1").unwrap());
        store.trust_project("/workspace/proj1").unwrap();
        assert!(store.is_trusted("/workspace/proj1").unwrap());
        assert_eq!(
            store.trusted_projects().unwrap(),
            vec!["/workspace/proj1".to_string()]
        );
        store.untrust_project("/workspace/proj1").unwrap();
        assert!(!store.is_trusted("/workspace/proj1").unwrap());
    }
}
