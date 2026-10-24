use super::*;

pub(super) fn record_plan_review_start(
    store: &Store,
    conversation: &str,
    plan: &str,
    config: &Config,
    specs: &[TaskSpec],
) -> Result<()> {
    if !config.policy.required_gates && !tasks::missing_quality_agents(specs).is_empty() {
        let _ = store.append_event(
            conversation,
            None,
            Some(plan),
            "notice",
            &json!({"text": "policy.required_gates is off: this plan changes code but gets no refactoring, static analysis, testing, security, performance or integration check"}),
        );
    }
    store.append_event(
        conversation,
        None,
        Some(plan),
        "planning_progress",
        &json!({"text": "Reviewing the plan"}),
    )?;
    Ok(())
}

pub(super) fn plan_review_prompt(
    project: &str,
    request: &str,
    specs: &[TaskSpec],
    first: bool,
) -> String {
    if first {
        tasks::plan_review_prompt(project, request, specs)
    } else {
        tasks::plan_review_followup(request, specs)
    }
}

pub(super) fn append_planning_gaps(prompt: String, gaps: Option<&str>) -> String {
    match gaps {
        Some(gaps) => format!("{prompt}\n\n{gaps}"),
        None => prompt,
    }
}

pub(super) fn plan_event_payload(
    text: &str,
    plan: &PreparedPlan,
    specs: &[TaskSpec],
    review: &std::result::Result<Verdict, String>,
    autonomous: bool,
    waiting: Option<&str>,
    scope_hash: String,
) -> Result<serde_json::Value> {
    Ok(json!({
        "plan": plan.id,
        "document": tasks::render_plan_document(text, specs),
        "parallel": tasks::parallel_peers(specs),
        "tasks": plan.tasks.iter().map(|task| task.spec.clone()).collect::<Vec<_>>(),
        "review": review_json(review),
        "automatic": autonomous && waiting.is_none(),
        "waiting": waiting,
        "scope_hash": scope_hash,
    }))
}
