use super::*;

#[path = "setup_and_conversations/schema.rs"]
mod schema;
use schema::{initialize_base_tables, initialize_plan_tables};
use std::path::PathBuf;

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let mut connection = Connection::open(path)?;
        connection.busy_timeout(std::time::Duration::from_secs(5))?;
        let found = validate_database(&connection, path)?;
        if found == 0 {
            Self::initialize(&connection)?;
        }
        // Additive runtime tables; no existing session is repaired or resumed.
        connection.execute_batch(include_str!("runtime_policy.sql"))?;
        // Keep schema migration and provenance initialization in one SQLite
        // transaction. Legacy reason text and user_selected bits cannot prove
        // who chose a team, so origin-less rows remain unknown.
        connection.execute_batch("PRAGMA foreign_keys=ON;")?;
        let migration =
            connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let has_user_selected: bool = migration.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('teams') WHERE name='user_selected')",
            [],
            |row| row.get(0),
        )?;
        if !has_user_selected {
            migration.execute_batch(
                "ALTER TABLE teams ADD COLUMN user_selected INTEGER NOT NULL DEFAULT 0 CHECK(user_selected IN (0,1));",
            )?;
        }
        let has_origin: bool = migration.query_row(
            "SELECT EXISTS(SELECT 1 FROM pragma_table_info('teams') WHERE name='origin')",
            [],
            |row| row.get(0),
        )?;
        if !has_origin {
            migration.execute_batch(
                "ALTER TABLE teams ADD COLUMN origin TEXT NOT NULL DEFAULT 'unknown' CHECK(origin IN ('unknown','user','planner'));
                 UPDATE teams SET origin='unknown',user_selected=0;",
            )?;
        } else if !has_user_selected {
            // An earlier schema already had structured origin; preserve its
            // confirmed user selections when adding the write-serialization flag.
            migration.execute_batch(
                "UPDATE teams SET user_selected=CASE WHEN origin='user' THEN 1 ELSE 0 END;",
            )?;
        }
        migration.commit()?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; PRAGMA foreign_keys=ON;",
        )?;
        let content = crate::lean::content::Content::new(path);
        content.cleanup_staging()?;
        Ok(Self {
            connection: Mutex::new(connection),
            content,
            content_write: Mutex::new(()),
            operations: Mutex::new(()),
        })
    }

    pub(in crate::lean) fn check_runtime_repository(path: &Path, identity: &str) -> Result<()> {
        if !path.try_exists()? {
            return Ok(());
        }
        let connection =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        if validate_database(&connection, path)? == 0 {
            return Ok(());
        }
        let foreign: bool = connection.query_row(
            "SELECT EXISTS(SELECT 1 FROM project_identities WHERE identity<>?1)",
            [identity],
            |row| row.get(0),
        )?;
        crate::require(
            !foreign,
            "runtime state belongs to another repository; use that project's own state directory",
        )
    }

    pub(super) fn initialize(connection: &Connection) -> Result<()> {
        let transaction = connection.unchecked_transaction()?;
        initialize_base_tables(&transaction)?;
        initialize_plan_tables(&transaction)?;
        transaction.execute_batch(include_str!("../scope/schema.sql"))?;
        Self::initialize_history(&transaction)?;
        Self::initialize_run_context(&transaction)?;
        transaction.execute_batch(include_str!("agent_runtime.sql"))?;
        transaction.pragma_update(None, "user_version", SCHEMA)?;
        transaction.commit()?;
        Ok(())
    }

    pub(in crate::lean) fn db(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.connection.lock().expect("store mutex")
    }

    pub(in crate::lean) fn record_project_identity(
        &self,
        project: &str,
        identity: &str,
    ) -> Result<()> {
        let mut db = self.db();
        let tx = db.transaction()?;
        let previous: Option<String> = tx
            .query_row(
                "SELECT identity FROM project_identities WHERE project=?1",
                [project],
                |row| row.get(0),
            )
            .optional()?;
        crate::require(
            previous.as_deref().is_none_or(|value| value == identity),
            "project identity changed; open the project's own runtime state",
        )?;
        tx.execute("INSERT INTO project_identities(project,identity) VALUES(?1,?2) ON CONFLICT(project) DO NOTHING", params![project,identity])?;
        tx.commit()?;
        Ok(())
    }

    pub(in crate::lean) fn repository_id(&self, project: &str) -> Result<Option<String>> {
        Ok(self
            .db()
            .query_row(
                "SELECT identity FROM project_identities WHERE project=?1",
                [project],
                |row| row.get(0),
            )
            .optional()?)
    }

    pub fn create_conversation(&self, project: &str) -> Result<Conversation> {
        let conversation = Conversation {
            id: crate::id(),
            project: project.into(),
            created_ms: now_ms(),
        };
        let mut db = self.db();
        let tx = db.transaction()?;
        tx.execute(
            "INSERT INTO conversations(id,project,created_ms) VALUES(?1,?2,?3)",
            params![
                conversation.id,
                conversation.project,
                conversation.created_ms
            ],
        )?;
        crate::lean::store::workflow::begin_interview(&tx, &conversation.id)?;
        tx.commit()?;
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

    /// The latest recorded work goal, falling back to the first user request.
    pub fn conversation_goal(&self, conversation: &str) -> Result<String> {
        let db = self.db();
        let mut goal: Option<String> = db.query_row(
            "SELECT s.summary FROM plan_scopes b JOIN scopes s ON s.id=b.scope_id JOIN plans p ON p.id=b.plan_id WHERE p.conversation_id=?1 ORDER BY p.created_ms DESC,p.rowid DESC,b.revision DESC LIMIT 1",
            [conversation],
            |row| row.get(0),
        ).optional()?;
        if goal.is_none() {
            goal = db.query_row(
                "SELECT summary FROM scopes WHERE conversation_id=?1 ORDER BY created_ms DESC,rowid DESC LIMIT 1",
                [conversation],
                |row| row.get(0),
            ).optional()?;
        }
        if let Some(goal) = goal.filter(|goal| !goal.trim().is_empty()) {
            return Ok(goal);
        }
        Ok(db.query_row(
            "SELECT text FROM messages WHERE conversation_id=?1 AND author='user' ORDER BY seq LIMIT 1",
            [conversation],
            |row| self.content.sql_text(row, 0),
        ).optional()?.unwrap_or_default())
    }

    /// Permanently removes a stopped conversation and its persisted history.
    pub fn delete_session(&self, conversation: &str) -> Result<()> {
        // Match add_message's lock order: content writes acquire this before
        // the database mutex and keep it until their reference is committed.
        let _content_write = self.content_write.lock().expect("content write mutex");
        let mut db = self.db();
        let tx = db.transaction()?;
        tx.execute_batch("PRAGMA defer_foreign_keys=ON;")?;
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM conversations WHERE id=?1)",
            [conversation],
            |row| row.get(0),
        )?;
        crate::require(exists, "session does not exist")?;
        let running: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM turns WHERE conversation_id=?1 AND status='running')",
            [conversation],
            |row| row.get(0),
        )?;
        crate::require(!running, "stop this session's work before deleting it")?;
        tx.execute("DELETE FROM result_evidence WHERE turn_id IN (SELECT id FROM turns WHERE conversation_id=?1)", [conversation])?;
        tx.execute("DELETE FROM agent_answer_waits WHERE request IN (SELECT id FROM agent_questions WHERE conversation_id=?1)", [conversation])?;
        tx.execute(
            "DELETE FROM agent_questions WHERE conversation_id=?1",
            [conversation],
        )?;
        tx.execute("DELETE FROM agent_results WHERE turn_id IN (SELECT id FROM turns WHERE conversation_id=?1)", [conversation])?;
        tx.execute("DELETE FROM agent_evidence WHERE turn_id IN (SELECT id FROM turns WHERE conversation_id=?1)", [conversation])?;
        tx.execute("DELETE FROM turn_inputs WHERE turn_id IN (SELECT id FROM turns WHERE conversation_id=?1)", [conversation])?;
        tx.execute("DELETE FROM scope_revision_proposals WHERE request IN (SELECT id FROM plan_revision_jobs WHERE conversation_id=?1)",[conversation])?;
        tx.execute("DELETE FROM plan_scopes WHERE plan_id IN (SELECT id FROM plans WHERE conversation_id=?1)",[conversation])?;
        tx.execute("DELETE FROM scope_documents WHERE scope_id IN (SELECT id FROM scopes WHERE conversation_id=?1)",[conversation])?;
        tx.execute(
            "DELETE FROM scopes WHERE conversation_id=?1",
            [conversation],
        )?;
        tx.execute(
            "DELETE FROM conversation_sources WHERE conversation_id=?1",
            [conversation],
        )?;
        tx.execute("DELETE FROM task_inputs WHERE task_id IN (SELECT id FROM tasks WHERE conversation_id=?1)", [conversation])?;
        tx.execute("DELETE FROM task_result_sources WHERE task_id IN (SELECT id FROM tasks WHERE conversation_id=?1) OR producer_task_id IN (SELECT id FROM tasks WHERE conversation_id=?1)", [conversation])?;
        tx.execute("DELETE FROM task_source_revisions WHERE task_id IN (SELECT id FROM tasks WHERE conversation_id=?1)", [conversation])?;
        tx.execute("DELETE FROM task_artifact_inputs WHERE task_id IN (SELECT id FROM tasks WHERE conversation_id=?1) OR source_task IN (SELECT id FROM tasks WHERE conversation_id=?1)", [conversation])?;
        tx.execute("DELETE FROM task_resume_results WHERE task_id IN (SELECT id FROM tasks WHERE conversation_id=?1) OR source_task IN (SELECT id FROM tasks WHERE conversation_id=?1)", [conversation])?;
        tx.execute("DELETE FROM task_contracts WHERE task_id IN (SELECT id FROM tasks WHERE conversation_id=?1)", [conversation])?;
        tx.execute("DELETE FROM integration_attempts WHERE task_id IN (SELECT id FROM tasks WHERE conversation_id=?1)", [conversation])?;
        tx.execute(
            "DELETE FROM plan_revision_jobs WHERE conversation_id=?1",
            [conversation],
        )?;
        tx.execute("DELETE FROM plan_heads WHERE plan_id IN (SELECT id FROM plans WHERE conversation_id=?1)", [conversation])?;
        tx.execute(
            "DELETE FROM plan_activations WHERE conversation_id=?1",
            [conversation],
        )?;
        tx.execute("DELETE FROM comment_scopes WHERE comment_id IN (SELECT id FROM comments WHERE conversation_id=?1)", [conversation])?;
        tx.execute(
            "DELETE FROM comments WHERE conversation_id=?1",
            [conversation],
        )?;
        tx.execute(
            "DELETE FROM approvals WHERE conversation_id=?1",
            [conversation],
        )?;
        tx.execute(
            "DELETE FROM approval_modes WHERE conversation_id=?1",
            [conversation],
        )?;
        tx.execute("DELETE FROM teams WHERE conversation_id=?1", [conversation])?;
        tx.execute(
            "DELETE FROM request_operations WHERE conversation_id=?1",
            [conversation],
        )?;
        tx.execute(
            "DELETE FROM artifact_deliveries WHERE conversation_id=?1",
            [conversation],
        )?;
        tx.execute(
            "DELETE FROM replan_blockers WHERE conversation_id=?1",
            [conversation],
        )?;
        tx.execute(
            "DELETE FROM agent_operations WHERE conversation_id=?1",
            [conversation],
        )?;
        tx.execute("DELETE FROM mail_scopes WHERE mail_id IN (SELECT id FROM mail WHERE conversation_id=?1)", [conversation])?;
        tx.execute("DELETE FROM mail_sources WHERE mail_id IN (SELECT id FROM mail WHERE conversation_id=?1)", [conversation])?;
        tx.execute("DELETE FROM turn_scopes WHERE turn_id IN (SELECT id FROM turns WHERE conversation_id=?1)", [conversation])?;
        tx.execute("DELETE FROM run_contexts WHERE turn_id IN (SELECT id FROM turns WHERE conversation_id=?1)", [conversation])?;
        tx.execute("DELETE FROM task_rounds WHERE task_id IN (SELECT id FROM tasks WHERE conversation_id=?1)", [conversation])?;
        tx.execute(
            "DELETE FROM checks WHERE task_id IN (SELECT id FROM tasks WHERE conversation_id=?1)",
            [conversation],
        )?;
        tx.execute("DELETE FROM tasks WHERE conversation_id=?1", [conversation])?;
        tx.execute("DELETE FROM turns WHERE conversation_id=?1", [conversation])?;
        tx.execute("DELETE FROM plans WHERE conversation_id=?1", [conversation])?;
        tx.execute(
            "DELETE FROM session_workflow WHERE conversation_id=?1",
            [conversation],
        )?;
        tx.execute(
            "DELETE FROM messages WHERE conversation_id=?1",
            [conversation],
        )?;
        tx.execute(
            "DELETE FROM events WHERE conversation_id=?1",
            [conversation],
        )?;
        tx.execute("DELETE FROM mail WHERE conversation_id=?1", [conversation])?;
        tx.execute(
            "DELETE FROM pauses WHERE conversation_id=?1",
            [conversation],
        )?;
        tx.execute("DELETE FROM conversations WHERE id=?1", [conversation])?;
        tx.commit()?;
        self.content.remove_unreferenced(&mut db)?;
        Ok(())
    }

    /// Stores a message and announces it in the event log in one transaction.
    pub fn add_message(&self, conversation: &str, author: &str, text: &str) -> Result<Message> {
        let _content_write = self.content_write.lock().expect("content write mutex");
        self.add_message_with_content_lock(conversation, author, text)
    }

    pub(in crate::lean) fn add_message_with_images(
        &self,
        conversation: &str,
        author: &str,
        text: &str,
        paths: &[PathBuf],
    ) -> Result<Message> {
        let _content_write = self.content_write.lock().expect("content write mutex");
        let mut images = Vec::with_capacity(paths.len());
        for path in paths {
            let metadata = std::fs::metadata(path).map_err(|error| {
                crate::Error::Invalid(format!("cannot read image {}: {error}", path.display()))
            })?;
            crate::require(
                metadata.is_file(),
                &format!("image attachment is not a regular file: {}", path.display()),
            )?;
            crate::require(
                metadata.len() <= crate::lean::content::MAX_IMAGE_BYTES as u64,
                "image attachment exceeds the 20 MiB limit",
            )?;
            let bytes = std::fs::read(path).map_err(|error| {
                crate::Error::Invalid(format!("cannot read image {}: {error}", path.display()))
            })?;
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("image");
            images.push(self.content.store_image(name, &bytes)?);
        }
        let stored_text = self.content.encode(text)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let seq = insert_message(&tx, conversation, author, &stored_text)?;
        for image in images {
            tx.execute("INSERT INTO message_images(message_seq,image_id,mime_type,bytes,name) VALUES(?1,?2,?3,?4,?5)",
                params![seq as i64, image.id, image.mime_type, image.bytes as i64, image.name])?;
        }
        maybe_restart_workflow(&tx, conversation, author, text)?;
        if author == "user" {
            super::Store::record_scope_confirmation_answer(&tx, conversation, text, seq)?;
        }
        tx.commit()?;
        Ok(Message {
            seq,
            author: author.into(),
            text: text.into(),
        })
    }

    pub(in crate::lean) fn linked_images(
        &self,
        conversation: &str,
    ) -> Result<Vec<(u64, crate::lean::content::ImageReference)>> {
        let db = self.db();
        let mut statement = db.prepare("SELECT message_seq,image_id,mime_type,bytes,name FROM message_images JOIN messages ON messages.seq=message_images.message_seq WHERE messages.conversation_id=?1 ORDER BY message_seq,image_id")?;
        let rows = statement.query_map([conversation], |row| {
            Ok((
                row.get::<_, i64>(0)? as u64,
                crate::lean::content::ImageReference {
                    id: row.get(1)?,
                    mime_type: row.get(2)?,
                    bytes: row.get::<_, i64>(3)? as u64,
                    name: row.get(4)?,
                },
            ))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(crate::Error::from)
    }

    pub(in crate::lean) fn read_image(
        &self,
        reference: &crate::lean::content::ImageReference,
    ) -> Result<Vec<u8>> {
        self.content.read_image(reference)
    }

    pub(super) fn add_message_with_content_lock(
        &self,
        conversation: &str,
        author: &str,
        text: &str,
    ) -> Result<Message> {
        let stored_text = self.content.encode(text)?;
        let mut db = self.db();
        let tx = db.transaction()?;
        let seq = insert_message(&tx, conversation, author, &stored_text)?;
        maybe_restart_workflow(&tx, conversation, author, text)?;
        if author == "user" {
            super::Store::record_scope_confirmation_answer(&tx, conversation, text, seq)?;
        }
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

fn maybe_restart_workflow(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    author: &str,
    text: &str,
) -> Result<()> {
    // An answer to an open decision continues the current goal.
    if author != "user"
        || text.starts_with(crate::lean::COMMENT_PREFIX)
        || text.starts_with("Interview answer (")
    {
        return Ok(());
    }
    if restart_completed_workflow(tx, conversation)? > 0 {
        append_run_event(
            tx,
            conversation,
            "workflow",
            serde_json::json!({"phase":"interview","summary":""}),
        )?;
    }
    Ok(())
}

/// A goal is complete when its latest plan is approved and its final audit,
/// if one ran, did not reject it; a rejected audit leaves remediation open.
fn restart_completed_workflow(tx: &rusqlite::Transaction<'_>, conversation: &str) -> Result<usize> {
    Ok(tx.execute(
        "WITH latest(id) AS (SELECT id FROM plans WHERE conversation_id=?1 ORDER BY created_ms DESC,rowid DESC LIMIT 1)
         UPDATE session_workflow SET phase='interview',summary='',upto_message=0
         WHERE conversation_id=?1 AND phase='implementation'
         AND EXISTS(SELECT 1 FROM tasks WHERE plan_id=(SELECT id FROM latest))
         AND NOT EXISTS(SELECT 1 FROM tasks WHERE plan_id=(SELECT id FROM latest) AND status<>'approved')
         AND NOT EXISTS(SELECT 1 FROM turns WHERE conversation_id=?1 AND status='running')
         AND COALESCE((SELECT json_extract(payload,'$.approved') FROM events
             WHERE conversation_id=?1 AND kind='audit' AND plan_id=(SELECT id FROM latest)
             ORDER BY seq DESC LIMIT 1),1)=1",
        [conversation],
    )?)
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

fn validate_database(connection: &Connection, path: &Path) -> Result<u32> {
    let found: u32 = connection.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let nonempty: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*')",
        [],
        |row| row.get(0),
    )?;
    crate::require(
        found == SCHEMA || (found == 0 && !nonempty),
        &format!(
            "{} has incompatible schema {found}; this build requires {SCHEMA}. Stop the daemon, archive the runtime directory containing this database, then restart to create fresh state. No data was migrated or deleted; client and daemon must be restarted together.",
            path.display()
        ),
    )?;
    Ok(found)
}
