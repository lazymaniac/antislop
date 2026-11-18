use crate::lean::*;
pub(in crate::lean) fn planning_scope_context(
    core: &Core,
    conversation: &str,
) -> Result<Option<String>> {
    let plan = core.store.latest_plan(conversation)?;
    let scope = plan
        .first()
        .map(|plan| core.store.plan_scope_id(&plan.plan_id))
        .transpose()?
        .flatten();
    let mut text = match &scope {
        Some(id) => core.store.scope_sources(conversation, id)?,
        None => String::new(),
    };
    text.push_str(
        &core
            .store
            .additional_source_context(conversation, scope.as_deref())?,
    );
    Ok((!text.is_empty()).then_some(text))
}
