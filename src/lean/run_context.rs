//! Compact execution metadata. Provider thread, session and turn identifiers
//! have different meanings and must never be used interchangeably.
use super::*;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunBackend {
    Cli,
    CodexAppServer,
    OpencodeServer,
}

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
    pub backend: RunBackend,
    pub provider_thread_id: Option<String>,
    pub provider_session_id: Option<String>,
    pub provider_turn_id: Option<String>,
}

impl Store {
    pub(super) fn initialize_run_context(connection: &Connection) -> Result<()> {
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS run_contexts(
                turn_id TEXT PRIMARY KEY REFERENCES turns(id),
                profile TEXT NOT NULL, cwd TEXT NOT NULL, add_dirs TEXT NOT NULL,
                access TEXT NOT NULL CHECK(access IN ('read_only','write')),
                backend TEXT NOT NULL CHECK(backend IN ('cli','codex_app_server','opencode_server')),
                provider_thread_id TEXT, provider_session_id TEXT, provider_turn_id TEXT);
             CREATE TABLE IF NOT EXISTS native_processes(
                pid INTEGER PRIMARY KEY,started_ms INTEGER NOT NULL,provider TEXT NOT NULL);",
        )?;
        Ok(())
    }

    /// Record once, before launching a process. Scope comes from the owning
    /// turn, rather than mutable frontend state. Repeating the same write is
    /// harmless; changing a running turn's scope or backend is rejected.
    pub fn record_run_context(&self, context: &RunContext) -> Result<()> {
        crate::require(
            !context.profile.is_empty() && context.cwd.is_absolute(),
            "run context requires a profile and an absolute working directory",
        )?;
        crate::require(
            context.add_dirs.iter().all(|path| path.is_absolute()),
            "run context requires absolute additional directories",
        )?;
        crate::require(
            match context.backend {
                RunBackend::Cli => true,
                RunBackend::CodexAppServer => context.provider == "codex",
                RunBackend::OpencodeServer => context.provider == "opencode",
            },
            "run backend does not match provider",
        )?;
        let cwd = context
            .cwd
            .to_str()
            .ok_or_else(|| Error::Invalid("working directory is not UTF-8".into()))?;
        let add_dirs = serde_json::to_string(&context.add_dirs)?;
        let backend = serde_json::to_value(context.backend)?;
        let access = serde_json::to_value(context.access)?;
        let mut db = self.db();
        let transaction = db.transaction()?;
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
        crate::require(matches, "run context does not match a running turn")?;
        transaction.execute(
            "INSERT INTO run_contexts(turn_id,profile,cwd,add_dirs,access,backend,
                provider_thread_id,provider_session_id,provider_turn_id)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(turn_id) DO NOTHING",
            params![
                context.turn_id,
                context.profile,
                cwd,
                add_dirs,
                access.as_str(),
                backend.as_str(),
                context.provider_thread_id,
                context.provider_session_id,
                context.provider_turn_id
            ],
        )?;
        let stored = Self::read_run_context(&transaction, &context.conversation, &context.turn_id)?;
        crate::require(
            stored.as_ref() == Some(context),
            "run context is already bound differently",
        )?;
        transaction.commit()?;
        Ok(())
    }

    /// Legacy turns deliberately return None: their historical cwd and access
    /// cannot be reconstructed safely from today's configuration.
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
                'add_dirs',json(r.add_dirs),'access',r.access,'backend',r.backend,
                'provider_thread_id',r.provider_thread_id,'provider_session_id',r.provider_session_id,
                'provider_turn_id',r.provider_turn_id)
             FROM run_contexts r JOIN turns t ON t.id=r.turn_id
             WHERE t.conversation_id=?1 AND t.id=?2",
            params![conversation, turn], |row| row.get::<_, String>(0),
        ).optional()?;
        raw.map(|raw| serde_json::from_str(&raw).map_err(Into::into))
            .transpose()
    }

    /// A native adapter binds the provider's turn separately from its thread.
    /// Retrying notification delivery cannot silently reassign that binding.
    pub fn bind_provider_turn(
        &self,
        conversation: &str,
        turn: &str,
        provider_turn: &str,
    ) -> Result<()> {
        crate::require(!provider_turn.is_empty(), "provider turn id is empty")?;
        let changed = self.db().execute(
            "UPDATE run_contexts SET provider_turn_id=?3
             WHERE turn_id=?2 AND backend!='cli'
                AND (provider_turn_id IS NULL OR provider_turn_id=?3)
                AND EXISTS(SELECT 1 FROM turns t WHERE t.id=?2 AND t.conversation_id=?1
                    AND t.status='running')",
            params![conversation, turn, provider_turn],
        )?;
        crate::require(
            changed == 1,
            "provider turn does not match a running native context",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(store: &Store, provider: &str) -> RunContext {
        let conversation = store.create_conversation("project").unwrap();
        let turn = store
            .start_turn(&conversation.id, "orchestrator", provider, 0, None)
            .unwrap();
        RunContext {
            turn_id: turn.id,
            conversation: conversation.id,
            role: turn.role,
            provider: provider.into(),
            profile: "default".into(),
            cwd: std::env::current_dir().unwrap(),
            add_dirs: vec![],
            access: RunAccess::ReadOnly,
            backend: RunBackend::Cli,
            provider_thread_id: None,
            provider_session_id: None,
            provider_turn_id: None,
        }
    }

    #[test]
    fn schema_nine_is_rejected_without_changing_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("core.db");
        let store = Store::open(&path).unwrap();
        let context = context(&store, "codex");
        store.set_session(&context.turn_id, "old-thread").unwrap();
        store
            .db()
            .execute_batch("DROP TABLE run_contexts; PRAGMA user_version=9;")
            .unwrap();
        drop(store);
        let before = std::fs::read(&path).unwrap();
        assert!(Store::open(&path).is_err());
        assert_eq!(before, std::fs::read(&path).unwrap());
    }

    #[test]
    fn context_survives_restart_and_keeps_thread_separate_from_session() {
        for provider in ["codex", "opencode", "claude", "gemini", "agy"] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("core.db");
            let store = Store::open(&path).unwrap();
            let mut context = context(&store, provider);
            store.record_run_context(&context).unwrap();
            store.record_run_context(&context).unwrap();
            store.set_session(&context.turn_id, "provider-id").unwrap();
            if provider == "codex" {
                context.provider_thread_id = Some("provider-id".into());
            } else {
                context.provider_session_id = Some("provider-id".into());
            }
            drop(store);
            let store = Store::open(&path).unwrap();
            assert_eq!(
                store
                    .run_context(&context.conversation, &context.turn_id)
                    .unwrap(),
                Some(context)
            );
        }
    }

    #[test]
    fn context_cannot_change_scope_backend_profile_or_directory() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let original = context(&store, "codex");
        store.record_run_context(&original).unwrap();
        let other = store.create_conversation("other").unwrap();
        assert!(
            store
                .run_context(&other.id, &original.turn_id)
                .unwrap()
                .is_none()
        );
        for field in [
            "conversation",
            "role",
            "provider",
            "profile",
            "cwd",
            "access",
            "backend",
        ] {
            let mut changed = original.clone();
            match field {
                "conversation" => changed.conversation = other.id.clone(),
                "role" => changed.role = "worker:foreign".into(),
                "provider" => changed.provider = "opencode".into(),
                "profile" => changed.profile = "foreign".into(),
                "cwd" => changed.cwd = original.cwd.join("foreign"),
                "access" => changed.access = RunAccess::Write,
                "backend" => changed.backend = RunBackend::CodexAppServer,
                _ => unreachable!(),
            }
            assert!(store.record_run_context(&changed).is_err(), "{field}");
            assert_eq!(
                store
                    .run_context(&original.conversation, &original.turn_id)
                    .unwrap(),
                Some(original.clone())
            );
        }
        store
            .finish_turn(&original.turn_id, TurnStatus::Done, None)
            .unwrap();
        assert!(store.record_run_context(&original).is_err());
    }

    #[test]
    fn native_turn_binding_is_scoped_and_idempotent() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let mut context = context(&store, "codex");
        assert!(
            store
                .bind_provider_turn(&context.conversation, &context.turn_id, "turn-1")
                .is_err()
        );
        context.backend = RunBackend::CodexAppServer;
        store.record_run_context(&context).unwrap();
        assert!(
            store
                .bind_provider_turn("foreign", &context.turn_id, "turn-1")
                .is_err()
        );
        assert!(
            store
                .bind_provider_turn(&context.conversation, &context.turn_id, "")
                .is_err()
        );
        store
            .bind_provider_turn(&context.conversation, &context.turn_id, "turn-1")
            .unwrap();
        store
            .bind_provider_turn(&context.conversation, &context.turn_id, "turn-1")
            .unwrap();
        assert!(
            store
                .bind_provider_turn(&context.conversation, &context.turn_id, "turn-2")
                .is_err()
        );
        let bound = store
            .run_context(&context.conversation, &context.turn_id)
            .unwrap()
            .unwrap();
        assert_eq!(bound.provider_turn_id.as_deref(), Some("turn-1"));
        assert_eq!(bound.provider_thread_id, None);
        assert_eq!(bound.provider_session_id, None);
        store
            .finish_turn(&context.turn_id, TurnStatus::Interrupted, None)
            .unwrap();
        assert!(
            store
                .bind_provider_turn(&context.conversation, &context.turn_id, "turn-1")
                .is_err()
        );
    }

    #[test]
    fn invalid_context_does_not_leave_a_row() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        let mut context = context(&store, "codex");
        context.backend = RunBackend::OpencodeServer;
        assert!(store.record_run_context(&context).is_err());
        context.backend = RunBackend::Cli;
        context.cwd = "relative".into();
        assert!(store.record_run_context(&context).is_err());
        context.cwd = std::env::current_dir().unwrap();
        context.add_dirs.push("relative".into());
        assert!(store.record_run_context(&context).is_err());
        assert_eq!(
            store
                .run_context(&context.conversation, &context.turn_id)
                .unwrap(),
            None
        );
    }

    #[test]
    fn native_session_cannot_be_rebound_and_legacy_resume_updates_atomically() {
        for (provider, backend) in [
            ("codex", RunBackend::CodexAppServer),
            ("opencode", RunBackend::OpencodeServer),
        ] {
            let store = Store::open(Path::new(":memory:")).unwrap();
            let mut context = context(&store, provider);
            context.backend = backend;
            store.record_run_context(&context).unwrap();
            store.set_session(&context.turn_id, "original").unwrap();
            store.set_session(&context.turn_id, "original").unwrap();
            assert!(store.set_session(&context.turn_id, "foreign").is_err());
            assert_eq!(
                store
                    .last_session(&context.conversation, &context.role, provider)
                    .unwrap()
                    .as_deref(),
                Some("original")
            );
            let bound = store
                .run_context(&context.conversation, &context.turn_id)
                .unwrap()
                .unwrap();
            if provider == "codex" {
                assert_eq!(bound.provider_thread_id.as_deref(), Some("original"));
                assert_eq!(bound.provider_session_id, None);
            } else {
                assert_eq!(bound.provider_session_id.as_deref(), Some("original"));
                assert_eq!(bound.provider_thread_id, None);
            }
        }
    }
}
