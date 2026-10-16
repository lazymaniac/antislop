//! A conversation starts before the executable task contract exists.
use crate::{Result, config::*, domain::*, require};
use std::collections::{BTreeMap, BTreeSet};

pub fn spec(objective: &str, workspace: &str, config: &PinnedConfig) -> Result<RunSpec> {
    require(!objective.trim().is_empty(), "task prompt is empty")?;
    let profile = config
        .config
        .resolve(WorkKind::Orchestration, RoleKind::Orchestrator)?;
    config
        .config
        .resolve(WorkKind::Orchestration, RoleKind::Verifier)?;
    let provider = serde_json::to_value(profile.provider)?
        .as_str()
        .unwrap()
        .to_owned();
    Ok(RunSpec {
        task_family: crate::quality::TaskFamily::Analysis,
        team_template: None,
        config: Some(config.clone()),
        objective: objective.to_owned(), workspace: workspace.to_owned(), provider,
        model: profile.model,
        requirements: vec![Requirement {
            id: "INTAKE".into(), original_text: objective.to_owned(), source_ref: "user:chat".into(),
            criteria: BTreeMap::from([("interview".into(), "Clarify the user's objective, boundaries, inputs, constraints and verifiable acceptance criteria through conversation. Propose agent roles and their configurations after the interview; do not execute the task during intake.".into())]),
        }],
        plan_items: vec![PlanItem {
            id: "INTERVIEW".into(), dependencies: BTreeSet::new(), requirement_ids: BTreeSet::from(["INTAKE".into()]),
            description: "Interview the user before proposing an executable task contract and agent configuration.".into(),
        }], input_files: vec![], editable_files: vec![], check_command: vec![],
    })
}

/// Product bootstrap supplies only the conversational orchestrator and its
/// independent reviewer. It is not a user-approved execution team.
pub fn bootstrap(binary: std::path::PathBuf) -> Result<PinnedConfig> {
    bootstrap_for(ProviderKind::Codex, binary)
}

/// Explicit initial CLI choice for a new conversation without a project config.
/// This does not choose task agents or replace a configured provider on failure.
pub fn bootstrap_for(kind: ProviderKind, binary: std::path::PathBuf) -> Result<PinnedConfig> {
    let kinds = BTreeSet::from([
        WorkKind::Orchestration,
        WorkKind::Context,
        WorkKind::Requirements,
        WorkKind::Options,
        WorkKind::Planning,
        WorkKind::Architecture,
        WorkKind::Dependencies,
        WorkKind::Implementation,
        WorkKind::Refactoring,
        WorkKind::StaticAnalysis,
        WorkKind::TestDesign,
        WorkKind::Testing,
        WorkKind::Security,
        WorkKind::Performance,
        WorkKind::Integration,
        WorkKind::Research,
        WorkKind::Writing,
        WorkKind::Analysis,
        WorkKind::Delivery,
    ]);
    Config {
        schema_version: 1,
        workspace: WorkspaceConfig::default(),
        providers: BTreeMap::from([(
            kind,
            crate::providers::discovery::default_configuration(kind, binary),
        )]),
        role_providers: BTreeMap::new(),
        profiles: BTreeMap::from([
            (
                "orchestrator".into(),
                Profile {
                    provider: kind,
                    model: None,
                    effort: Effort::Deep,
                    work_kinds: BTreeSet::from([WorkKind::Orchestration]),
                    roles: BTreeSet::from([RoleKind::Orchestrator]),
                },
            ),
            (
                "executor".into(),
                Profile {
                    provider: kind,
                    model: None,
                    effort: Effort::Deep,
                    work_kinds: kinds.clone(),
                    roles: BTreeSet::from([RoleKind::Executor]),
                },
            ),
            (
                "reviewer".into(),
                Profile {
                    provider: kind,
                    model: None,
                    effort: Effort::Deep,
                    work_kinds: kinds.clone(),
                    roles: BTreeSet::from([RoleKind::Verifier]),
                },
            ),
        ]),
        orchestrator: OrchestratorConfig {
            review_checks_per_part: crate::config::default_review_checks(),
            profile: "orchestrator".into(),
        },
        routing: kinds
            .into_iter()
            .map(|kind| {
                (
                    kind,
                    RoleRouting {
                        executor: if kind == WorkKind::Orchestration {
                            "orchestrator"
                        } else {
                            "executor"
                        }
                        .into(),
                        verifier: "reviewer".into(),
                    },
                )
            })
            .collect(),
        policy: Policy {
            mode: Mode::Supervised,
            revision: 1,
            local_write: false,
            local_checks: false,
            local_commits: false,
        },
    }
    .pin()
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TaskDefinition {
    pub task_family: crate::quality::TaskFamily,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_template: Option<crate::team::TemplateKind>,
    pub objective: String,
    pub requirements: Vec<Requirement>,
    pub plan_items: Vec<PlanItem>,
    pub input_files: Vec<String>,
    pub editable_files: Vec<String>,
    pub check_command: Vec<String>,
}
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub task: TaskDefinition,
    pub configuration: Config,
    pub rationale: String,
}

