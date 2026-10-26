//! Compact execution metadata. Provider thread, session and turn identifiers
//! have different meanings and must never be used interchangeably.
use super::*;
use std::path::PathBuf;

#[cfg(test)]
#[path = "run_context/tests.rs"]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunAccess {
    ReadOnly,
    Write,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RunContext {
    pub turn_id: String,
    pub conversation: String,
    pub role: String,
    pub provider: String,
    pub profile: String,
    pub cwd: PathBuf,
    pub add_dirs: Vec<PathBuf>,
    pub access: RunAccess,
    pub provider_thread_id: Option<String>,
    pub provider_session_id: Option<String>,
}

impl Store {
    pub(super) fn initialize_run_context(connection: &Connection) -> Result<()> {
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS run_contexts(
                turn_id TEXT PRIMARY KEY REFERENCES turns(id),
                profile TEXT NOT NULL, cwd TEXT NOT NULL, add_dirs TEXT NOT NULL,
                access TEXT NOT NULL CHECK(access IN ('read_only','write')),
                provider_thread_id TEXT, provider_session_id TEXT);
             CREATE TABLE IF NOT EXISTS native_processes(
                pid INTEGER PRIMARY KEY,started_ms INTEGER NOT NULL,provider TEXT NOT NULL);",
        )?;
        Ok(())
    }

    /// Record once, before launching a process. Scope comes from the owning
    /// turn, rather than mutable frontend state. Repeating the same write is
    /// harmless; changing a running turn's scope is rejected.
    pub fn record_run_context(&self, context: &RunContext) -> Result<()> {
        let encoded = encode_context(context)?;
        let mut db = self.db();
        let transaction = db.transaction()?;
        require_running_turn(&transaction, context)?;
        insert_context(&transaction, context, &encoded)?;
        transaction.commit()?;
        Ok(())
    }

    /// A turn that failed before launch has no recorded execution context.
    /// Never reconstruct its permissions from the current configuration.
    pub fn run_context(&self, conversation: &str, turn: &str) -> Result<Option<RunContext>> {
        Self::read_run_context(&self.db(), conversation, turn)
    }

    fn read_run_context(
        db: &Connection,
        conversation: &str,
        turn: &str,
    ) -> Result<Option<RunContext>> {
        let raw = db.query_row(
            "SELECT json_object('turn_id',t.id,'conversation',t.conversation_id,
                'role',t.role,'provider',t.provider,'profile',r.profile,'cwd',r.cwd,
                'add_dirs',json(r.add_dirs),'access',r.access,
                'provider_thread_id',r.provider_thread_id,'provider_session_id',r.provider_session_id)
             FROM run_contexts r JOIN turns t ON t.id=r.turn_id
             WHERE t.conversation_id=?1 AND t.id=?2",
            params![conversation, turn], |row| row.get::<_, String>(0),
        ).optional()?;
        raw.map(|raw| serde_json::from_str(&raw).map_err(Into::into))
            .transpose()
    }
}

struct EncodedContext<'a> {
    cwd: &'a str,
    add_dirs: String,
    access: serde_json::Value,
}

fn encode_context(context: &RunContext) -> Result<EncodedContext<'_>> {
    crate::require(
        !context.profile.is_empty() && context.cwd.is_absolute(),
        "run context requires a profile and an absolute working directory",
    )?;
    crate::require(
        context.add_dirs.iter().all(|path| path.is_absolute()),
        "run context requires absolute additional directories",
    )?;
    let cwd = context
        .cwd
        .to_str()
        .ok_or_else(|| Error::Invalid("working directory is not UTF-8".into()))?;
    Ok(EncodedContext {
        cwd,
        add_dirs: serde_json::to_string(&context.add_dirs)?,
        access: serde_json::to_value(context.access)?,
    })
}

fn require_running_turn(
    transaction: &rusqlite::Transaction<'_>,
    context: &RunContext,
) -> Result<()> {
    let matches: bool = transaction.query_row(
        "SELECT EXISTS(SELECT 1 FROM turns WHERE id=?1 AND conversation_id=?2
                AND role=?3 AND provider=?4 AND status='running')",
        params![
            context.turn_id,
            context.conversation,
            context.role,
            context.provider
        ],
        |row| row.get(0),
    )?;
    crate::require(matches, "run context does not match a running turn")
}

fn insert_context(
    transaction: &rusqlite::Transaction<'_>,
    context: &RunContext,
    encoded: &EncodedContext<'_>,
) -> Result<()> {
    transaction.execute(
        "INSERT INTO run_contexts(turn_id,profile,cwd,add_dirs,access,
                provider_thread_id,provider_session_id)
             VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(turn_id) DO NOTHING",
        params![
            context.turn_id,
            context.profile,
            encoded.cwd,
            encoded.add_dirs,
            encoded.access.as_str(),
            context.provider_thread_id,
            context.provider_session_id
        ],
    )?;
    let stored = Store::read_run_context(transaction, &context.conversation, &context.turn_id)?;
    crate::require(
        stored.as_ref() == Some(context),
        "run context is already bound differently",
    )
}
