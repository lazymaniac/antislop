//! Pure durable session/turn transitions. Persistence is acknowledged before the
//! caller may proceed; unknown delivery is never an automatic replay request.
use crate::{
    Result,
    config::RoleKind,
    domain::{Run, Stage},
    hash,
    providers::{FailureKind, ProviderEvent, Support, TurnReceipt, UsageReport},
    require,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SessionScope {
    pub agent_id: String,
    pub task_id: Option<String>,
    pub role: RoleKind,
    pub provider: String,
    pub binary_version: String,
    pub stage: Stage,
    pub spec_revision: u64,
    pub policy_revision: u64,
    pub workspace: String,
    pub input_hash: String,
    pub configuration_hash: String,
}
impl SessionScope {
    /// Authenticate completed work against its actual policy revision. Mode
    /// transitions do not rewrite old sessions or authorize a new reservation.
    pub(crate) fn matches_run(
        &self,
        run: &Run,
        agent_id: String,
        task_id: Option<String>,
        role: RoleKind,
    ) -> Result<bool> {
        let mut expected = Self::for_run(
            run,
            agent_id,
            task_id,
            role,
            self.provider.clone(),
            self.binary_version.clone(),
        )?;
        if run.mode_changes.is_empty() {
            return Ok(*self == expected);
        }
        let configurations = run.mode_configurations()?;
        if let Some(config) = configurations.iter().find(|config| {
            config.content_hash == self.configuration_hash
                && config.config.policy.revision == self.policy_revision
        }) {
            expected.configuration_hash = config.content_hash.clone();
            expected.policy_revision = config.config.policy.revision;
        }
        Ok(*self == expected)
    }

    pub fn for_run(
        run: &Run,
        agent_id: String,
        task_id: Option<String>,
        role: RoleKind,
        provider: String,
        binary_version: String,
    ) -> Result<Self> {
        let scope = Self {
            agent_id,
            task_id,
            role,
            provider,
            binary_version,
            stage: run.stage,
            spec_revision: run.spec_revision,
            policy_revision: run
                .spec
                .config
                .as_ref()
                .map_or(0, |c| c.config.policy.revision),
            workspace: std::path::Path::new(&run.spec.workspace)
                .canonicalize()?
                .to_string_lossy()
                .into(),
            input_hash: hash(serde_json::to_vec(&run.input_snapshot)?),
            configuration_hash: run
                .spec
                .config
                .as_ref()
                .map(|c| c.content_hash.clone())
                .unwrap_or(hash(serde_json::to_vec(&(
                    &run.spec.provider,
                    &run.spec.model,
                ))?)),
        };
        scope.validate()?;
        Ok(scope)
    }
    pub fn validate(&self) -> Result<()> {
        require(
            text(&self.agent_id)
                && text(&self.provider)
                && text(&self.binary_version)
                && text(&self.workspace),
            "session scope identity is incomplete",
        )?;
        require(
            self.task_id.as_ref().is_none_or(|id| text(id)),
            "empty session task identity",
        )?;
        require(
            self.spec_revision > 0 && digest(&self.input_hash) && digest(&self.configuration_hash),
            "session scope revision/hashes are invalid",
        )
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TurnState {
    PendingUnknown,
    Pending,
    Sent,
    DeliveryUnknown,
    Completed,
    Stopped,
    Failed,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SessionRecord {
    pub id: String,
    pub scope: SessionScope,
    pub native_session_id: Option<String>,
    pub native_observed: bool,
    pub state: TurnState,
    pub current_turn_id: Option<String>,
}
impl SessionRecord {
    pub fn new(id: String, scope: SessionScope) -> Result<Self> {
        require(text(&id), "managed session ID required")?;
        scope.validate()?;
        Ok(Self {
            id,
            scope,
            native_session_id: None,
            native_observed: false,
            state: TurnState::PendingUnknown,
            current_turn_id: None,
        })
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OutputRef {
    pub document: String,
    pub hash: String,
}
impl OutputRef {
    pub fn validate(&self) -> Result<()> {
        require(
            text(&self.document) && digest(&self.hash),
            "invalid durable output reference",
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RejectedOutput {
    pub reason: String,
    pub hash: String,
    pub output: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct TurnRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejected_output: Option<RejectedOutput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<crate::context::transport::Transport>,
    pub id: String,
    pub session_id: String,
    pub prompt_hash: String,
    pub schema_hash: String,
    pub context_packet_hash: Option<String>,
    pub state: TurnState,
    pub provider_completed: bool,
    pub cancellation_requested: bool,
    pub output_ref: Option<OutputRef>,
    /// Private persisted fallback until the runtime has an artifact/blob store.
    pub output: Option<String>,
    pub usage: UsageReport,
    pub failure: Option<FailureKind>,
}
impl TurnRecord {
    pub fn new(
        id: String,
        session_id: String,
        prompt_hash: String,
        schema_hash: String,
        context_packet_hash: Option<String>,
    ) -> Result<Self> {
        let turn = Self {
            rejected_output: None,
            context: None,
            id,
            session_id,
            prompt_hash,
            schema_hash,
            context_packet_hash,
            state: TurnState::PendingUnknown,
            provider_completed: false,
            cancellation_requested: false,
            output_ref: None,
            output: None,
            usage: UsageReport::Unavailable,
            failure: None,
        };
        turn.validate()?;
        Ok(turn)
    }
    fn validate(&self) -> Result<()> {
        if let Some(rejected) = &self.rejected_output {
            require(
                rejected.reason == "incomplete_context"
                    && rejected.hash == hash(&rejected.output)
                    && !self.provider_completed
                    && self.state != TurnState::Completed
                    && self.context.as_ref().is_some_and(|c| c.complete().is_err()),
                "rejected native output changed or became an accepted turn",
            )?;
        }
        if let Some(context) = &self.context {
            context.validate()?;
            require(
                context.bundle.scope.turn_id == self.id
                    && context.bundle.scope.prompt_hash == self.prompt_hash
                    && context.bundle.scope.schema_hash == self.schema_hash,
                "context does not belong to this prepared turn",
            )?;
            if self.provider_completed || self.state == TurnState::Completed {
                context.complete()?;
            }
        }
        require(
            text(&self.id)
                && text(&self.session_id)
                && digest(&self.prompt_hash)
                && digest(&self.schema_hash)
                && self.context_packet_hash.as_ref().is_none_or(|h| digest(h)),
            "invalid durable turn identity/hashes",
        )?;
        if self.state == TurnState::Completed {
            require(
                self.provider_completed && self.output_ref.is_some(),
                "completed turn requires committed provider completion and output",
            )?;
        }
        if let Some(reference) = &self.output_ref {
            reference.validate()?;
            if let Some(output) = &self.output {
                require(
                    hash(output) == reference.hash,
                    "durable output hash mismatch",
                )?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SessionLedger {
    pub revision: u64,
    pub sessions: BTreeMap<String, SessionRecord>,
    pub turns: BTreeMap<String, TurnRecord>,
    /// Durable reservation order. Older turns absent from this list have
    /// unknown chronology; UUID/map ordering is not a substitute for time.
    #[serde(default)]
    pub turn_order: Vec<String>,
}
pub type Persist<'a> = dyn FnMut(&SessionLedger, &str) -> Result<()> + 'a;
impl SessionLedger {
    pub fn validate_for_run(&self, run_id: &str) -> Result<()> {
        self.validate()?;
        require(
            self.turns.values().all(|turn| {
                turn.context
                    .as_ref()
                    .is_none_or(|c| c.bundle.scope.run_id == run_id)
            }),
            "context transport belongs to another run",
        )
    }
    pub fn validate(&self) -> Result<()> {
        let mut ordered = BTreeSet::new();
        for id in &self.turn_order {
            require(
                self.turns.contains_key(id) && ordered.insert(id),
                "turn order contains an unknown or repeated turn",
            )?;
        }
        let mut native = BTreeMap::new();
        for (id, session) in &self.sessions {
            require(
                id == &session.id && text(id),
                "session map identity mismatch",
            )?;
            session.scope.validate()?;
            if let Some(raw) = &session.native_session_id {
                require(text(raw), "empty native provider session ID")?;
                let canonical = match uuid::Uuid::parse_str(raw) {
                    Ok(uuid) => uuid.to_string(),
                    Err(_) if session.scope.provider == "fake" => raw.clone(),
                    Err(_) => {
                        return Err(crate::Error::Invalid(
                            "native session ID is not a UUID".into(),
                        ));
                    }
                };
                let key = (&session.scope.provider, canonical);
                require(
                    native.insert(key, id).is_none(),
                    "native provider session cannot belong to two managed sessions",
                )?;
            }
            if let Some(turn) = &session.current_turn_id {
                require(
                    self.turns.get(turn).is_some_and(|t| t.session_id == *id),
                    "current turn is not bound to session",
                )?;
            }
        }
        for (id, turn) in &self.turns {
            require(
                id == &turn.id && self.sessions.contains_key(&turn.session_id),
                "turn map or managed session mismatch",
            )?;
            turn.validate()?;
            if let Some(context) = &turn.context {
                let scope = &self.sessions[&turn.session_id].scope;
                require(
                    context.matches_provider(&scope.provider)
                        && context.bundle.scope.spec_revision == scope.spec_revision
                        && context.bundle.scope.input_hash == scope.input_hash,
                    "persisted context differs from its session scope",
                )?;
            }
        }
        Ok(())
    }
    fn commit(
        &mut self,
        kind: &str,
        change: impl FnOnce(&mut Self) -> Result<()>,
        persist: &mut Persist<'_>,
    ) -> Result<()> {
        let mut next = self.clone();
        change(&mut next)?;
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or_else(|| crate::Error::Invalid("session revision overflow".into()))?;
        next.validate()?;
        persist(&next, kind)?;
        *self = next;
        Ok(())
    }
    pub fn reserve(
        &mut self,
        session: SessionRecord,
        turn: TurnRecord,
        persist: &mut Persist<'_>,
    ) -> Result<()> {
        self.reserve_inner(session, turn, None, persist)
    }
    /// The caller separately reconciles effects and authorizes this exact
    /// continuation. A historical delivery-unknown turn is never overwritten.
    pub fn reserve_resumed(
        &mut self,
        session: SessionRecord,
        turn: TurnRecord,
        previous_turn_id: &str,
        persist: &mut Persist<'_>,
    ) -> Result<()> {
        let previous = self
            .turns
            .get(previous_turn_id)
            .ok_or_else(|| crate::Error::Invalid("resume checkpoint missing".into()))?;
        let current = self
            .sessions
            .get(&session.id)
            .ok_or_else(|| crate::Error::Invalid("resume session missing".into()))?;
        require(
            previous.session_id == session.id
                && turn.id != previous.id
                && turn.state == TurnState::PendingUnknown
                && previous.output.is_none()
                && previous.output_ref.is_none()
                && current.current_turn_id.as_deref() == Some(previous_turn_id)
                && current.state == previous.state
                && current.native_observed
                && current.native_session_id.is_some()
                && current.native_session_id == session.native_session_id
                && matches!(
                    previous.state,
                    TurnState::Failed | TurnState::Stopped | TurnState::DeliveryUnknown
                ),
            "resume requires the exact observed incomplete session checkpoint",
        )?;
        self.reserve_inner(session, turn, Some(previous_turn_id), persist)
    }
    fn reserve_inner(
        &mut self,
        session: SessionRecord,
        turn: TurnRecord,
        resumed_from: Option<&str>,
        persist: &mut Persist<'_>,
    ) -> Result<()> {
        require(
            session.id == turn.session_id,
            "turn reserved for a different managed session",
        )?;
        if let Some(existing) = self.turns.get(&turn.id) {
            require(
                existing.session_id == turn.session_id
                    && existing.prompt_hash == turn.prompt_hash
                    && existing.schema_hash == turn.schema_hash
                    && existing.context_packet_hash == turn.context_packet_hash
                    && self
                        .sessions
                        .get(&session.id)
                        .is_some_and(|saved| saved.scope == session.scope),
                "turn ID reused with changed payload",
            )?;
            return Ok(());
        }
        self.commit(
            "turn_reserved",
            move |next| {
                if let Some(existing) = next.sessions.get(&session.id) {
                    require(
                        existing.scope == session.scope,
                        "managed session scope changed; replacement required",
                    )?;
                    require(
                        (resumed_from.is_some() && existing.state == TurnState::DeliveryUnknown)
                            || matches!(
                                existing.state,
                                TurnState::Completed | TurnState::Stopped | TurnState::Failed
                            ),
                        "managed session still has an unresolved turn",
                    )?;
                } else {
                    next.sessions.insert(session.id.clone(), session.clone());
                }
                let record = next
                    .sessions
                    .get_mut(&session.id)
                    .expect("reserved session");
                record.current_turn_id = Some(turn.id.clone());
                record.state = turn.state;
                next.turn_order.push(turn.id.clone());
                next.turns.insert(turn.id.clone(), turn);
                Ok(())
            },
            persist,
        )
    }
    pub fn record_provider_event(
        &mut self,
        turn_id: &str,
        event: &ProviderEvent,
        persist: &mut Persist<'_>,
    ) -> Result<()> {
        let event_turn = match event {
            ProviderEvent::OutputRejected { turn_id, .. } => Some(turn_id.as_str()),
            ProviderEvent::ContextPrepared { turn_id, .. }
            | ProviderEvent::ContextRead { turn_id, .. }
            | ProviderEvent::ContextObserved { turn_id, .. } => Some(turn_id.as_str()),
            ProviderEvent::DeliveryPendingUnknown { turn_id }
            | ProviderEvent::DeliveryPending { turn_id, .. }
            | ProviderEvent::DeliverySent { turn_id }
            | ProviderEvent::CancellationRequested { turn_id }
            | ProviderEvent::Stopped { turn_id }
            | ProviderEvent::Completed { turn_id } => Some(turn_id.as_str()),
            _ => None,
        };
        require(
            event_turn.is_none_or(|id| id == turn_id),
            "provider callback turn identity mismatch",
        )?;
        self.commit(
            "provider_event",
            |next| {
                let turn = next
                    .turns
                    .get_mut(turn_id)
                    .ok_or_else(|| crate::Error::Invalid("unknown durable turn".into()))?;
                let session = next
                    .sessions
                    .get_mut(&turn.session_id)
                    .expect("bound session");
                require(
                    session.current_turn_id.as_deref() == Some(turn_id),
                    "callback belongs to an old turn",
                )?;
                require(
                    !matches!(
                        turn.state,
                        TurnState::Completed
                            | TurnState::Stopped
                            | TurnState::Failed
                            | TurnState::DeliveryUnknown
                    ),
                    "callback after terminal/unknown turn state",
                )?;
                match event {
                    ProviderEvent::OutputRejected { output, .. } => {
                        require(
                            turn.state == TurnState::Sent
                                && session.native_observed
                                && turn.rejected_output.is_none()
                                && !turn.provider_completed
                                && turn.context.as_ref().is_some_and(|c| c.complete().is_err()),
                            "native rejection lacks an incomplete context turn",
                        )?;
                        turn.rejected_output = Some(RejectedOutput {
                            reason: "incomplete_context".into(),
                            hash: hash(output),
                            output: output.clone(),
                        });
                    }
                    ProviderEvent::ContextPrepared { transport, .. } => {
                        require(
                            turn.state == TurnState::PendingUnknown
                                && turn.context.is_none()
                                && transport.matches_provider(&session.scope.provider)
                                && transport.bundle.scope.spec_revision
                                    == session.scope.spec_revision
                                && transport.bundle.scope.input_hash == session.scope.input_hash
                                && transport.receipts.is_empty()
                                && transport.observed.is_empty(),
                            "context preparation is out of scope or after native delivery",
                        )?;
                        turn.context = Some(*transport.clone());
                    }
                    ProviderEvent::ContextRead { receipt, .. } => {
                        require(
                            turn.state == TurnState::Sent && session.native_observed,
                            "context read before observed native delivery",
                        )?;
                        turn.context
                            .as_mut()
                            .ok_or_else(|| {
                                crate::Error::Invalid("context was not prepared".into())
                            })?
                            .accept(receipt.clone())?;
                    }
                    ProviderEvent::ContextObserved { sequence, .. } => {
                        require(
                            turn.state == TurnState::Sent && session.native_observed,
                            "context observation before native delivery",
                        )?;
                        turn.context
                            .as_mut()
                            .ok_or_else(|| {
                                crate::Error::Invalid("context was not prepared".into())
                            })?
                            .observe(*sequence)?;
                    }
                    ProviderEvent::DeliveryPendingUnknown { .. } => require(
                        turn.state == TurnState::PendingUnknown,
                        "unknown delivery intent out of order",
                    )?,
                    ProviderEvent::DeliveryPending { session_id, .. } => {
                        require(
                            matches!(turn.state, TurnState::PendingUnknown | TurnState::Pending),
                            "delivery intent after send",
                        )?;
                        bind_native(session, session_id, false)?;
                        turn.state = TurnState::Pending;
                    }
                    ProviderEvent::DeliverySent { .. } => {
                        require(
                            matches!(turn.state, TurnState::PendingUnknown | TurnState::Pending),
                            "duplicate/out-of-order send receipt",
                        )?;
                        turn.state = TurnState::Sent;
                    }
                    ProviderEvent::SessionObserved { session_id } => {
                        bind_native(session, session_id, true)?
                    }
                    ProviderEvent::Usage { report } => turn.usage = report.clone(),
                    ProviderEvent::CancellationRequested { .. } => {
                        turn.cancellation_requested = true
                    }
                    ProviderEvent::Stopped { .. } => {
                        require(
                            turn.cancellation_requested,
                            "stop receipt without cancellation request",
                        )?;
                        turn.state = TurnState::Stopped;
                    }
                    ProviderEvent::Completed { .. } => {
                        require(
                            turn.state == TurnState::Sent
                                && session.native_observed
                                && !turn.provider_completed,
                            "completion before durable send/native session or duplicated",
                        )?;
                        turn.provider_completed = true;
                    }
                }
                session.state = turn.state;
                Ok(())
            },
            persist,
        )
    }
    pub fn complete(
        &mut self,
        turn_id: &str,
        receipt: &TurnReceipt,
        reference: OutputRef,
        persist: &mut Persist<'_>,
    ) -> Result<()> {
        reference.validate()?;
        require(
            receipt.turn_id == turn_id
                && receipt.delivered
                && receipt.completed
                && receipt.stopped
                && hash(&receipt.output) == reference.hash,
            "invalid completed CLI receipt/output",
        )?;
        if let Some(turn) = self.turns.get(turn_id)
            && turn.state == TurnState::Completed
        {
            let session = self
                .sessions
                .get(&turn.session_id)
                .expect("validated session");
            require(
                turn.output_ref.as_ref() == Some(&reference)
                    && turn.output.as_deref() == Some(receipt.output.as_str())
                    && turn.usage == receipt.usage
                    && session.scope.provider == receipt.provider
                    && session.native_session_id.as_ref() == Some(&receipt.session_id),
                "completed turn output changed",
            )?;
            return Ok(());
        }
        self.commit(
            "turn_output_committed",
            |next| {
                let turn = next
                    .turns
                    .get_mut(turn_id)
                    .ok_or_else(|| crate::Error::Invalid("unknown durable turn".into()))?;
                let session = next
                    .sessions
                    .get_mut(&turn.session_id)
                    .expect("bound session");
                require(
                    turn.state == TurnState::Sent
                        && turn.provider_completed
                        && session.native_observed
                        && session.native_session_id.as_ref() == Some(&receipt.session_id)
                        && session.scope.provider == receipt.provider,
                    "receipt is not bound to the observed provider session/turn",
                )?;
                turn.output_ref = Some(reference);
                turn.output = Some(receipt.output.clone());
                turn.usage = receipt.usage.clone();
                turn.state = TurnState::Completed;
                session.state = TurnState::Completed;
                Ok(())
            },
            persist,
        )
    }
    pub fn fail(
        &mut self,
        turn_id: &str,
        kind: FailureKind,
        stopped: bool,
        persist: &mut Persist<'_>,
    ) -> Result<()> {
        self.commit(
            "turn_failed",
            |next| {
                let turn = next
                    .turns
                    .get_mut(turn_id)
                    .ok_or_else(|| crate::Error::Invalid("unknown durable turn".into()))?;
                require(
                    turn.state != TurnState::Completed,
                    "cannot fail a committed completed turn",
                )?;
                turn.failure = Some(kind);
                turn.state = if stopped {
                    TurnState::Failed
                } else {
                    TurnState::DeliveryUnknown
                };
                next.sessions
                    .get_mut(&turn.session_id)
                    .expect("bound session")
                    .state = turn.state;
                Ok(())
            },
            persist,
        )
    }
    pub fn interrupted(&mut self, persist: &mut Persist<'_>) -> Result<()> {
        self.commit(
            "sessions_interrupted",
            |next| {
                for turn in next.turns.values_mut() {
                    if matches!(
                        turn.state,
                        TurnState::PendingUnknown | TurnState::Pending | TurnState::Sent
                    ) {
                        turn.state = TurnState::DeliveryUnknown;
                        next.sessions
                            .get_mut(&turn.session_id)
                            .expect("bound session")
                            .state = TurnState::DeliveryUnknown;
                    }
                }
                Ok(())
            },
            persist,
        )
    }
}
fn bind_native(session: &mut SessionRecord, id: &str, observed: bool) -> Result<()> {
    require(
        uuid::Uuid::parse_str(id).is_ok() || session.scope.provider == "fake",
        "native session ID must be an exact provider UUID",
    )?;
    require(
        session
            .native_session_id
            .as_deref()
            .is_none_or(|old| old == id),
        "native provider session changed",
    )?;
    session.native_session_id = Some(id.into());
    session.native_observed |= observed;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum RecoveryDecision {
    NoAction,
    ReconcileUnknown { reason: String },
    ResumeExact { native_session_id: String },
    ReplacementRequired { reason: String },
    AwaitAuthorization,
}
pub fn recovery_decision(
    session: &SessionRecord,
    turn: &TurnRecord,
    current_scope: &SessionScope,
    resume_support: Support,
    effects_reconciled: bool,
    resume_authorized: bool,
) -> RecoveryDecision {
    if turn.state == TurnState::Completed {
        return RecoveryDecision::NoAction;
    }
    if session.scope != *current_scope {
        return RecoveryDecision::ReplacementRequired {
            reason: "workspace/input/configuration/policy or agent scope changed".into(),
        };
    }
    if !effects_reconciled {
        return RecoveryDecision::ReconcileUnknown {
            reason:
                "interrupted delivery/effects require reconciliation before another native turn"
                    .into(),
        };
    }
    if !session.native_observed || session.native_session_id.is_none() {
        return RecoveryDecision::ReconcileUnknown{reason:"no observed native provider ID; history lookup is not implemented and a new session must not impersonate a resume".into()};
    }
    if resume_support != Support::Supported {
        return RecoveryDecision::ReplacementRequired {
            reason: "native exact-ID resume is unsupported or unverified".into(),
        };
    }
    if !resume_authorized {
        return RecoveryDecision::AwaitAuthorization;
    }
    RecoveryDecision::ResumeExact {
        native_session_id: session.native_session_id.clone().expect("observed ID"),
    }
}
pub fn text(value: &str) -> bool {
    !value.trim().is_empty() && !value.chars().any(char::is_control)
}
pub fn digest(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
