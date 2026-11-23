use crate::lean::*;

/// Hash a length-framed pair so untrusted IDs cannot form paths
/// or alias another plan/task pair by moving a delimiter between the IDs.
pub(in crate::lean) fn writer_base_scratch(state: &Path, plan_id: &str, spec_id: &str) -> PathBuf {
    let key = format!("{}:{plan_id}{spec_id}", plan_id.len());
    state
        .join("worktrees")
        .join(format!("base-{}", crate::hash(key.as_bytes())))
}

/// Apply the selected rigor's generated tasks; configuration has no overrides.
#[cfg(test)]
pub(in crate::lean) fn finalize_plan(specs: Vec<TaskContract>) -> Vec<TaskContract> {
    finalize_team_plan(specs, tasks::Team::Feature)
}

pub(in crate::lean) fn finalize_team_plan(
    specs: Vec<TaskContract>,
    team: tasks::Team,
) -> Vec<TaskContract> {
    if team == tasks::Team::NoQualityGates {
        return tasks::finalize_no_quality_gates(specs);
    }
    let specs = if team == tasks::Team::Feature {
        tasks::with_feature_gates(specs)
    } else {
        tasks::with_team_gates(specs, team.gates())
    };
    if team.options() {
        tasks::with_options(specs)
    } else {
        tasks::without_options(specs)
    }
}