pub fn proposed_spec(run: &Run, raw: &str) -> Result<RunSpec> {
    require(
        run.intake,
        "task-contract proposals require an active interview",
    )?;
    let proposal: Proposal = crate::transport::decode(raw.as_bytes())?;
    require(
        !proposal.rationale.trim().is_empty(),
        "task proposal needs a rationale",
    )?;
    let original = &run
        .spec
        .config
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("intake configuration missing".into()))?
        .config;
    let config = &proposal.configuration;
    require(
        config.orchestrator.review_checks_per_part == original.orchestrator.review_checks_per_part,
        "task proposal cannot change runtime review pagination",
    )?;
    require(
        config.role_providers == original.role_providers,
        "task proposal cannot change user-configured role provider limits",
    )?;
    let discovered = run
        .orchestration_rounds
        .last()
        .map(crate::providers::discovery::allowed)
        .transpose()?
        .unwrap_or_default();
    require(
        config.providers.iter().all(|(kind, provider)| {
            original
                .providers
                .get(kind)
                .or_else(|| discovered.get(kind))
                == Some(provider)
        }),
        "a proposal cannot invent provider binaries, homes, isolation or capabilities",
    )?;
    require(
        config.policy.mode == original.policy.mode
            && Some(config.policy.revision) == original.policy.revision.checked_add(1),
        "task proposal cannot change autonomy mode and must advance the policy revision",
    )?;
    require(
        proposal
            .task
            .requirements
            .iter()
            .any(|r| r.original_text == run.spec.objective && r.source_ref == "user:chat"),
        "task proposal must retain the exact original chat prompt",
    )?;
    require(
        proposal.task.requirements.iter().all(|r| r.id != "INTAKE")
            && proposal.task.plan_items.iter().all(|p| p.id != "INTERVIEW"),
        "provisional interview checks cannot become the executable task contract",
    )?;
    config.resolve(WorkKind::Orchestration, RoleKind::Orchestrator)?;
    config.resolve(WorkKind::Orchestration, RoleKind::Verifier)?;
    let profile = config.resolve(WorkKind::Context, RoleKind::Executor)?;
    let spec = RunSpec {
        task_family: proposal.task.task_family,
        team_template: proposal.task.team_template,
        objective: proposal.task.objective,
        workspace: run.spec.workspace.clone(),
        provider: serde_json::to_value(profile.provider)?
            .as_str()
            .unwrap()
            .into(),
        model: profile.model,
        config: Some(config.pin()?),
        requirements: proposal.task.requirements,
        plan_items: proposal.task.plan_items,
        input_files: proposal
            .task
            .input_files
            .iter()
            .map(|path| {
                crate::workspace::inputs::relative(std::path::Path::new(&run.spec.workspace), path)
            })
            .collect::<Result<_>>()?,
        editable_files: proposal.task.editable_files,
        check_command: proposal.task.check_command,
    };
    spec.validate()?;
    // Resolve proposed source scopes before they can become native review work.
    // Invalid/unreadable proposals are rejected as proposals, not uncertain effects.
    crate::workspace::validate_inputs(std::path::Path::new(&spec.workspace), &spec.input_files)?;
    if let Some(round) = run.orchestration_rounds.last() {
        let context: serde_json::Value =
            crate::transport::decode(round.request_context.as_bytes())?;
        if let Some(value) = context.get("project_instructions") {
            let index: crate::workspace::instructions::Index =
                serde_json::from_value(value.clone())?;
            // Initial preparation has an empty provisional index. Only the
            // source-bound interview observation can require additional pins.
            if let Some(observed) = context["interview_source_hash"].as_str() {
                require(
                    index.observation_hash == observed,
                    "project instruction index differs from the interview observation",
                )?;
                for document in &index.documents {
                    let applies = document.directory_scope == "."
                        || spec.input_files.iter().any(|input| {
                            std::path::Path::new(input).starts_with(&document.directory_scope)
                        });
                    require(
                        !applies
                            || spec.input_files.iter().any(|input| {
                                input == "."
                                    || std::path::Path::new(&document.path).starts_with(input)
                            }),
                        &format!(
                            "task contract omits applicable observed project instruction {}; include its file or an existing containing directory without expanding directory inputs",
                            document.path
                        ),
                    )?;
                }
            }
        }
    }
    if let Some(catalogue) = run
        .orchestration_rounds
        .last()
        .and_then(|r| r.provider_catalogue.as_ref())
    {
        let team = crate::team::TeamPlan::bootstrap(spec.task_family, &spec)?;
        for agent in &team.agents {
            let profile = config.resolve(agent.role.work_kind(), agent.kind)?;
            require(
                catalogue
                    .entries
                    .get(&profile.provider)
                    .is_some_and(|entry| {
                        matches!(entry.state, crate::providers::discovery::State::Compatible)
                            && entry.configuration == config.providers[&profile.provider]
                    }),
                "selected task profile has no compatible observed CLI; propose a supported choice explicitly",
            )?;
        }
    }
    Ok(spec)
}

