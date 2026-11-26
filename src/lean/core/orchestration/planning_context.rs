use crate::lean::*;
pub(in crate::lean) fn planning_scope_context(
    core: &Core,
    conversation: &str,
    plan: Option<&str>,
    confirmed_scope: &str,
) -> Result<Option<String>> {
    let latest;
    let plan = match plan {
        Some(plan) => Some(plan),
        None => {
            latest = core.store.latest_plan(conversation)?;
            latest.first().map(|task| task.plan_id.as_str())
        }
    };
    let scope = plan
        .map(|plan| core.store.plan_scope_id(plan))
        .transpose()?
        .flatten();
    let mut text = match &scope {
        Some(id) => core.store.scope_sources(conversation, id)?,
        None => String::new(),
    };
    let additional = core
        .store
        .additional_source_context(conversation, scope.as_deref())?;
    if scope.is_none() && !additional.is_empty() && !confirmed_scope.trim().is_empty() {
        text.push_str(confirmed_scope);
        text.push('\n');
    }
    text.push_str(&additional);
    Ok((!text.is_empty()).then_some(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_plan_with_an_attachment_keeps_the_confirmed_scope() {
        let (_dir, core, conversation) = crate::lean::agent_tools::tests::core();
        let source = core
            .store
            .register_source(&conversation, "Attached source body")
            .unwrap();
        let scope = planning_scope_context(
            &core,
            &conversation,
            None,
            "Confirmed behavior and acceptance criteria",
        )
        .unwrap()
        .unwrap();
        assert!(scope.contains("Confirmed behavior and acceptance criteria"));
        assert!(scope.contains(&source.id));
        assert!(!scope.contains("Attached source body"));
    }

    #[test]
    fn a_turn_bound_to_the_accepted_plan_gets_its_scope() {
        let (_dir, core, conversation) = crate::lean::agent_tools::tests::core();
        let spec = json!({"id":"inspect", "title":"Inspect", "goal":"Check", "kind":"analysis",
            "change_kind":"none", "acceptance_criteria":[{"id":"C1","text":"Check the behavior"}]});
        let add = |summary: &str| {
            let plan = core
                .store
                .add_plan(&conversation, std::slice::from_ref(&spec))
                .unwrap()
                .remove(0)
                .plan_id;
            let scope = core.store.create_scope(&conversation, summary).unwrap();
            core.store
                .bind_plan_scope(&conversation, &plan, &scope.id)
                .unwrap();
            plan
        };
        let accepted = add("Accepted scope for running tasks");
        add("Scope of newer draft");
        let text = planning_scope_context(&core, &conversation, Some(&accepted), "")
            .unwrap()
            .unwrap();
        assert!(text.contains("Accepted scope for running tasks"));
        assert!(!text.contains("Scope of newer draft"));
    }
}
