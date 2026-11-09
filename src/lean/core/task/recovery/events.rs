use super::*;

pub(super) fn append(core: &Core, work: &RecoveryWork, payload: &serde_json::Value) -> Result<()> {
    core.store.append_event(
        &work.req.conversation,
        None,
        Some(&work.req.plan),
        "task_recovery",
        payload,
    )?;
    Ok(())
}
