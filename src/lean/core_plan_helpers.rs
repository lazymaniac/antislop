use super::*;

/// Hash a length-framed pair so untrusted IDs cannot form paths
/// or alias another plan/task pair by moving a delimiter between the IDs.
pub(super) fn writer_base_scratch(state: &Path, plan_id: &str, spec_id: &str) -> PathBuf {
    let key = format!("{}:{plan_id}{spec_id}", plan_id.len());
    state
        .join("worktrees")
        .join(format!("base-{}", crate::hash(key.as_bytes())))
}

/// Add options after quality gates so every writer, including gates, is covered.
pub(super) fn finalize_plan(specs: Vec<TaskContract>, required_gates: bool) -> Vec<TaskContract> {
    let specs = if required_gates {
        tasks::with_required_agents(specs)
    } else {
        specs
    };
    tasks::with_options(specs)
}

/// What a plan that changes code lacks of the agents the pool makes required
/// before the change (the gates after it are added by the runtime): told to
/// the plan verifier, who rejects the plan for it.
pub(super) fn planning_gaps(specs: &[TaskContract]) -> Option<String> {
    if !specs.iter().any(|spec| spec.writes) {
        return None;
    }
    let mut gaps = Vec::new();
    if !specs.iter().any(|spec| spec.kind == WorkKind::Context) {
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
