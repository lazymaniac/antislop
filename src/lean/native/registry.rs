use super::*;

impl Registry {
    pub(crate) fn reap_idle(&self) -> Result<()> {
        let retired = {
            let mut sessions = self.sessions.lock().map_err(invalid)?;
            let keys = sessions
                .iter()
                .filter(|(_, session)| {
                    Arc::strong_count(session) == 1
                        && !session.executing.load(Ordering::SeqCst)
                        && session.touched.lock().is_ok_and(|at| at.elapsed() > IDLE)
                        && session
                            .terminal
                            .lock()
                            .is_ok_and(|mut terminal| !terminal.attached())
                })
                .map(|(key, _)| key.clone())
                .collect::<Vec<_>>();
            keys.into_iter()
                .filter_map(|key| sessions.remove(&key))
                .collect::<Vec<_>>()
        };
        drop(retired);
        Ok(())
    }

    /// Starts the role's CLI again from its recorded run, resuming the
    /// provider's own session. This is how an idle or earlier session is
    /// looked at: the provider's history is the only history. `role#rN` names
    /// the session of round N, which is resumed read-only.
    pub(crate) fn restore(
        &self,
        core: &Core,
        conversation: &str,
        role_key: &str,
    ) -> Result<Arc<Session>> {
        let (role, round) = parse_role_key(role_key)?;
        let turn_id = recorded_turn(core, conversation, role, round)?;
        let context = recorded_context(core, conversation, &turn_id)?;
        let (spec, signature) = restored_spec(core, &context, round)?;
        require(
            spec.resume.is_some(),
            "the provider has not reported this session's identifier yet",
        )?;
        verify_execution(core, &turn_id, role, round, &signature)?;
        let scope = turn_scope(core, &turn_id)?
            .map(|(task, round)| {
                format!(
                    "{:?}:{}:{}",
                    super::super::SessionPolicy::TaskRound { task, round },
                    context.profile,
                    signature
                )
            })
            .unwrap_or_default();
        self.prepare_scoped(core, conversation, role_key, &spec, &scope)
    }

    pub(crate) fn shutdown(&self) -> Result<()> {
        let mut sessions = self.sessions.lock().map_err(invalid)?;
        for session in sessions.values() {
            session.stop.store(true, Ordering::SeqCst);
        }
        sessions.clear();
        Ok(())
    }

    /// The role's running session, or a new one when its specification
    /// (apart from resume) differs or its CLI has ended.
    pub(crate) fn prepare_scoped(
        &self,
        core: &Core,
        conversation: &str,
        role: &str,
        spec: &agent::Spec,
        scope: &str,
    ) -> Result<Arc<Session>> {
        let signature = session_signature(role, spec, scope);
        let key = (conversation.to_owned(), role.to_owned());
        let mut sessions = self.sessions.lock().map_err(invalid)?;
        // The live CLI continues its own conversation; a turn that wants a
        // different session (or a fresh one after a restart) needs another CLI.
        if let Some(session) = sessions.get(&key)
            && session.signature == signature
            && !session.stop.load(Ordering::SeqCst)
            && (spec.resume == session.identity() || !session.primed.load(Ordering::SeqCst))
            && session.check_alive().is_ok()
        {
            return Ok(session.clone());
        }
        sessions.remove(&key);
        let session = Arc::new(Session::start(core, spec.clone(), signature, role)?);
        sessions.insert(key, session.clone());
        Ok(session)
    }

    pub(crate) fn get(&self, conversation: &str, role: &str) -> Result<Option<Arc<Session>>> {
        Ok(self
            .sessions
            .lock()
            .map_err(invalid)?
            .get(&(conversation.into(), role.into()))
            .cloned())
    }

    /// Identifiers other live sessions already own.
    pub(super) fn claimed(&self) -> Vec<String> {
        self.sessions.lock().map_or_else(
            |_| vec![],
            |sessions| {
                sessions
                    .values()
                    .filter_map(|session| session.identity())
                    .collect()
            },
        )
    }
}

fn session_signature(role: &str, spec: &agent::Spec, scope: &str) -> String {
    let mut stable = spec.clone();
    stable.resume = None;
    if scope.is_empty() {
        format!("{role}:{stable:?}")
    } else {
        format!("{role}:{scope}:{stable:?}")
    }
}

