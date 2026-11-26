use super::*;

pub(super) fn record_plan_review_start(
    store: &Store,
    conversation: &str,
    plan: &str,
) -> Result<()> {
    store.append_event(
        conversation,
        None,
        Some(plan),
        "planning_progress",
        &json!({"text": "Reviewing the plan"}),
    )?;
    Ok(())
}

pub(super) fn plan_event_payload(
    text: &str,
    plan: &PreparedPlan,
    specs: &[TaskContract],
    review: &std::result::Result<Verdict, String>,
    waiting: Option<&str>,
    revision_token: String,
) -> Result<serde_json::Value> {
    Ok(json!({
        "plan": plan.id,
        "team_version": tasks::TEAM_VERSION,
        "document": tasks::render_plan_document(text, specs),
        "parallel": tasks::parallel_peers(specs),
        "tasks": plan.tasks.iter().map(|task| task.spec.clone()).collect::<Vec<_>>(),
        "review": review_json(review),
        "automatic": false,
        "waiting": waiting,
        "revision_token": revision_token,
    }))
}
