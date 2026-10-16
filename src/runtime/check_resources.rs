//! Durable custody of check containers. Fences prove checkpoint ordering only;
//! policy, effective sandbox and exact input admission remain separate duties.
//! Unknown creation/start is never a request to replay the operation.
use super::team::{check_recovery::intent_hash, checks::CheckState};
use crate::process::container::{ContainerResource, CreationObservation, RemovalProof};
use crate::{Result, domain::Run, hash, id, require};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

type Persist<'a> = dyn FnMut(&Run, &str) -> Result<()> + 'a;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Reserved,
    CreateUnknown,
    Created,
    StartUnknown,
    CleanupUnknown,
    Removed,
    AbortedBeforeCreate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    phase: Phase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    container_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    id: String,
    run_id: String,
    invocation_id: String,
    intent_hash: String,
    binary: PathBuf,
    image_id: String,
    owner_token: String,
    name: String,
    steps: Vec<Step>,
}

impl Record {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn invocation_id(&self) -> &str {
        &self.invocation_id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn owner_token(&self) -> &str {
        &self.owner_token
    }
    pub fn binary(&self) -> &std::path::Path {
        &self.binary
    }
    pub fn image_id(&self) -> &str {
        &self.image_id
    }
    pub fn phase(&self) -> Option<Phase> {
        self.steps.last().map(|step| step.phase)
    }
    pub fn resource(&self) -> Option<ContainerResource> {
        self.steps
            .last()?
            .container_id
            .as_ref()
            .map(|container_id| ContainerResource {
                binary: self.binary.clone(),
                container_id: container_id.clone(),
                image_id: self.image_id.clone(),
                owner_token: self.owner_token.clone(),
            })
    }
    pub fn unresolved(&self) -> bool {
        !matches!(
            self.phase(),
            Some(Phase::Removed | Phase::AbortedBeforeCreate)
        )
    }
    fn validate(&self, run_id: &str) -> Result<()> {
        require(
            self.run_id == run_id
                && uuid::Uuid::parse_str(&self.id).is_ok()
                && !self.invocation_id.is_empty()
                && self.intent_hash.len() == 64
                && self.intent_hash.bytes().all(|b| b.is_ascii_hexdigit())
                && self.name == format!("antislop-check-{}", self.id),
            "invalid container custody identity/scope",
        )?;
        // Validate the pre-ID engine/token binding without issuing an effect.
        ContainerResource {
            binary: self.binary.clone(),
            container_id: "0".repeat(64),
            image_id: self.image_id.clone(),
            owner_token: self.owner_token.clone(),
        }
        .validate()?;
        let mut previous = None;
        let mut known_id = None;
        for step in &self.steps {
            require(
                match previous {
                    None => step.phase == Phase::Reserved,
                    Some(from) => legal(from, step.phase),
                },
                "invalid or replayed container custody transition",
            )?;
            if step.phase == Phase::Created {
                require(
                    step.container_id.is_some() && known_id.is_none(),
                    "container creation binding missing or replaced",
                )?;
                known_id = step.container_id.clone();
            }
            require(
                step.container_id == known_id,
                "container custody ID changed or appeared before creation",
            )?;
            if let Some(container_id) = &step.container_id {
                ContainerResource {
                    binary: self.binary.clone(),
                    container_id: container_id.clone(),
                    image_id: self.image_id.clone(),
                    owner_token: self.owner_token.clone(),
                }
                .validate()?;
            }
            previous = Some(step.phase);
        }
        require(previous.is_some(), "empty container custody history")
    }
}

fn legal(from: Phase, to: Phase) -> bool {
    matches!(
        (from, to),
        (
            Phase::Reserved,
            Phase::CreateUnknown | Phase::AbortedBeforeCreate
        ) | (Phase::CreateUnknown, Phase::Created)
            | (Phase::Created, Phase::StartUnknown | Phase::CleanupUnknown)
            | (Phase::StartUnknown, Phase::CleanupUnknown)
            | (Phase::CleanupUnknown, Phase::Removed)
    )
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    records: BTreeMap<String, Record>,
}
impl Ledger {
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }
    pub fn records(&self) -> impl Iterator<Item = &Record> {
        self.records.values()
    }
    pub fn get(&self, id: &str) -> Option<&Record> {
        self.records.get(id)
    }
    pub fn validate_for_run(&self, run_id: &str) -> Result<()> {
        let mut names = std::collections::BTreeSet::new();
        let mut tokens = std::collections::BTreeSet::new();
        let mut containers = std::collections::BTreeSet::new();
        for (key, record) in &self.records {
            record.validate(run_id)?;
            require(
                key == record.id()
                    && names.insert(record.name())
                    && tokens.insert(record.owner_token()),
                "duplicate container custody identity",
            )?;
            if let Some(resource) = record.resource() {
                require(
                    containers.insert((record.binary.clone(), resource.container_id)),
                    "container ID claimed by multiple checks",
                )?;
            }
        }
        Ok(())
    }
}

