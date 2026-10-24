//! Durable round attribution and bounded history reads. Legacy turns remain
//! unassigned unless the runtime recorded their scope explicitly.
use super::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMessage {
    pub conversation: String,
    pub plan: Option<String>,
    pub recipient: String,
    pub operation_id: String,
    pub expected_round: Option<u32>,
    pub expected_run: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ContentOwner {
    Round {
        task: String,
        round: u32,
        field: String,
    },
    Mail {
        id: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum HistoryScope {
    Round { task: String, round: u32 },
    Session { turn: String },
}

impl Store {
    pub(super) fn initialize_history(connection: &Connection) -> Result<()> {
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS turn_scopes(
                turn_id TEXT PRIMARY KEY REFERENCES turns(id),
                task_id TEXT REFERENCES tasks(id), round INTEGER);
             CREATE INDEX IF NOT EXISTS events_by_session ON events(conversation_id,turn_id,seq);
             CREATE INDEX IF NOT EXISTS events_by_mail_id
                ON events(conversation_id,json_extract(payload,'$.id'))
                WHERE kind='mail' AND turn_id IS NULL;
             CREATE INDEX IF NOT EXISTS turns_by_round ON turn_scopes(task_id,round,turn_id);
             CREATE TABLE IF NOT EXISTS mail_sources(mail_id INTEGER PRIMARY KEY REFERENCES mail(id),turn_id TEXT NOT NULL REFERENCES turns(id));
             CREATE TABLE IF NOT EXISTS mail_scopes(
                mail_id INTEGER PRIMARY KEY REFERENCES mail(id),task_id TEXT,round INTEGER,run TEXT);
             CREATE INDEX IF NOT EXISTS mail_by_round ON mail_scopes(task_id,round,mail_id);
             CREATE INDEX IF NOT EXISTS mail_by_source ON mail_sources(turn_id,mail_id);
             CREATE TABLE IF NOT EXISTS comment_scopes(
                comment_id INTEGER PRIMARY KEY REFERENCES comments(id), round INTEGER, run TEXT);
             CREATE TABLE IF NOT EXISTS agent_operations(
                conversation_id TEXT NOT NULL REFERENCES conversations(id), operation_id TEXT NOT NULL,
                signature TEXT NOT NULL, mail_id INTEGER NOT NULL REFERENCES mail(id),
                PRIMARY KEY(conversation_id,operation_id));
             CREATE TABLE IF NOT EXISTS task_rounds(
                task_id TEXT NOT NULL REFERENCES tasks(id), round INTEGER NOT NULL,
                status TEXT NOT NULL, content TEXT NOT NULL,
                PRIMARY KEY(task_id,round));",
        )?;
        Ok(())
    }

    pub(crate) fn agent_mail_user_message(
        &self,
        conversation: &str,
        mail: u64,
    ) -> Result<Option<u64>> {
        let mail = i64::try_from(mail)
            .map_err(|_| Error::Invalid("mail identifier exceeds SQLite range".into()))?;
        let message: Option<i64> = self
            .db()
            .query_row(
                "SELECT COALESCE(json_extract(e.payload,'$.user_message'),
                (SELECT json_extract(n.payload,'$.seq') FROM events n
                 WHERE n.seq=e.seq+1 AND n.conversation_id=e.conversation_id
                 AND n.kind='message' AND json_extract(n.payload,'$.author')='user'))
             FROM events e WHERE e.conversation_id=?1 AND e.turn_id IS NULL
             AND e.kind='mail' AND json_extract(e.payload,'$.sender')='user'
             AND json_extract(e.payload,'$.recipient')='orchestrator'
             AND json_extract(e.payload,'$.id')=?2 ORDER BY e.seq DESC LIMIT 1",
                params![conversation, mail],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        Ok(message.map(|seq| seq as u64))
    }
}

#[path = "history/content.rs"]
mod content;
#[path = "history/mail.rs"]
mod mail;
#[path = "history/messages.rs"]
mod messages;
#[path = "history/rounds.rs"]
mod rounds;

#[cfg(test)]
#[path = "history/tests.rs"]
mod tests;
