use super::*;

const STARTUP_RETRIES: u32 = 2;

#[path = "turn/execute.rs"]
mod execute;
#[path = "turn/finish.rs"]
mod finish;
#[path = "turn/setup.rs"]
mod setup;

pub(super) struct TurnState<'a> {
    pub(super) session_policy: SessionPolicy,
    pub(super) conversation: &'a str,
    pub(super) role: String,
    pub(super) profile: Result<ResolvedProfile>,
    pub(super) cwd: PathBuf,
    pub(super) add_dirs: Vec<PathBuf>,
    pub(super) access: agent::Access,
    pub(super) provider: String,
    pub(super) resume: Option<String>,
    pub(super) inbox: Vec<store::Mail>,
    pub(super) history_mail: std::collections::HashSet<u64>,
    pub(super) comments: Vec<u64>,
    pub(super) turn: store::Turn,
}

pub(super) struct TurnEvents<'a> {
    store: &'a store::Store,
    conversation: String,
    turn: store::Turn,
    inbox: Vec<store::Mail>,
    pub(super) persistence_error: std::cell::RefCell<Option<Error>>,
}

impl<'a> TurnEvents<'a> {
    fn new(core: &'a Core, state: &TurnState<'_>) -> Self {
        Self {
            store: &core.store,
            conversation: state.conversation.into(),
            turn: state.turn.clone(),
            inbox: state.inbox.clone(),
            persistence_error: std::cell::RefCell::new(None),
        }
    }

    pub(super) fn announce(&self, kind: &str, payload: serde_json::Value) {
        if let Err(error) = self.store.append_event(
            &self.conversation,
            Some(&self.turn.id),
            self.turn.plan_id.as_deref(),
            kind,
            &payload,
        ) {
            self.persistence_error.borrow_mut().get_or_insert(error);
        }
    }
}

impl Core {
    pub(in crate::lean) fn turn_inner(
        &self,
        setup: TurnSetup<'_>,
        cancel: &AtomicBool,
        prompt: impl FnOnce(bool, bool) -> String,
    ) -> Result<Option<String>> {
        let lock = self.turn_lock(setup.conversation, &setup.role);
        let Some(_guard) = mail::lock_cancellable(&lock, cancel) else {
            return Ok(None);
        };
        let Some(state) = TurnState::begin(self, setup, cancel)? else {
            return Ok(None);
        };
        let events = TurnEvents::new(self, &state);
        announce_started(&state, &events)?;
        let outcome = execute::execute(self, state, cancel, prompt, &events);
        finish::finish(self, outcome, events, cancel)
    }
}

impl TurnState<'_> {
    fn begin<'a>(
        core: &Core,
        setup: TurnSetup<'a>,
        cancel: &AtomicBool,
    ) -> Result<Option<TurnState<'a>>> {
        setup::wait_until_resumed(core, &setup, cancel)?;
        setup::begin(core, setup).map(Some)
    }
}

fn announce_started(state: &TurnState<'_>, events: &TurnEvents<'_>) -> Result<()> {
    events.announce(
        "turn_started",
        json!({
            "role": state.role,
            "provider": state.provider,
            "model": state.profile.as_ref().ok().and_then(|profile| profile.model.clone()),
            "effort": state.profile.as_ref().ok().and_then(|profile| profile.native_effort.clone()),
            "comments": state.comments,
            "execution_signature": state.profile.as_ref().ok().map(rounds::execution_signature).transpose()?,
        }),
    );
    Ok(())
}
