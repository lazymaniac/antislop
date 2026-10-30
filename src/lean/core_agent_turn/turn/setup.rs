use super::*;

pub(super) fn wait_until_resumed(
    core: &Core,
    setup: &TurnSetup<'_>,
    cancel: &AtomicBool,
) -> Result<()> {
    if ORCHESTRATION_ROLES.contains(&setup.role.as_str()) || setup.role == PLAN_VERIFIER {
        return Ok(());
    }
    while !cancel.load(Ordering::SeqCst) && core.store.paused(setup.conversation)? {
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    Ok(())
}

pub(super) fn begin<'a>(core: &Core, setup: TurnSetup<'a>) -> Result<TurnState<'a>> {
    let start = TurnStart::new(core, setup)?;
    let (inbox, history_mail) =
        pending_inbox(core, start.conversation, &start.role, start.upto_message)?;
    let comments = pending_comments(core, start.conversation, &start.role, start.upto_message)?;
    let turn = core.store.start_turn(
        start.conversation,
        &start.role,
        &start.provider,
        start.upto_message,
        start.plan_id.as_deref(),
    )?;
    attribute_inbox(core, start.conversation, &inbox, &turn.id)?;
    Ok(start.finish(turn, inbox, history_mail, comments))
}

struct TurnStart<'a> {
    session_policy: SessionPolicy,
    conversation: &'a str,
    plan_id: Option<String>,
    role: String,
    profile: Result<ResolvedProfile>,
    cwd: PathBuf,
    add_dirs: Vec<PathBuf>,
    access: agent::Access,
    upto_message: u64,
    provider: String,
    resume: Option<String>,
}

impl<'a> TurnStart<'a> {
    fn new(core: &Core, setup: TurnSetup<'a>) -> Result<Self> {
        let parts = SetupParts::from_setup(setup);
        let provider = profile_provider(&parts.profile);
        let resume = resume_session(core, &parts, &provider)?;
        Ok(Self {
            session_policy: parts.session_policy,
            conversation: parts.conversation,
            plan_id: parts.plan_id,
            role: parts.role,
            profile: parts.profile,
            cwd: parts.cwd,
            add_dirs: parts.add_dirs,
            access: parts.access,
            upto_message: parts.upto_message,
            provider,
            resume,
        })
    }

    fn finish(
        self,
        turn: store::Turn,
        inbox: Vec<store::Mail>,
        history_mail: std::collections::HashSet<u64>,
        comments: Vec<u64>,
    ) -> TurnState<'a> {
        TurnState {
            session_policy: self.session_policy,
            conversation: self.conversation,
            role: self.role,
            profile: self.profile,
            cwd: self.cwd,
            add_dirs: self.add_dirs,
            access: self.access,
            provider: self.provider,
            resume: self.resume,
            inbox,
            history_mail,
            comments,
            turn,
        }
    }
}

fn pending_inbox(
    core: &Core,
    conversation: &str,
    role: &str,
    upto_message: u64,
) -> Result<(Vec<store::Mail>, std::collections::HashSet<u64>)> {
    let inbox = core.store.pending_mail(conversation, role)?;
    if role != ORCHESTRATOR {
        return Ok((inbox, Default::default()));
    }
    let mut visible = Vec::with_capacity(inbox.len());
    let mut history_mail = std::collections::HashSet::new();
    for mail in inbox {
        if mail.kind == "agent_work"
            && let Some(message) = core.store.agent_mail_user_message(conversation, mail.id)?
        {
            if message > upto_message {
                continue;
            }
            history_mail.insert(mail.id);
        }
        visible.push(mail);
    }
    Ok((visible, history_mail))
}

fn pending_comments(
    core: &Core,
    conversation: &str,
    role: &str,
    upto_message: u64,
) -> Result<Vec<u64>> {
    if role != ORCHESTRATOR && role != COMMENT_ORCHESTRATOR {
        return Ok(Vec::new());
    }
    let answered = core.store.last_orchestration_answered(conversation)?;
    Ok(core
        .store
        .comments(conversation, "open")?
        .into_iter()
        .filter(|comment| comment.message > answered && comment.message <= upto_message)
        .map(|comment| comment.id)
        .collect())
}

fn attribute_inbox(
    core: &Core,
    conversation: &str,
    inbox: &[store::Mail],
    turn: &str,
) -> Result<()> {
    let ids = inbox.iter().map(|mail| mail.id).collect::<Vec<_>>();
    core.store.attribute_mail(conversation, &ids, turn)
}

struct SetupParts<'a> {
    session_policy: SessionPolicy,
    conversation: &'a str,
    plan_id: Option<String>,
    role: String,
    profile: Result<ResolvedProfile>,
    cwd: PathBuf,
    add_dirs: Vec<PathBuf>,
    access: agent::Access,
    upto_message: u64,
}

impl<'a> SetupParts<'a> {
    fn from_setup(setup: TurnSetup<'a>) -> Self {
        let TurnSetup {
            session_policy,
            conversation,
            plan_id,
            role,
            profile,
            cwd,
            add_dirs,
            access,
            upto_message,
        } = setup;
        Self {
            session_policy,
            conversation,
            plan_id,
            role,
            profile,
            cwd,
            add_dirs,
            access,
            upto_message,
        }
    }
}

fn profile_provider(profile: &Result<ResolvedProfile>) -> String {
    profile
        .as_ref()
        .map_or("unconfigured", |profile| agent::name(profile.provider))
        .to_string()
}

fn resume_session(core: &Core, setup: &SetupParts<'_>, provider: &str) -> Result<Option<String>> {
    core.session_for_turn(
        setup.conversation,
        &setup.role,
        provider,
        &setup.session_policy,
        setup.profile.as_ref().ok(),
        &setup.cwd,
        &setup.add_dirs,
        setup.access,
    )
}
