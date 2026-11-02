use super::super::*;

pub(in crate::lean) fn build<'a>(
    conversation: &'a str,
    task: &Task,
    role: String,
    profile: Result<ResolvedProfile>,
    cwd: &Path,
    add_dirs: Vec<PathBuf>,
) -> TurnSetup<'a> {
    TurnSetup {
        session_policy: SessionPolicy::TaskRound {
            task: task.id.clone(),
            round: task.round,
        },
        conversation,
        plan_id: Some(task.plan_id.clone()),
        role,
        profile,
        cwd: cwd.to_path_buf(),
        add_dirs,
        access: agent::Access::ReadOnly,
        upto_message: 0,
    }
}
