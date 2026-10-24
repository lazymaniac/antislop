use super::*;

impl Core {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn matching_round_session(
        &self,
        conversation: &str,
        role: &str,
        provider: &str,
        task: &str,
        round: u32,
        profile: Option<&ResolvedProfile>,
        cwd: &Path,
        add_dirs: &[PathBuf],
        access: agent::Access,
    ) -> Result<Option<String>> {
        let Some((turn, Some(session))) = self.scoped_turn(conversation, role, task, round)? else {
            return Ok(None);
        };
        self.session_for_context(
            conversation,
            provider,
            &turn,
            session,
            profile,
            cwd,
            add_dirs,
            access,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn session_for_context(
        &self,
        conversation: &str,
        provider: &str,
        turn: &str,
        session: String,
        profile: Option<&ResolvedProfile>,
        cwd: &Path,
        add_dirs: &[PathBuf],
        access: agent::Access,
    ) -> Result<Option<String>> {
        let Some(context) = self.store.run_context(conversation, turn)? else {
            return Ok(None);
        };
        self.session_matches_context(
            conversation,
            provider,
            turn,
            profile,
            cwd,
            add_dirs,
            access,
            &context,
        )
        .map(|matches| matches.then_some(session))
    }

    fn scoped_turn(
        &self,
        conversation: &str,
        role: &str,
        task: &str,
        round: u32,
    ) -> Result<Option<(String, Option<String>)>> {
        Ok(self
            .store
            .db()
            .query_row(
                "SELECT t.id,t.session_id FROM turns t JOIN turn_scopes s ON s.turn_id=t.id
             WHERE t.conversation_id=?1 AND t.role=?2
             AND s.task_id=?3 AND s.round=?4
             ORDER BY t.started_ms DESC,t.rowid DESC LIMIT 1",
                params![conversation, role, task, round],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    #[allow(clippy::too_many_arguments)]
    fn session_matches_context(
        &self,
        conversation: &str,
        provider: &str,
        turn: &str,
        profile: Option<&ResolvedProfile>,
        cwd: &Path,
        add_dirs: &[PathBuf],
        access: agent::Access,
        context: &store::RunContext,
    ) -> Result<bool> {
        let signature = self.execution_signature(conversation, turn)?;
        let expected = profile.map(execution_signature).transpose()?;
        let access = match access {
            agent::Access::ReadOnly => store::RunAccess::ReadOnly,
            agent::Access::Write => store::RunAccess::Write,
        };
        Ok(signature.is_some()
            && signature == expected
            && context.provider == provider
            && profile.is_some_and(|profile| profile.profile == context.profile)
            && context.cwd == cwd
            && context.add_dirs == add_dirs
            && context.access == access)
    }

    fn execution_signature(&self, conversation: &str, turn: &str) -> Result<Option<String>> {
        Ok(self.store.db().query_row(
            "SELECT json_extract(payload,'$.execution_signature') FROM events
             WHERE conversation_id=?1 AND turn_id=?2 AND kind='turn_started' ORDER BY seq DESC LIMIT 1",
            params![conversation, turn],
            |row| row.get(0),
        ).optional()?.flatten())
    }
}
