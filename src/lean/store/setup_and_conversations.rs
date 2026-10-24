use super::*;

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
        transaction.execute_batch(
            "CREATE TABLE IF NOT EXISTS conversations(
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
               trusted_ms INTEGER NOT NULL);",
        )?;
        transaction.execute_batch(
            "CREATE TABLE IF NOT EXISTS plans(
               id TEXT PRIMARY KEY,
               conversation_id TEXT NOT NULL REFERENCES conversations(id),
               created_ms INTEGER NOT NULL);
             CREATE INDEX IF NOT EXISTS plans_by_conversation ON plans(conversation_id, created_ms);
             CREATE INDEX IF NOT EXISTS events_by_plan ON events(conversation_id, plan_id, seq);
             CREATE INDEX IF NOT EXISTS turns_by_plan ON turns(conversation_id, plan_id, started_ms);
             ",
        )?;
        transaction.execute_batch(include_str!("../ledger/schema.sql"))?;
        Self::initialize_history(&transaction)?;
        Self::initialize_run_context(&transaction)?;
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
        // A follow-up after completed implementation starts a new requirements
        // checkpoint. Keep all plans, turns and ledger evidence as history.
        if author == "user" && !text.starts_with(crate::lean::COMMENT_PREFIX) {
            let restarted = tx.execute(
                "UPDATE session_workflow SET phase='interview',summary='',upto_message=0
                 WHERE conversation_id=?1 AND phase='implementation'
                 AND EXISTS(SELECT 1 FROM tasks WHERE plan_id=(SELECT id FROM plans WHERE conversation_id=?1 ORDER BY created_ms DESC,rowid DESC LIMIT 1))
                 AND NOT EXISTS(SELECT 1 FROM tasks WHERE plan_id=(SELECT id FROM plans WHERE conversation_id=?1 ORDER BY created_ms DESC,rowid DESC LIMIT 1) AND status<>'approved')
                 AND NOT EXISTS(SELECT 1 FROM turns WHERE conversation_id=?1 AND status='running')",
                [conversation],
            )?;
            if restarted > 0 {
                let run_id = crate::id();
                tx.execute(
                    "INSERT INTO runs(id,conversation_id,goal,created_ms,start_message) VALUES(?1,?2,?3,?4,?5)",
                    params![run_id, conversation, crate::lean::tasks::bound(text, 1000), now_ms(), seq as i64],
                )?;
                tx.execute(
                    "INSERT INTO planning_runs VALUES(?1,?2) ON CONFLICT(conversation_id) DO UPDATE SET run_id=excluded.run_id",
                    params![conversation, run_id],
                )?;
                tx.execute(
                    "INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,'run_created',?2,?3)",
                    params![conversation, serde_json::json!({"run":run_id}).to_string(), now_ms()],
                )?;
                tx.execute(
                    "INSERT INTO events(conversation_id,kind,payload,created_ms) VALUES(?1,'workflow',?2,?3)",
                    params![conversation, serde_json::json!({"phase":"interview","summary":""}).to_string(), now_ms()],
                )?;
            }
        }
        tx.commit()?;
        Ok(Message {
            seq,
            author: author.into(),
            text: text.into(),
        })
    }
}
