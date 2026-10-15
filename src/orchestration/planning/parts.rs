//! Native review pagination. Every original check remains mandatory; each page
//! has its own independently authenticated session and immutable scope.
use super::*;
use crate::config::RoleKind;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Part {
    pub id: String,
    pub check_ids: BTreeSet<String>,
    pub session_id: Option<String>,
    pub output: Option<String>,
}
pub fn plan(run: &Run, round: &Round, response: &Response) -> Result<Vec<Part>> {
    let size = run
        .spec
        .config
        .as_ref()
        .map(|c| c.config.orchestrator.review_checks_per_part)
        .unwrap_or(0);
    if size == 0 || response.team_plan_json.is_none() {
        return Ok(vec![]);
    }
    let all = expected_checks(run, response);
    let ids: Vec<_> = all
        .iter()
        .filter(|id| !matches!(id.as_str(), "V01" | "V04"))
        .cloned()
        .collect();
    if ids.len() <= size {
        return Ok(vec![]);
    }
    ids.chunks(size)
        .enumerate()
        .map(|(index, chunk)| {
            let mut check_ids: BTreeSet<_> = chunk.iter().cloned().collect();
            check_ids.extend(["V01".into(), "V04".into()]);
            let id = hash(serde_json::to_vec(&(
                "native-review-part-v1",
                &round.id,
                round.output.as_ref().map(hash),
                &all,
                index,
                &check_ids,
            ))?);
            Ok(Part {
                id,
                check_ids,
                session_id: None,
                output: None,
            })
        })
        .collect()
}
pub fn validate_plan(run: &Run, round: &Round, response: &Response) -> Result<()> {
    let agents = round
        .review_known_agents
        .as_ref()
        .ok_or_else(|| crate::Error::Invalid("review agent catalogue was not frozen".into()))?;
    let context: Value = crate::transport::decode(round.request_context.as_bytes())?;
    let bound = if let Some(v) = context.get("known_agent_ids") {
        v.as_array().is_some()
            && serde_json::from_value::<BTreeSet<String>>(v.clone()).is_ok_and(|a| &a == agents)
    } else {
        round.recoveries.iter().any(|r| {
            r.handoff_hash == hash(serde_json::to_vec(&r.handoff).unwrap_or_default())
                && run.approvals.iter().any(|a| {
                    a.actor == "local_user"
                        && a.action == format!("recover-orchestration:{}", round.id)
                        && a.scope_hash == r.approval_scope
                        && a.spec_revision == round.spec_revision
                })
                && r.handoff.get("review_known_agents").is_some_and(|v| {
                    serde_json::from_value::<BTreeSet<String>>(v.clone())
                        .is_ok_and(|a| &a == agents)
                })
        })
    };
    require(
        bound,
        "review agent catalogue differs from its original input or approved recovery",
    )?;
    let expected = plan(run, round, response)?;
    require(
        !expected.is_empty()
            && expected.len() == round.review_parts.len()
            && expected
                .iter()
                .zip(&round.review_parts)
                .all(|(a, b)| a.id == b.id && a.check_ids == b.check_ids),
        "review partition plan changed or omits original checks",
    )
}
pub fn pending(round: &Round) -> Option<&Part> {
    round.review_parts.iter().find(|p| p.output.is_none())
}
pub fn request(run: &Run, round: &Round, response: &Response, part: &Part) -> Result<PreparedTurn> {
    validate_plan(run, round, response)?;
    require(
        round
            .review_parts
            .iter()
            .any(|p| p.id == part.id && p.check_ids == part.check_ids),
        "review request does not belong to the frozen partition plan",
    )?;
    let mut request = super::verifier_request_checks(run, round, response, part.check_ids.clone())?;
    request.prompt.push_str(&format!("\nNative review part {}. Evaluate exactly this part's expected_checks against the complete supplied original context and resolved proposal. Other parts retain their own obligations; do not generate their check results here. Keep each result specific and concise, using shared proofs only for identical methods/receipt sets. Missing proof is fail/blocked, never assumed pass. This part alone cannot approve the whole proposal.",part.id));
    Ok(request)
}
pub fn authenticate_part(
    run: &Run,
    round: &Round,
    response: &Response,
    part: &Part,
    native: &mut BTreeSet<(String, String)>,
) -> Result<ReviewResponse> {
    authenticate_input(run, round, response, part, native)?;
    let review: ReviewResponse =
        crate::transport::decode(part.output.as_ref().unwrap().as_bytes())?;
    super::validate_review_checks(run, round, response, &review, &part.check_ids)?;
    Ok(review)
}
fn authenticate_input(
    run: &Run,
    round: &Round,
    response: &Response,
    part: &Part,
    native: &mut BTreeSet<(String, String)>,
) -> Result<()> {
    super::authenticate_receipt(
        run,
        round,
        &part.session_id,
        &part.output,
        "orchestrator:verifier",
        RoleKind::Verifier,
        native,
    )?;
    let session = &run.session_ledger.sessions[part.session_id.as_ref().unwrap()];
    let turn = &run.session_ledger.turns[session.current_turn_id.as_ref().unwrap()];
    let mut prepared = request(run, round, response, part)?.with_context_for_agents(
        &crate::providers::ObservedInvocation {
            turn_id: &turn.id,
            run,
            resume: None,
            context_packet: run.message_packets.get(&turn.id),
        },
        round.review_known_agents.as_ref(),
    )?;
    if let Some(recovery) = round.recoveries.iter().rev().find(|r| {
        r.session_id == session.id
            && r.kind != crate::runtime::orchestration_recovery::Kind::Restore
    }) {
        crate::runtime::orchestration_recovery::decorate(run, &mut prepared, recovery)?;
    }
    require(
        turn.prompt_hash == prepared.prompt_hash() && turn.schema_hash == prepared.schema_hash()?,
        "native review part input differs from its scoped request",
    )?;
    Ok(())
}
pub fn recovery_part<'a>(
    run: &Run,
    round: &'a Round,
    response: &Response,
) -> Result<Option<&'a Part>> {
    validate_plan(run, round, response)?;
    let mut native = BTreeSet::new();
    for part in &round.review_parts {
        if part.output.is_none() {
            return Ok(Some(part));
        }
        authenticate_input(run, round, response, part, &mut native)?;
        let report =
            crate::transport::decode::<ReviewResponse>(part.output.as_ref().unwrap().as_bytes());
        if report
            .and_then(|r| super::validate_review_checks(run, round, response, &r, &part.check_ids))
            .is_err()
        {
            return Ok(Some(part));
        }
    }
    Ok(None)
}
pub fn validate_progress(
    run: &Run,
    round: &Round,
    response: &Response,
) -> Result<BTreeSet<(String, String)>> {
    validate_plan(run, round, response)?;
    super::authenticate_turn_receipts(run, round, false)?;
    let executor = &run.session_ledger.sessions[round.executor_session.as_ref().unwrap()];
    let raw = executor.native_session_id.as_ref().unwrap();
    let mut native = BTreeSet::from([(
        executor.scope.provider.clone(),
        uuid::Uuid::parse_str(raw)
            .map(|id| id.to_string())
            .unwrap_or(raw.clone()),
    )]);
    let mut missing = false;
    for part in &round.review_parts {
        if part.output.is_none() {
            missing = true;
            continue;
        }
        require(
            !missing,
            "review parts completed out of the persisted order",
        )?;
        authenticate_part(run, round, response, part, &mut native)?;
    }
    Ok(native)
}
/// Derived data only: authentication always re-reads the exact native reports.
pub fn combine(run: &Run, round: &Round, response: &Response) -> Result<ReviewResponse> {
    validate_plan(run, round, response)?;
    super::authenticate_turn_receipts(run, round, false)?;
    let executor = &run.session_ledger.sessions[round.executor_session.as_ref().unwrap()];
    let raw = executor.native_session_id.as_ref().unwrap();
    let mut native = BTreeSet::from([(
        executor.scope.provider.clone(),
        uuid::Uuid::parse_str(raw)
            .map(|id| id.to_string())
            .unwrap_or(raw.clone()),
    )]);
    let mut checks = BTreeMap::new();
    let mut messages = vec![];
    for part in &round.review_parts {
        let review = authenticate_part(run, round, response, part, &mut native)?;
        messages.extend(review.agent_messages);
        for check in review.checks {
            // V01/V04 were separately mandatory in every authenticated part.
            checks.entry(check.check_id.clone()).or_insert(check);
        }
    }
    let review = ReviewResponse {
        agent_messages: messages,
        contract_version: round.contract_version.clone(),
        output_hash: hash(round.output.as_ref().unwrap()),
        spec_revision: round.spec_revision,
        input_hash: round.input_hash.clone(),
        checks: checks.into_values().collect(),
        findings: vec![],
    };
    super::validate_review(run, round, response, &review)?;
    Ok(review)
}
pub fn authenticate(run: &Run, round: &Round, response: &Response) -> Result<()> {
    let combined = combine(run, round, response)?;
    require(
        round.review.as_deref() == Some(serde_json::to_string(&combined)?.as_str()),
        "combined review differs from its authenticated native parts",
    )
}
