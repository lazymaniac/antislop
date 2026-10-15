//! Version-bound edits to an actual earlier native proposal. No execution state is editable.
use super::*;
use crate::team::{AgentRecord, TaskRecord};

pub const FORMAT: &str = "team-plan-amendment-v1";
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Fields {
    pub id: String,
    pub set: BTreeMap<String, Value>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Amendment {
    pub format: String,
    pub base_round_id: String,
    pub base_output_hash: String,
    #[serde(default)]
    pub task_fields: Vec<Fields>,
    #[serde(default)]
    pub agent_fields: Vec<Fields>,
    #[serde(default)]
    pub add_tasks: Vec<TaskRecord>,
    #[serde(default)]
    pub add_agents: Vec<AgentRecord>,
    #[serde(default)]
    pub remove_tasks: BTreeSet<String>,
    #[serde(default)]
    pub remove_agents: BTreeSet<String>,
    #[serde(default)]
    pub risk_updates: Vec<RiskOwner>,
    #[serde(default)]
    pub communicates_with: Option<BTreeSet<(String, String)>>,
}
#[derive(Debug, Clone, Serialize)]
pub struct Resolved {
    pub team: TeamPlan,
    pub risk_owners: Vec<RiskOwner>,
    pub bases: Vec<(String, String)>,
}
fn parse(raw: &str) -> Result<Option<Amendment>> {
    let value: Value = crate::transport::decode(raw.as_bytes())?;
    if value.get("format").is_none() {
        return Ok(None);
    }
    let amended: Amendment = serde_json::from_value(value)?;
    require(
        amended.format == FORMAT,
        "unsupported team amendment format",
    )?;
    Ok(Some(amended))
}
fn base_response(run: &Run, index: usize) -> Result<Response> {
    let round = &run.orchestration_rounds[index];
    require(
        !matches!(round.previous_status, Status::Completed | Status::Cancelled),
        "terminal dialogue is not a team amendment base",
    )?;
    let response = super::authenticate_turn_receipts(run, round, false)?;
    require(
        response.team_plan_json.is_some() && response.intake_proposal_json.is_none(),
        "amendment base is not a native team proposal",
    )?;
    Ok(response)
}
/// Iterative backwards traversal prevents forward references, cycles and stack growth.
pub fn resolve(run: &Run, response: &Response) -> Result<Resolved> {
    resolve_before(
        run,
        response,
        run.orchestration_rounds.len().saturating_sub(1),
    )
}
pub(super) fn resolve_before(run: &Run, response: &Response, mut bound: usize) -> Result<Resolved> {
    let mut current = response.clone();
    let mut chain = vec![];
    let mut bases = vec![];
    let (mut team, mut risks) = loop {
        let raw = current
            .team_plan_json
            .as_ref()
            .ok_or_else(|| crate::Error::Invalid("no team proposal".into()))?;
        let Some(amendment) = parse(raw)? else {
            break (
                crate::transport::decode::<TeamPlan>(raw.as_bytes())?,
                current.risk_owners,
            );
        };
        require(
            current.risk_owners.is_empty(),
            "amendments inherit risk ownership; use explicit risk_updates",
        )?;
        let index = run
            .orchestration_rounds
            .iter()
            .position(|r| r.id == amendment.base_round_id)
            .ok_or_else(|| crate::Error::Invalid("unknown amendment base".into()))?;
        require(
            index < bound,
            "amendment must reference an earlier proposal, never itself or a forward round",
        )?;
        let round = &run.orchestration_rounds[index];
        require(
            round
                .output
                .as_ref()
                .is_some_and(|raw| hash(raw) == amendment.base_output_hash),
            "amendment base output changed",
        )?;
        current = base_response(run, index)?;
        bound = index;
        bases.push((round.id.clone(), amendment.base_output_hash.clone()));
        chain.push(amendment);
    };
    let allowed_risks = run.risk_coverage.keys().cloned().collect();
    for amendment in chain.into_iter().rev() {
        apply(&mut team, &mut risks, amendment, &allowed_risks)?;
    }
    Ok(Resolved {
        team,
        risk_owners: risks,
        bases,
    })
}
fn edits<T: Serialize + for<'de> Deserialize<'de>>(
    items: &mut [T],
    changes: Vec<Fields>,
    allowed: &[&str],
    removed: &BTreeSet<String>,
) -> Result<()> {
    let mut seen = BTreeSet::new();
    for change in changes {
        require(
            seen.insert(change.id.clone())
                && !removed.contains(&change.id)
                && !change.set.is_empty(),
            "duplicate, removed or empty entity amendment",
        )?;
        require(
            change.set.keys().all(|k| allowed.contains(&k.as_str())),
            "amendment cannot edit identity, execution history, sessions, profiles or outcomes",
        )?;
        let mut found = false;
        for item in items.iter_mut() {
            let mut value = serde_json::to_value(&*item)?;
            if value["id"] != change.id {
                continue;
            }
            found = true;
            for (key, replacement) in &change.set {
                value[key] = replacement.clone();
            }
            *item = serde_json::from_value(value)?;
            break;
        }
        require(found, "amendment entity does not exist")?;
    }
    Ok(())
}
fn apply(
    team: &mut TeamPlan,
    risks: &mut Vec<RiskOwner>,
    patch: Amendment,
    allowed_risks: &BTreeSet<String>,
) -> Result<()> {
    for id in &patch.remove_tasks {
        require(
            team.tasks.iter().any(|t| &t.id == id),
            "removed task does not exist",
        )?;
    }
    for id in &patch.remove_agents {
        require(
            team.agents.iter().any(|a| &a.id == id),
            "removed agent does not exist",
        )?;
    }
    edits(
        &mut team.tasks,
        patch.task_fields,
        &[
            "owner_id",
            "verifier_id",
            "description",
            "stage",
            "requirement_ids",
            "criterion_ids",
            "plan_item_ids",
            "deliverable",
            "dependencies",
            "required_gates",
            "source_dependencies",
            "write_scopes",
            "check_plan",
            "change_contract",
            "integration",
            "escalation",
        ],
        &patch.remove_tasks,
    )?;
    edits(
        &mut team.agents,
        patch.agent_fields,
        &["role", "kind", "supervisor_id", "verifier_id", "rationale"],
        &patch.remove_agents,
    )?;
    team.tasks.retain(|t| !patch.remove_tasks.contains(&t.id));
    team.agents.retain(|a| !patch.remove_agents.contains(&a.id));
    for task in patch.add_tasks {
        require(
            !team.tasks.iter().any(|t| t.id == task.id) && !patch.remove_tasks.contains(&task.id),
            "added task identity already exists or was removed",
        )?;
        team.tasks.push(task);
    }
    for agent in patch.add_agents {
        require(
            !team.agents.iter().any(|a| a.id == agent.id)
                && !patch.remove_agents.contains(&agent.id),
            "added agent identity already exists or was removed",
        )?;
        team.agents.push(agent);
    }
    if let Some(edges) = patch.communicates_with {
        team.communicates_with = edges;
    }
    let mut seen = BTreeSet::new();
    for update in patch.risk_updates {
        require(
            seen.insert(update.risk_id.clone()),
            "duplicate amended risk owner",
        )?;
        require(
            allowed_risks.contains(&update.risk_id),
            "amendment cannot invent a risk identity",
        )?;
        risks.retain(|r| r.risk_id != update.risk_id);
        risks.push(update);
    }
    Ok(())
}
pub fn available_bases(run: &Run) -> Vec<Value> {
    (0..run.orchestration_rounds.len()).filter_map(|i| {
        let response=base_response(run,i).ok()?;
        let raw=response.team_plan_json.as_ref()?;
        let round=&run.orchestration_rounds[i];
        Some(json!({"round_id":round.id,"output_hash":round.output.as_ref().map(hash),"proposal_hash":hash(raw),"diagnostic":round.diagnostic,"status":round.status}))
    }).collect()
}