fn parse_role_key(role_key: &str) -> Result<(&str, Option<u32>)> {
    match role_key.split_once("#r") {
        Some((role, round)) => Ok((
            role,
            Some(
                round
                    .parse()
                    .map_err(|_| invalid("invalid round in the session name"))?,
            ),
        )),
        None => Ok((role_key, None)),
    }
}

fn recorded_turn(
    core: &Core,
    conversation: &str,
    role: &str,
    round: Option<u32>,
) -> Result<String> {
    if let Some(round) = round {
        return recorded_round(core, conversation, role, round);
    }
    core.store
        .last_turn(conversation, role)?
        .map(|turn| turn.id)
        .ok_or_else(|| invalid("native session has not started"))
}

fn recorded_round(core: &Core, conversation: &str, role: &str, round: u32) -> Result<String> {
    core.store
        .db()
        .query_row(
            "SELECT t.id FROM turns t JOIN turn_scopes s ON s.turn_id=t.id
         WHERE t.conversation_id=?1 AND t.role=?2 AND s.round=?3 AND t.session_id IS NOT NULL
         ORDER BY t.started_ms DESC,t.rowid DESC LIMIT 1",
            rusqlite::params![conversation, role, round],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| invalid("no session was recorded for that round"))
}

fn recorded_context(core: &Core, conversation: &str, turn: &str) -> Result<store::RunContext> {
    let context = core
        .store
        .run_context(conversation, turn)?
        .ok_or_else(|| invalid("this run recorded no working directory to resume in"))?;
    require(
        context.cwd.is_dir(),
        "the session's working directory no longer exists; start new work",
    )?;
    Ok(context)
}

fn restored_spec(
    core: &Core,
    context: &store::RunContext,
    round: Option<u32>,
) -> Result<(agent::Spec, String)> {
    let config = core.config_snapshot();
    let profile = config
        .profiles
        .get(&context.profile)
        .ok_or_else(|| invalid("the session's profile no longer exists"))?;
    validate_profile(profile, context)?;
    let provider = config
        .providers
        .get(&profile.provider)
        .ok_or_else(|| invalid("the session's provider is not configured"))?;
    let (binary, args) = crate::config::resolve_command(provider, profile)?;
    let spec = restored_agent_spec(context, profile, round, binary, args);
    let signature = super::super::rounds::spec_signature(&context.profile, profile.effort, &spec)?;
    Ok((spec, signature))
}

fn validate_profile(profile: &crate::config::Profile, context: &store::RunContext) -> Result<()> {
    require(
        agent::name(profile.provider) == context.provider,
        "the session's provider changed; start a new turn before attaching",
    )
}

fn restored_agent_spec(
    context: &store::RunContext,
    profile: &crate::config::Profile,
    round: Option<u32>,
    binary: std::path::PathBuf,
    args: Vec<String>,
) -> agent::Spec {
    agent::Spec {
        provider: profile.provider,
        binary,
        args,
        model: profile.model.clone(),
        effort: profile
            .provider
            .native_effort(profile.effort)
            .map(str::to_owned),
        cwd: context.cwd.clone(),
        add_dirs: context.add_dirs.clone(),
        access: if context.access == store::RunAccess::Write && round.is_none() {
            agent::Access::Write
        } else {
            agent::Access::ReadOnly
        },
        resume: context
            .provider_thread_id
            .clone()
            .or(context.provider_session_id.clone()),
    }
}

fn verify_execution(
    core: &Core,
    turn: &str,
    role: &str,
    round: Option<u32>,
    signature: &str,
) -> Result<()> {
    if round.is_some() || super::super::task_role(role).is_none() {
        return Ok(());
    }
    let recorded: Option<String> = core
        .store
        .db()
        .query_row(
            "SELECT json_extract(payload,'$.execution_signature') FROM events
         WHERE turn_id=?1 AND kind='turn_started' ORDER BY seq DESC LIMIT 1",
            [turn],
            |row| row.get(0),
        )
        .optional()?
        .flatten();
    require(
        recorded.as_deref() == Some(signature),
        "the execution profile changed; start a fresh task turn before attaching",
    )
}
