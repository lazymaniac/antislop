use super::{ObservedWork, Runtime};
use crate::{
    Result,
    config::{RoleKind, WorkKind},
    domain::*,
    hash, id,
    orchestration::planning::{self, Response, ReviewResponse, Round, RoundStatus},
    providers, require,
};
use std::sync::{Arc, atomic::AtomicBool};

pub(super) fn prepare(run: &mut Run, scope: &str) -> Result<Status> {
    run.spec.validate()?;
    require(
        matches!(
            run.status,
            Status::AwaitingStart
                | Status::Ready
                | Status::AwaitingResult
                | Status::ChangesRequested
                | Status::Blocked
                | Status::Completed
                | Status::Cancelled
        ) && !run.reconciliation_required,
        "orchestration requires a stopped, reconciled run",
    )?;
    let pinned = run.spec.config.as_ref().ok_or_else(|| {
        crate::Error::Invalid("orchestrator requires explicit pinned configuration".into())
    })?;
    pinned
        .config
        .validate_work_kinds(&std::collections::BTreeSet::from([WorkKind::Orchestration]))?;
    require(
        scope == planning::start_scope(run)?,
        "orchestration scope is stale",
    )?;
    let message_ids = planning::queued_ids(run);
    require(
        message_ids
            .iter()
            .any(|id| run.communication_ledger.messages[id].recipient_agent_id == "orchestrator"),
        "orchestration requires a queued user message",
    )?;
    require(
        crate::workspace::read_inputs(
            std::path::Path::new(&run.spec.workspace),
            &run.spec.input_files,
        )? == run.input_snapshot,
        "workspace inputs changed; revise before orchestration",
    )?;
    let template = if run.intake {
        serde_json::Value::Null
    } else {
        serde_json::to_value(planning::template(run)?)?
    };
    let round_id = id();
    // Keep dialogue history linear. Embedding prior request contexts would
    // recursively duplicate the entire conversation at every turn.
    let history:Vec<_>=run.orchestration_rounds.iter().map(|r|serde_json::json!({
        "id":r.id,"spec_revision":r.spec_revision,"source_hashes":r.source_hashes,
        "status":r.status,"executor_session":r.executor_session,"verifier_session":r.verifier_session,
        "output":r.output,"review":r.review,"diagnostic":r.diagnostic,
        "historical_context":"Exact prior request context remains in the local round record; current sources and scope below govern this turn."})).collect();
    // Point-to-point messages belong to their recipients. Management receives
    // its inbox and its own outgoing reports, never unrelated private traffic
    // or child session transcripts. Runtime retains the complete audit ledger.
    let messages: Vec<_> = run
        .communication_ledger
        .history
        .iter()
        .filter(|message| {
            message.recipient_agent_id == "orchestrator"
                || matches!(&message.sender,
            crate::communication::Sender::Agent { agent_id } if agent_id == "orchestrator")
        })
        .map(|entry| {
            serde_json::json!({"record":run.communication_ledger.messages[&entry.message_id],
            "content":run.message_bodies.get(&entry.message_id)})
        })
        .collect();
    let context = serde_json::to_string(&serde_json::json!({"round_id":round_id,"spec":run.spec,
        "contract_version":planning::REVIEW_CONTRACT_VERSION,"checklist_version":checklist_version(),
        "historical_run_checklist_version":run.checklist_version,
        "spec_revision":run.spec_revision,"spec_hash":hash(serde_json::to_vec(&run.spec)?),
        "stage":run.stage,"status":run.status,"attempt":run.attempt,"sources":run.sources(),
        "historical_runtime_diagnostic":run.diagnostic,"historical_provider_failure":run.provider_failure,
        "quality":run.quality,"quality_gates":run.quality_gates,"artifacts":run.artifacts,"reviews":run.reviews,
        "accepted_team":run.team,"prior_rounds":history,
        "messages":messages,
        "element_comments":run.element_comments,
        "comment_contract":"Return one comment_reactions entry for every unresolved, non-deleted element comment: answer, question, or proposed_task with a real task ID. React to the exact annotated element/version. Delivery does not resolve a comment or authorize work. Completed/cancelled runs permit read-only dialogue; they cannot install or execute a new team. Explain any necessary new run/scope explicitly.",
        "diff_comments":&run.diff_comments,
        "unresolved_diff_comments":run.diff_comments.iter().filter(|c| !c.resolved).collect::<Vec<_>>(),
        "provider_catalogue":run.orchestration_rounds.iter().rev().find_map(|r|r.provider_catalogue.as_ref()),
        "intake":run.intake,
        "intake_contract":"When intake is true, conduct an interview in the user's language. Clarify objective, boundaries, inputs, constraints and acceptance criteria. Roles and their system prompts are predefined; propose the suitable roles and CLI/model/effort configuration yourself. The user is not required to prepare a JSON spec or choose every agent. Return team_plan_json null and risk_owners [] during the interview. When the interview has resolved the scope, return intake_proposal_json: JSON with task (task_family, objective, requirements, plan_items, input_files, editable_files, check_command), configuration (the unpinned Config shape) and rationale. Use new requirement/plan IDs, replacing INTAKE/INTERVIEW. Keep the exact original prompt in a requirement with source_ref user:chat, preserve all subsequent interview obligations, choose role routes/profiles/models/effort yourself from the declared providers, retain the exact user-configured role_providers limits (Codex may be restricted to orchestrator and agy to executors/verifiers), keep provider binary/home/isolation/capabilities unchanged, keep the autonomy mode and increment policy.revision by one. Runtime computes the config pin. This is a proposed contract for explicit user acceptance, not permission to start work. Do not claim that the provisional intake checklist is the user's confirmed task contract.",
        "management_contract":"Manage from task reports, artifacts, review decisions, findings, addressed messages and user diff comments. Address unresolved diff comments by scheduling Implementer or Refactorer tasks with matching write scopes. Child session transcripts are not management input. Ask the responsible agent for missing information; runtime authenticates receipts and progress.",
        "risk_catalog":serde_json::from_str::<serde_json::Value>(include_str!("../../fixtures/risks.json"))?,
        "team_template":template,"limitations":"Fixed-stage execution and proposed team execution are distinct. A proposal does not run tasks or satisfy quality gates. External/destructive effects are not authorized."}))?;
    let previous = run.status;
    run.approvals.push(Approval {
        action: format!("orchestrate:{round_id}"),
        scope_hash: scope.into(),
        spec_revision: run.spec_revision,
        actor: "local_user".into(),
    });
    run.orchestration_rounds.push(Round {
        recoveries: vec![],
        contract_version: planning::REVIEW_CONTRACT_VERSION.into(),
        checklist_version: checklist_version(),
        id: round_id,
        spec_revision: run.spec_revision,
        source_hashes: run.source_hashes(),
        input_hash: hash(serde_json::to_vec(&run.input_snapshot)?),
        approval_scope: scope.into(),
        previous_status: previous,
        previous_active_session: run.active_session.clone(),
        previous_provider_failure: run.provider_failure.clone(),
        message_ids,
        request_sequence: run.communication_ledger.history.len() as u64,
        comments_hash: crate::comments::input_hash(run),
        request_context: context,
        input_evidence_id: None,
        output_evidence_id: None,
        proposed_sources: Default::default(),
        intake_evidence_id: None,
        provider_catalogue: None,
        discovery_status: run.intake.then(|| "queued".into()),
        status: RoundStatus::Running,
        executor_session: None,
        verifier_session: None,
        output: None,
        review: None,
        diagnostic: None,
    });
    run.status = Status::Running;
    run.reconciliation_required = true;
    run.provider_failure = None;
    Ok(previous)
}
pub(super) fn accept(run: &mut Run, round_id: &str, scope: &str) -> Result<()> {
    require(
        !run.reconciliation_required
            && matches!(
                run.status,
                Status::AwaitingStart
                    | Status::Ready
                    | Status::AwaitingResult
                    | Status::ChangesRequested
                    | Status::Blocked
            ),
        "team acceptance requires a stopped reconciled run",
    )?;
    let index = run
        .orchestration_rounds
        .iter()
        .position(|r| r.id == round_id)
        .ok_or_else(|| crate::Error::Invalid("unknown orchestration round".into()))?;
    let round = &run.orchestration_rounds[index];
    require(
        !planning::new_context_after_round(run, round),
        "new messages require a fresh orchestration review before team acceptance",
    )?;
    require(
        index + 1 == run.orchestration_rounds.len(),
        "a later orchestration round superseded this proposal",
    )?;
    require(
        round.status == RoundStatus::Reviewed && scope == planning::accept_scope(run, round)?,
        "team approval is stale or not independently reviewed",
    )?;
    require(
        crate::workspace::read_inputs(
            std::path::Path::new(&run.spec.workspace),
            &run.spec.input_files,
        )? == run.input_snapshot,
        "workspace changed after orchestration review",
    )?;
    let response = planning::authenticate(run, round)?;
    let mut team = planning::validate_response(run, &response)?
        .ok_or_else(|| crate::Error::Invalid("this round contains no team proposal".into()))?;
    require(
        run.team.as_ref().is_none_or(planning::replaceable),
        "team execution history cannot be replaced by a fresh proposal",
    )?;
    run.approvals.push(Approval {
        action: format!("team:{round_id}"),
        scope_hash: scope.into(),
        spec_revision: run.spec_revision,
        actor: "local_user".into(),
    });
    for agent in &mut team.agents {
        if agent.id == "orchestrator" {
            agent.session_id = run.orchestration_rounds[index].executor_session.clone();
        }
        if agent.id == "orchestrator:verifier" {
            agent.session_id = run.orchestration_rounds[index].verifier_session.clone();
        }
    }
    team.validate(&run.spec)?;
    run.team = Some(team);
    // Fixed-stage identities cease to exist when the explicit team is
    // installed. Preserve their queued traffic as cancelled history.
    super::messaging::retire_recipients(run)?;
    run.team_risk_owners = response.risk_owners;
    run.orchestration_rounds[index].status = RoundStatus::Accepted;
    Ok(())
}
fn evidence(run: &mut Run, round: &Round, source: &str, output: &str) -> String {
    let evidence_id = id();
    run.evidence.push(Evidence {
        id: evidence_id.clone(),
        artifact_hash: hash(output),
        spec_revision: run.spec_revision,
        source: source.into(),
        output: output.into(),
        output_hash: hash(output),
        success: true,
        input_hash: Some(round.input_hash.clone()),
    });
    evidence_id
}
impl Runtime {
    pub(super) fn work_orchestration(
        self: Arc<Self>,
        mut run: Run,
        previous: Status,
        token: Arc<AtomicBool>,
        lease: crate::recovery::WorkspaceLease,
    ) {
        let _lease_scope = crate::process::LeaseScope::new(&lease.fds());
        let index = run.orchestration_rounds.len() - 1;
        let primary_session = run.orchestration_rounds[index]
            .previous_active_session
            .clone();
        let outcome = (|| -> Result<()> {
            if run.intake && run.orchestration_rounds[index].provider_catalogue.is_none() {
                run.orchestration_rounds[index].discovery_status = Some("running".into());
                self.persist_progress(&run)?;
                let search = std::env::var_os("PATH").unwrap_or_default();
                let candidates = crate::providers::discovery::candidates(&run.spec.config.as_ref().unwrap().config, &search);
                let mut entries = std::collections::BTreeMap::new();
                for (kind, configuration) in candidates {
                    self.await_action(&run)?;
                    entries.insert(kind, crate::providers::discovery::probe_entry(&run, kind, configuration,
                        token.clone(), std::time::Duration::from_secs(10), &search)?);
                }
                let catalogue = crate::providers::discovery::Catalogue { entries };
                let mut context: serde_json::Value = crate::transport::decode(run.orchestration_rounds[index].request_context.as_bytes())?;
                context["provider_catalogue"] = serde_json::to_value(&catalogue)?;
                context["provider_catalogue_contract"] = serde_json::json!("Compatibility reports are runtime-owned public CLI probes, not authentication or model-quality acceptance. Use only compatible exact configurations for new provider choices; retain explicitly declared binaries. Do not invent a model catalogue or claim an unverified login. Missing, rejected and timed-out CLIs do not become routes. The initial orchestrator is not silently replaced.");
                run.orchestration_rounds[index].request_context = serde_json::to_string(&context)?;
                run.orchestration_rounds[index].provider_catalogue = Some(catalogue);
                run.orchestration_rounds[index].discovery_status = Some("complete".into());
                self.persist_progress(&run)?;
            }
            let round = run.orchestration_rounds[index].clone();
            if let Some(input_id)=&round.input_evidence_id {
                require(run.evidence.iter().any(|e|&e.id==input_id&&e.source=="runtime:orchestrator-inputs"
                    &&e.output==round.request_context&&e.output_hash==hash(&e.output)&&e.spec_revision==run.spec_revision),
                    "retained management input receipt changed")?;
            }else{
                let input_id=evidence(&mut run,&round,"runtime:orchestrator-inputs",&round.request_context);
                run.orchestration_rounds[index].input_evidence_id=Some(input_id);
                self.persist_progress(&run)?;
            }
            let executor =
                providers::build_for_work(&run, WorkKind::Orchestration, RoleKind::Orchestrator)?;
            let output=if let Some(output)=run.orchestration_rounds[index].output.clone() {
                output
            }else{
                let request = planning::executor_request(&round);
            let view = run.clone();
            let (session, receipt) = self.observed_turn(
                &mut run,
                &view,
                executor.as_ref(),
                ObservedWork::Prepared {
                    role: RoleKind::Orchestrator,
                    agent_id: "orchestrator",
                    task_id: &round.id,
                    message_ids: &round.message_ids,
                    request: &request,
                },
                token.clone(),
            )?;
            run.orchestration_rounds[index].executor_session = Some(session);
            run.orchestration_rounds[index].output = Some(receipt.output.clone());
            let output_id = evidence(
                &mut run,
                &round,
                "runtime:orchestrator-output",
                &receipt.output,
            );
            run.orchestration_rounds[index].output_evidence_id = Some(output_id);
            self.persist_progress(&run)?;
                receipt.output
            };
            if run.orchestration_rounds[index].output_evidence_id.is_none() {
                let output_id=evidence(&mut run,&round,"runtime:orchestrator-output",&output);
                run.orchestration_rounds[index].output_evidence_id=Some(output_id);
                self.persist_progress(&run)?;
            }
            let validated = crate::transport::decode::<Response>(output.as_bytes())
                .and_then(|response| {
                    planning::validate_response(&run, &response).map(|_| response)
                });
            let response = match validated {
                Ok(response) => response,
                Err(error) => {
                    run.orchestration_rounds[index].status = RoundStatus::Rejected;
                    run.orchestration_rounds[index].diagnostic = Some(error.to_string());
                    run.status = previous;
                    return Ok(());
                }
            };
            if planning::interview_reply(&run, &response) {
                // Keep the interview available for the user's next answer.
                // This authenticates the actual orchestrator turn, but confers
                // no independent review or approval on a future contract.
                planning::authenticate_interview_reply(&run, &run.orchestration_rounds[index])?;
                run.orchestration_rounds[index].status = RoundStatus::AwaitingUser;
                run.status = previous;
                return Ok(());
            }
            if let Some(raw) = &response.intake_proposal_json
                &&run.orchestration_rounds[index].intake_evidence_id.is_none() {
                let spec = super::intake::proposed_spec(&run, raw)?;
                let sources = crate::workspace::read_inputs(std::path::Path::new(&spec.workspace), &spec.input_files)?;
                let output = serde_json::to_string(&sources)?;
                let evidence_id = evidence(&mut run, &round, "runtime:intake-sources", &output);
                run.orchestration_rounds[index].proposed_sources = sources;
                run.orchestration_rounds[index].intake_evidence_id = Some(evidence_id);
                self.persist_progress(&run)?;
            }
            let round = run.orchestration_rounds[index].clone();
            if run.orchestration_rounds[index].review.is_none() {
            let verifier = providers::build_for_work(&run, WorkKind::Orchestration, RoleKind::Verifier)?;
            let request = planning::verifier_request(&run, &round, &response)?;
            let view = run.clone();
            let (session, receipt) = self.observed_turn(
                &mut run,
                &view,
                verifier.as_ref(),
                ObservedWork::Prepared {
                    role: RoleKind::Verifier,
                    agent_id: "orchestrator:verifier",
                    task_id: &round.id,
                    message_ids: &round.message_ids,
                    request: &request,
                },
                token,
            )?;
            run.orchestration_rounds[index].verifier_session = Some(session);
            run.orchestration_rounds[index].review = Some(receipt.output);
            }
            let round = run.orchestration_rounds[index].clone();
            let accepted = crate::transport::decode::<ReviewResponse>(
                round.review.as_ref().unwrap().as_bytes(),
            )
            .and_then(|review| planning::validate_review(&run, &round, &response, &review))
            .and_then(|_| planning::authenticate(&run, &round).map(|_| ()));
            run.orchestration_rounds[index].status = if accepted.is_ok() {
                RoundStatus::Reviewed
            } else {
                RoundStatus::Rejected
            };
            run.orchestration_rounds[index].diagnostic = accepted.err().map(|e| e.to_string());
            if run.orchestration_rounds[index].status == RoundStatus::Reviewed {
                let mut store = self.store.lock().expect("store mutex");
                let mut current = store.get(&run.id)?;
                require(current.spec_revision == run.spec_revision && current.attempt == run.attempt
                    && current.status != Status::Cancelled && crate::comments::input_hash(&current) == round.comments_hash,
                    "element comments changed during review; a fresh orchestration round is required")?;
                for reaction in &response.comment_reactions {
                    let comment = current.element_comments.iter_mut().find(|c| c.id == reaction.comment_id).unwrap();
                    if let Some(existing)=comment.reactions.iter().find(|r|r.round_id==round.id) {
                        require(serde_json::to_value(&existing.response)?==serde_json::to_value(reaction)?,"retained comment reaction changed")?;
                    }else{comment.reactions.push(crate::comments::Reaction {round_id:round.id.clone(), response:reaction.clone()});}
                }
                current.orchestration_rounds = run.orchestration_rounds.clone();
                store.save(&current, "comment_reactions_reviewed", None)?;
                run.element_comments = current.element_comments;
            }
            require(
                crate::workspace::read_inputs(
                    std::path::Path::new(&run.spec.workspace),
                    &run.spec.input_files,
                )? == run.input_snapshot,
                "workspace changed during orchestration; reconcile before continuing",
            )?;
            run.status = previous;
            Ok(())
        })()
        .and_then(|()| {
            require(
                crate::workspace::read_inputs(
                    std::path::Path::new(&run.spec.workspace),
                    &run.spec.input_files,
                )? == run.input_snapshot,
                "workspace changed during orchestration; reconcile before continuing",
            )
        });
        if let Err(error) = &outcome {
            run.orchestration_rounds[index].status = RoundStatus::Failed;
            run.orchestration_rounds[index].diagnostic = Some(error.to_string());
        } else {
            run.provider_failure = run.orchestration_rounds[index]
                .previous_provider_failure
                .clone();
        }
        // Management turns belong to the installed agents too. Retain their
        // previous task sessions as history while showing this round's actual
        // observed session, including failed native turns.
        if let Some(team) = &mut run.team {
            let round_id = &run.orchestration_rounds[index].id;
            for session in run.session_ledger.sessions.values().filter(|session| {
                session.scope.task_id.as_ref() == Some(round_id)
                    && matches!(
                        session.scope.agent_id.as_str(),
                        "orchestrator" | "orchestrator:verifier"
                    )
            }) {
                if let Some(agent) = team
                    .agents
                    .iter_mut()
                    .find(|agent| agent.id == session.scope.agent_id)
                    && agent.session_id.as_ref() != Some(&session.id)
                {
                    if let Some(previous) = agent.session_id.replace(session.id.clone()) {
                        agent.session_history.insert(previous);
                    }
                    agent.session_history.remove(&session.id);
                }
            }
        }
        drop(_lease_scope);
        self.finish_attempt_with_session(run, outcome, Some(primary_session), Some(lease));
    }
}