/// Non-cloneable, non-deserializable evidence that the create-intent checkpoint
/// was acknowledged. It is not execution/sandbox/policy authorization.
pub struct CreateFence {
    record: Record,
}
impl CreateFence {
    pub fn record(&self) -> &Record {
        &self.record
    }
}
/// Same checkpoint ordering protection immediately before a remote start.
/// The backend must additionally authenticate its policy, inputs and sandbox.
pub struct StartFence {
    resource: ContainerResource,
}
impl StartFence {
    pub fn into_resource(self) -> ContainerResource {
        self.resource
    }
}

fn current_intent(run: &Run, invocation_id: &str) -> Result<String> {
    require(
        run.status == crate::domain::Status::Running && !run.reconciliation_required,
        "container creation/start requires an active reconciled check continuation",
    )?;
    let matches = run
        .task_checks
        .iter()
        .filter(|i| i.id == invocation_id)
        .collect::<Vec<_>>();
    require(
        matches.len() == 1,
        "container custody requires one persisted task-check intent",
    )?;
    let intent = matches[0];
    require(
        intent.state == CheckState::Intent
            && intent.spec_revision == run.spec_revision
            && run.mode_configuration_matches(&intent.configuration_hash)
            && run.approvals.iter().any(|a| {
                a.actor == "local_user"
                    && a.spec_revision == intent.spec_revision
                    && a.scope_hash == intent.authorization_scope
                    && matches!(
                        a.action.as_str(),
                        "step_team"
                            | "refresh_task_sources"
                            | "retry_task_verification"
                            | "resume_task_session"
                            | "replace_task_session"
                    )
            }),
        "container custody has no current check-intent/approval binding",
    )?;
    intent_hash(intent)
}

fn commit(run: &mut Run, next: Ledger, event: &str, persist: &mut Persist<'_>) -> Result<()> {
    next.validate_for_run(&run.id)?;
    let mut candidate = run.clone();
    candidate.check_resources = next;
    persist(&candidate, event)?;
    *run = candidate;
    Ok(())
}

pub fn reserve(
    run: &mut Run,
    invocation_id: &str,
    binary: PathBuf,
    image_id: String,
    persist: &mut Persist<'_>,
) -> Result<String> {
    run.check_resources.validate_for_run(&run.id)?;
    let binding = current_intent(run, invocation_id)?;
    require(
        !run.check_resources
            .records()
            .any(|r| r.invocation_id() == invocation_id),
        "task-check container intent already exists; do not replay it",
    )?;
    let record_id = id();
    let record = Record {
        id: record_id.clone(),
        run_id: run.id.clone(),
        invocation_id: invocation_id.into(),
        intent_hash: binding,
        binary,
        image_id,
        owner_token: id(),
        name: format!("antislop-check-{record_id}"),
        steps: vec![Step {
            phase: Phase::Reserved,
            container_id: None,
        }],
    };
    let mut next = run.check_resources.clone();
    next.records.insert(record_id.clone(), record);
    commit(run, next, "check_container_reserved", persist)?;
    Ok(record_id)
}

fn transition(
    run: &mut Run,
    id: &str,
    phase: Phase,
    container_id: Option<String>,
    persist: &mut Persist<'_>,
) -> Result<()> {
    run.check_resources.validate_for_run(&run.id)?;
    let mut next = run.check_resources.clone();
    let record = next
        .records
        .get_mut(id)
        .ok_or_else(|| crate::Error::Invalid("unknown check container custody".into()))?;
    require(
        record.phase().is_some_and(|from| legal(from, phase)),
        "container operation cannot be replayed or reordered",
    )?;
    record.steps.push(Step {
        phase,
        container_id,
    });
    commit(run, next, "check_container_checkpoint", persist)
}

