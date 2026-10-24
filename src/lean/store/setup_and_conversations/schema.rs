use super::*;

const BASE_TABLE_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS conversations(
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
               conversation_id TEXT REFERENCES conversations(id),
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
               trusted_ms INTEGER NOT NULL);";

const PLAN_TABLE_SCHEMA: &str = "CREATE TABLE IF NOT EXISTS plans(
               id TEXT PRIMARY KEY,
               conversation_id TEXT NOT NULL REFERENCES conversations(id),
               created_ms INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS plans_by_conversation ON plans(conversation_id, created_ms);
             CREATE INDEX IF NOT EXISTS events_by_plan ON events(conversation_id, plan_id, seq);
             CREATE INDEX IF NOT EXISTS turns_by_plan ON turns(conversation_id, plan_id, started_ms);
             ";

pub(super) fn initialize_base_tables(transaction: &rusqlite::Transaction<'_>) -> Result<()> {
    transaction.execute_batch(BASE_TABLE_SCHEMA)?;
    Ok(())
}

pub(super) fn initialize_plan_tables(transaction: &rusqlite::Transaction<'_>) -> Result<()> {
    transaction.execute_batch(PLAN_TABLE_SCHEMA)?;
    Ok(())
}
