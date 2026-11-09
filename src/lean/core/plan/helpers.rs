use crate::lean::*;

/// Hash a length-framed pair so untrusted IDs cannot form paths
/// or alias another plan/task pair by moving a delimiter between the IDs.
pub(in crate::lean) fn writer_base_scratch(state: &Path, plan_id: &str, spec_id: &str) -> PathBuf {
    let key = format!("{}:{plan_id}{spec_id}", plan_id.len());
    state
        .join("worktrees")
        .join(format!("base-{}", crate::hash(key.as_bytes())))
}

/// Add options after quality gates so every writer, including gates, is covered.
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

/// What a plan that changes code lacks of the agents the pool makes required
/// before the change (the gates after it are added by the runtime): told to
/// the plan reviewer, who rejects the plan for it.
pub(in crate::lean) fn planning_gaps(specs: &[TaskContract], team: tasks::Team) -> Option<String> {
    if !specs.iter().any(|spec| spec.changes_code()) {
        return None;
    }
    let mut gaps = Vec::new();
    if team.needs_context_scout() && !specs.iter().any(|spec| spec.kind == WorkKind::Context) {
        gaps.push(
            "no context scout: a read-only task of kind `context` that reads the files, \
             contracts and invariants the change touches, which the writing tasks depend on",
        );
    }
    if gaps.is_empty() {
        return None;
    }
    Some(format!(
        "The runtime requires these for a plan that changes code, and this one lacks them; \
         do not approve it until it has them:\n- {}",
        gaps.join("\n- ")
    ))
}
