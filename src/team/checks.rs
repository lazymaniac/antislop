//! Exact locally executed check plans, distinct from a model's tool-run claims.
use super::*;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckPhase {
    BeforeExecution,
    #[default]
    BeforeVerification,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlannedCheck {
    pub id: String,
    pub argv: Vec<String>,
    #[serde(default)]
    pub phase: CheckPhase,
    #[serde(default)]
    pub expected_exit_code: i32,
    pub criterion_ids: BTreeSet<String>,
    pub gate_ids: BTreeSet<String>,
    pub rationale: String,
}
impl PlannedCheck {
    pub fn review_id(&self) -> String {
        format!(
            "{}:{}",
            if self.phase == CheckPhase::BeforeExecution {
                "BASELINE"
            } else {
                "RUNNER"
            },
            self.id
        )
    }
}
pub fn runner_gates(role: CatalogRole, family: TaskFamily) -> BTreeSet<String> {
    let owned = role_quality_gates(role, family);
    crate::quality::gate_definitions()
        .into_iter()
        .filter(|g| {
            g.runner_required
                && (!g.software_only || family == TaskFamily::Software)
                && owned
                    .iter()
                    .any(|(id, phase)| *id == g.gate_id && *phase == g.phase)
        })
        .map(|g| crate::quality::gate_check_id(&g.gate_id, g.phase))
        .collect()
}
pub fn validate_plan(
    task: &TaskRecord,
    require_coverage: bool,
    role: CatalogRole,
    family: TaskFamily,
) -> Result<()> {
    let mut ids = BTreeSet::new();
    let allowed: BTreeSet<_> = crate::quality::gate_definitions()
        .into_iter()
        .filter(|g| !g.software_only || family == TaskFamily::Software)
        .map(|g| crate::quality::gate_check_id(&g.gate_id, g.phase))
        .collect();
    let mut criteria = BTreeSet::new();
    let mut gates = BTreeSet::new();
    for check in &task.check_plan {
        require(
            text(&check.id)
                && ids.insert(&check.id)
                && text(&check.rationale)
                && !check.criterion_ids.is_empty()
                && check.criterion_ids.is_subset(&task.criterion_ids)
                && check.gate_ids.is_subset(&allowed),
            "task check has missing/duplicate identity, invalid scope or no coverage rationale",
        )?;
        crate::domain::Run::check_action(&check.argv)?;
        require(
            check.expected_exit_code >= 0
                && (check.phase == CheckPhase::BeforeExecution || check.expected_exit_code == 0),
            "only a baseline may expect a known nonzero exit code",
        )?;
        if check.phase == CheckPhase::BeforeVerification {
            criteria.extend(check.criterion_ids.iter().cloned());
            gates.extend(check.gate_ids.iter().cloned());
        }
    }
    if require_coverage {
        let mandatory = runner_gates(role, family);
        require(
            mandatory.is_subset(&gates) && (mandatory.is_empty() || criteria == task.criterion_ids),
            "task runner plan omits a required gate or assigned criterion",
        )?;
    }
    Ok(())
}
