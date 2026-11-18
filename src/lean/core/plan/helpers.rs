use crate::lean::*;

/// Hash a length-framed pair so untrusted IDs cannot form paths
/// or alias another plan/task pair by moving a delimiter between the IDs.
pub(in crate::lean) fn writer_base_scratch(state: &Path, plan_id: &str, spec_id: &str) -> PathBuf {
    let key = format!("{}:{plan_id}{spec_id}", plan_id.len());
    state
        .join("worktrees")
        .join(format!("base-{}", crate::hash(key.as_bytes())))
}

/// Add options prerequisites for eligible writers, excluding quality gates.
#[cfg(test)]
pub(in crate::lean) fn finalize_plan(
    specs: Vec<TaskContract>,
    required_gates: bool,
    local_options: bool,
) -> Vec<TaskContract> {
    finalize_team_plan(specs, required_gates, local_options, tasks::Team::Feature)
}

/// The plan `team` runs: its gates when policy requires gates, and options
/// when both policy and the team allow them.
pub(in crate::lean) fn finalize_team_plan(
    specs: Vec<TaskContract>,
    required_gates: bool,
    local_options: bool,
    team: tasks::Team,
) -> Vec<TaskContract> {
    let specs = if required_gates {
        tasks::with_team_gates(specs, team.gates())
    } else {
        specs
    };
    if local_options && team.options() {
        tasks::with_options(specs)
    } else {
        tasks::without_options(specs)
    }
}