pub fn scope(run: &Run, round: &crate::orchestration::planning::Round) -> Result<String> {
    round.validate_proposed_observation()?;
    if let Some(observation) = &round.proposed_input_observation {
        return Ok(crate::hash(serde_json::to_vec(&(
            "accept_intake_v2",
            crate::orchestration::planning::accept_scope(run, round)?,
            &round.proposed_sources,
            observation,
        ))?));
    }
    Ok(crate::hash(serde_json::to_vec(&(
        "accept_intake_v1",
        crate::orchestration::planning::accept_scope(run, round)?,
        &round.proposed_sources,
    ))?))
}

pub fn pending(run: &Run) -> Option<(String, String)> {
    let round = run.orchestration_rounds.last()?;
    if !run.intake
        || run.reconciliation_required
        || !matches!(
            run.status,
            Status::AwaitingStart
                | Status::Ready
                | Status::AwaitingResult
                | Status::ChangesRequested
                | Status::Blocked
        )
        || round.status != crate::orchestration::planning::RoundStatus::Reviewed
        || crate::orchestration::planning::new_context_after_round(run, round)
    {
        return None;
    }
    let response: crate::orchestration::planning::Response =
        crate::transport::decode(round.output.as_ref()?.as_bytes()).ok()?;
    let spec = proposed_spec(run, response.intake_proposal_json.as_deref()?).ok()?;
    let capture = crate::workspace::observation::capture(
        std::path::Path::new(&spec.workspace),
        &spec.input_files,
    )
    .ok()?;
    if capture.snapshot != round.proposed_sources
        || round
            .proposed_input_observation
            .as_ref()
            .is_some_and(|o| o != &capture.observation)
    {
        return None;
    }
    Some((round.id.clone(), scope(run, round).ok()?))
}

