//! Process-local, fully authenticated team material. No deserializer: a ready
//! flag or a client proposal cannot construct acceptance authority.
use crate::{
    Result,
    domain::Run,
    hash,
    orchestration::planning::{self, RiskOwner, Round, RoundStatus},
    require,
    team::TeamPlan,
};

pub(crate) fn reviewed_round(run: &Run) -> Option<&Round> {
    run.orchestration_rounds.last().filter(|round| {
        round.status == RoundStatus::Reviewed
            && round
                .output
                .as_deref()
                .and_then(|s| crate::transport::decode::<planning::Response>(s.as_bytes()).ok())
                .is_some_and(|r| r.team_plan_json.is_some())
    })
}

pub(crate) struct Prepared {
    run_hash: String,
    round_id: String,
    scope: String,
    team: TeamPlan,
    risk_owners: Vec<RiskOwner>,
}
impl Prepared {
    pub(crate) fn build(run: &Run) -> Result<Self> {
        require(
            !run.reconciliation_required
                && matches!(
                    run.status,
                    crate::domain::Status::AwaitingStart
                        | crate::domain::Status::Ready
                        | crate::domain::Status::AwaitingResult
                        | crate::domain::Status::ChangesRequested
                        | crate::domain::Status::Blocked
                ),
            "team acceptance preparation requires a stopped reconciled run",
        )?;
        let round = reviewed_round(run).ok_or_else(|| {
            crate::Error::Invalid("no reviewed team candidate to authenticate".into())
        })?;
        require(
            !planning::new_context_after_round(run, round)
                && run.team.as_ref().is_none_or(planning::replaceable),
            "team acceptance candidate is superseded or has execution history",
        )?;
        let response = planning::authenticate(run, round)?;
        let team = planning::validate_response(run, &response)?
            .ok_or_else(|| crate::Error::Invalid("reviewed response has no team".into()))?;
        let risk_owners = planning::amendment::resolve(run, &response)?.risk_owners;
        super::orchestration_recovery::sources_current(run)?;
        Ok(Self {
            run_hash: hash(serde_json::to_vec(run)?),
            round_id: round.id.clone(),
            scope: planning::accept_scope(run, round)?,
            team,
            risk_owners,
        })
    }
    pub(crate) fn validate_current(&self, run: &Run) -> Result<()> {
        require(
            self.run_hash == hash(serde_json::to_vec(run)?),
            "authenticated team state changed",
        )?;
        super::orchestration_recovery::sources_current(run)
    }
    pub(crate) fn material(
        &self,
        run: &Run,
        round_id: &str,
        scope: &str,
    ) -> Result<(TeamPlan, Vec<RiskOwner>)> {
        self.validate_current(run)?;
        require(
            self.round_id == round_id && self.scope == scope,
            "authenticated team approval scope changed",
        )?;
        Ok((self.team.clone(), self.risk_owners.clone()))
    }
}