pub fn begin_create(run: &mut Run, id: &str, persist: &mut Persist<'_>) -> Result<CreateFence> {
    let record = run
        .check_resources
        .get(id)
        .ok_or_else(|| crate::Error::Invalid("unknown check container custody".into()))?;
    require(
        current_intent(run, record.invocation_id())? == record.intent_hash,
        "task-check intent changed before container creation",
    )?;
    transition(run, id, Phase::CreateUnknown, None, persist)?;
    Ok(CreateFence {
        record: run.check_resources.get(id).unwrap().clone(),
    })
}

/// Record a returned or freshly observed identity. This cannot authorize start.
pub fn record_created(
    run: &mut Run,
    id: &str,
    resource: &ContainerResource,
    persist: &mut Persist<'_>,
) -> Result<()> {
    resource.validate()?;
    let record = run
        .check_resources
        .get(id)
        .ok_or_else(|| crate::Error::Invalid("unknown check container custody".into()))?;
    require(
        record.binary == resource.binary
            && record.image_id == resource.image_id
            && record.owner_token == resource.owner_token,
        "created container differs from its durable custody binding",
    )?;
    transition(
        run,
        id,
        Phase::Created,
        Some(resource.container_id.clone()),
        persist,
    )
}

/// Recover a lost create response without creating, starting or removing
/// anything. A missing name remains unresolved because create may be in flight.
pub fn observe_creation(record: &Record) -> Result<Option<CreationObservation>> {
    record.validate(&record.run_id)?;
    require(
        record.phase() == Some(Phase::CreateUnknown),
        "creation observation requires an unsettled create checkpoint",
    )?;
    crate::process::container::observe_creation(
        record.binary.clone(),
        record.name(),
        record.image_id(),
        record.owner_token(),
    )
}

pub fn record_observed_creation(
    run: &mut Run,
    id: &str,
    observation: CreationObservation,
    persist: &mut Persist<'_>,
) -> Result<()> {
    let record = run
        .check_resources
        .get(id)
        .ok_or_else(|| crate::Error::Invalid("unknown check container custody".into()))?;
    require(
        record.name() == observation.name(),
        "observed creation belongs to another reserved container",
    )?;
    record_created(run, id, observation.resource(), persist)
}

pub fn begin_start(run: &mut Run, id: &str, persist: &mut Persist<'_>) -> Result<StartFence> {
    let record = run
        .check_resources
        .get(id)
        .ok_or_else(|| crate::Error::Invalid("unknown check container custody".into()))?;
    require(
        current_intent(run, record.invocation_id())? == record.intent_hash,
        "task-check intent changed before container start",
    )?;
    let resource = record
        .resource()
        .ok_or_else(|| crate::Error::Invalid("container start has no persisted exact ID".into()))?;
    transition(
        run,
        id,
        Phase::StartUnknown,
        Some(resource.container_id.clone()),
        persist,
    )?;
    Ok(StartFence { resource })
}

pub fn begin_cleanup(
    run: &mut Run,
    id: &str,
    persist: &mut Persist<'_>,
) -> Result<ContainerResource> {
    run.check_resources.validate_for_run(&run.id)?;
    let record = run
        .check_resources
        .get(id)
        .ok_or_else(|| crate::Error::Invalid("unknown check container custody".into()))?;
    let resource = record.resource().ok_or_else(|| {
        crate::Error::Invalid(
            "unknown creation requires exact-name observation, not assumed absence".into(),
        )
    })?;
    if record.phase() != Some(Phase::CleanupUnknown) {
        transition(
            run,
            id,
            Phase::CleanupUnknown,
            Some(resource.container_id.clone()),
            persist,
        )?;
    }
    Ok(resource)
}

pub fn record_removed(
    run: &mut Run,
    id: &str,
    proof: RemovalProof,
    persist: &mut Persist<'_>,
) -> Result<()> {
    let record = run
        .check_resources
        .get(id)
        .ok_or_else(|| crate::Error::Invalid("unknown check container custody".into()))?;
    require(
        record.resource().as_ref() == Some(proof.resource()),
        "cleanup proof belongs to another container binding",
    )?;
    transition(
        run,
        id,
        Phase::Removed,
        Some(proof.resource().container_id.clone()),
        persist,
    )
}

pub fn abort_before_create(run: &mut Run, id: &str, persist: &mut Persist<'_>) -> Result<()> {
    transition(run, id, Phase::AbortedBeforeCreate, None, persist)
}

pub fn scope_hash(record: &Record) -> Result<String> {
    Ok(hash(serde_json::to_vec(record)?))
}