pub fn accept(run: &mut Run, round_id: &str, expected_scope: &str) -> Result<()> {
    let (current_id, current_scope) = pending(run)
        .ok_or_else(|| crate::Error::Invalid("no current reviewed intake proposal".into()))?;
    require(
        current_id == round_id && current_scope == expected_scope,
        "intake approval is stale",
    )?;
    let index = run.orchestration_rounds.len() - 1;
    let round = &run.orchestration_rounds[index];
    let response = crate::orchestration::planning::authenticate(run, round)?;
    let spec = proposed_spec(
        run,
        response
            .intake_proposal_json
            .as_deref()
            .ok_or_else(|| crate::Error::Invalid("missing task contract".into()))?,
    )?;
    let capture = crate::workspace::observation::capture(
        std::path::Path::new(&spec.workspace),
        &spec.input_files,
    )?;
    require(
        capture.snapshot == round.proposed_sources
            && round
                .proposed_input_observation
                .as_ref()
                .is_none_or(|o| o == &capture.observation),
        "proposed task inputs changed after review",
    )?;
    run.input_snapshot = round.proposed_sources.clone();
    run.input_observation = round.proposed_input_observation.clone();
    run.diff_baseline = Some(run.input_snapshot.clone());
    run.diff_baseline_observation = run.input_observation.clone();
    run.approvals.push(Approval {
        action: format!("intake:{round_id}"),
        scope_hash: expected_scope.into(),
        spec_revision: run.spec_revision,
        actor: "local_user".into(),
    });
    run.spec = spec;
    run.spec_revision = run
        .spec_revision
        .checked_add(1)
        .ok_or_else(|| crate::Error::Invalid("specification revision exhausted".into()))?;
    run.intake = false;
    run.chat_origin = true;
    run.stage = Stage::Discovery;
    run.status = Status::AwaitingStart;
    run.orchestration_rounds[index].status = crate::orchestration::planning::RoundStatus::Accepted;
    run.diagnostic = Some("Interview contract and agent routing accepted; no task execution or quality gate approved.".into());
    crate::verification::refresh_provenance(run);
    Ok(())
}

/// Repair the legacy file-only reader's false effect uncertainty, retaining the
/// authored answer and requiring a separate approval for its missing review.
pub(crate) fn settle_legacy_directory_capture(run: &mut Run) -> Result<bool> {
    use crate::{
        config::IsolationProfile,
        orchestration::planning::{self, RoundStatus},
    };
    if !run.intake
        || run.status != Status::Blocked
        || !run.reconciliation_required
        || run.provider_failure.is_some()
        || run.team.is_some()
        || !run.artifacts.is_empty()
        || !run.pending_edits.is_empty()
    {
        return Ok(false);
    }
    let Some(round) = run.orchestration_rounds.last() else {
        return Ok(false);
    };
    if round.status != RoundStatus::Failed
        || round.diagnostic.as_deref() != Some("input path must be a normal relative path")
        || round.intake_evidence_id.is_some()
        || !round.proposed_sources.is_empty()
        || run.session_ledger.sessions.values().any(|s| {
            s.scope.task_id.as_ref() == Some(&round.id) && s.scope.role == RoleKind::Verifier
        })
    {
        return Ok(false);
    }
    let profile = run
        .spec
        .config
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("missing intake configuration".into()))?
        .config
        .resolve(WorkKind::Orchestration, RoleKind::Orchestrator)?;
    require(
        matches!(
            profile.isolation,
            IsolationProfile::ReadOnly | IsolationProfile::ToolFree
        ),
        "intake source capture had unrestricted native access",
    )?;
    planning::authenticate_unreviewed_proposal(run, round)?;
    let new_context = planning::new_context_after_round(run, round);
    run.reconciliation_required = false;
    run.status = Status::AwaitingStart;
    run.diagnostic=Some(if new_context {
        "The directory scope is now supported. Your queued update requires a fresh orchestrator reply; the earlier response remains in history. Approve the queued conversation turn. No project work or contract is approved."
    } else {
        "The directory scope is now supported. Completed orchestrator output is retained. Approve read-only management recovery to start its missing independent review; no contract or project work is approved."
    }.into());
    Ok(true)
}
