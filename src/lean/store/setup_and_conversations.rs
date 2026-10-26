use super::*;

#[path = "setup_and_conversations/schema.rs"]
mod schema;
use schema::{initialize_base_tables, initialize_plan_tables};

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let found: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
        let nonempty: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*')",
            [],
            |row| row.get(0),
        )?;
        crate::require(
            found == SCHEMA || (found == 0 && !nonempty),
            &format!(
                "{} has incompatible schema {found}; this build requires {SCHEMA}. Stop the daemon, manually archive the project's .antislop/ directory, then restart to create fresh state. No data was migrated or deleted; client and daemon must be restarted together.",
                path.display()
            ),
        )?;
        if found == 0 {
            Self::initialize(&connection)?;
        }
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
            content: crate::lean::content::Content::new(path),
        })
    }

    pub(super) fn initialize(connection: &Connection) -> Result<()> {
        let transaction = connection.unchecked_transaction()?;
        initialize_base_tables(&transaction)?;
        initialize_plan_tables(&transaction)?;
        transaction.execute_batch(include_str!("../ledger/schema.sql"))?;
        Self::initialize_history(&transaction)?;
        Self::initialize_run_context(&transaction)?;
        transaction.execute_batch(include_str!("agent_runtime.sql"))?;
        transaction.pragma_update(None, "user_version", SCHEMA)?;
        transaction.commit()?;
        Ok(())
    }

    pub(in crate::lean) fn decode_content(&self, text: &str) -> Result<String> {
        self.content.decode(text)
    }

    pub(in crate::lean) fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
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
        let seq = insert_message(&tx, conversation, author, &stored_text)?;
        maybe_start_requirements_checkpoint(&tx, conversation, author, text, seq)?;
        tx.commit()?;
        Ok(Message {
            seq,
            author: author.into(),
            text: text.into(),
        })
    }
}

fn insert_message(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    author: &str,
    stored_text: &str,
) -> Result<u64> {
    tx.execute(
        "INSERT INTO messages(conversation_id,author,text,created_ms) VALUES(?1,?2,?3,?4)",
        params![conversation, author, stored_text, now_ms()],
    )?;
    let sequence = tx.last_insert_rowid() as u64;
    insert_message_event(tx, conversation, author, stored_text, sequence)?;
    Ok(sequence)
}

fn insert_message_event(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    author: &str,
    text: &str,
    sequence: u64,
) -> Result<()> {
    tx.execute(
        "INSERT INTO events(conversation_id,turn_id,plan_id,kind,payload,created_ms) VALUES(?1,NULL,NULL,'message',?2,CAST(unixepoch('subsec')*1000 AS INTEGER))",
        params![conversation, serde_json::json!({"seq": sequence, "author": author, "text": text}).to_string()],
    )?;
    Ok(())
}

fn maybe_start_requirements_checkpoint(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    author: &str,
    text: &str,
    sequence: u64,
) -> Result<()> {
    if author != "user" || text.starts_with(crate::lean::COMMENT_PREFIX) {
        return Ok(());
    }
    if restart_completed_workflow(tx, conversation)? > 0 {
        create_requirements_run(tx, conversation, text, sequence)?;
    }
    Ok(())
}

fn restart_completed_workflow(tx: &rusqlite::Transaction<'_>, conversation: &str) -> Result<usize> {
    Ok(tx.execute(
        "UPDATE session_workflow SET phase='interview',summary='',upto_message=0
         WHERE conversation_id=?1 AND phase='implementation'
         AND EXISTS(SELECT 1 FROM tasks WHERE plan_id=(SELECT id FROM plans WHERE conversation_id=?1 ORDER BY created_ms DESC,rowid DESC LIMIT 1))
         AND NOT EXISTS(SELECT 1 FROM tasks WHERE plan_id=(SELECT id FROM plans WHERE conversation_id=?1 ORDER BY created_ms DESC,rowid DESC LIMIT 1) AND status<>'approved')
         AND NOT EXISTS(SELECT 1 FROM turns WHERE conversation_id=?1 AND status='running')",
        [conversation],
    )?)
}

fn create_requirements_run(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    text: &str,
    sequence: u64,
) -> Result<()> {
    let run_id = crate::id();
    insert_requirements_run(tx, conversation, text, sequence, &run_id)?;
    link_planning_run(tx, conversation, &run_id)?;
    append_requirements_events(tx, conversation, &run_id)
}

fn insert_requirements_run(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    text: &str,
    sequence: u64,
    run_id: &str,
) -> Result<()> {
    tx.execute(
        "INSERT INTO runs(id,conversation_id,goal,created_ms,start_message) VALUES(?1,?2,?3,?4,?5)",
        params![
            run_id,
            conversation,
            crate::lean::tasks::bound(text, 1000),
            now_ms(),
            sequence as i64
        ],
    )?;
    Ok(())
}

fn link_planning_run(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    run_id: &str,
) -> Result<()> {
    tx.execute(
        "INSERT INTO planning_runs VALUES(?1,?2) ON CONFLICT(conversation_id) DO UPDATE SET run_id=excluded.run_id",
        params![conversation, run_id],
    )?;
    Ok(())
}

fn append_requirements_events(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    run_id: &str,
) -> Result<()> {
    append_run_event(
        tx,
        conversation,
        "run_created",
        serde_json::json!({"run": run_id}),
    )?;
    append_run_event(
        tx,
        conversation,
        "workflow",
        serde_json::json!({"phase": "interview", "summary": ""}),
    )
}

fn append_run_event(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    kind: &str,
    payload: serde_json::Value,
) -> Result<()> {
    tx.execute(
        "INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,?2,?3,?4)",
        params![conversation, kind, payload.to_string(), now_ms()],
    )?;
    Ok(())
}
