use super::super::*;

pub(super) struct PlanView {
    pub workflow: store::Workflow,
    pub graph: Vec<Task>,
    pub graph_json: String,
}

pub(super) fn load(core: &Core, conversation: &str) -> Result<PlanView> {
    let workflow = core.store.workflow(conversation)?;
    if workflow.planning() {
        announce_planning(core, conversation)?;
    }

    let graph = core.store.latest_plan(conversation)?;
    record_revision_view(core, conversation, &graph)?;
    let graph_json = graph_json(core, &graph)?;
    Ok(PlanView {
        workflow,
        graph,
        graph_json,
    })
}

fn announce_planning(core: &Core, conversation: &str) -> Result<()> {
    core.store.append_event(
        conversation,
        None,
        None,
        "planning_progress",
        &json!({"text":"Working on the plan draft"}),
    )?;
    Ok(())
}

fn record_revision_view(core: &Core, conversation: &str, graph: &[Task]) -> Result<()> {
    let view = graph
        .first()
        .map(|task| {
            Ok::<_, Error>((
                task.plan_id.clone(),
                core.store.plan_revision(&task.plan_id)?,
            ))
        })
        .transpose()?;
    core.revision_views
        .lock()
        .expect("revision views mutex")
        .insert(conversation.into(), view);
    Ok(())
}

fn graph_json(core: &Core, graph: &[Task]) -> Result<String> {
    let base_revision = graph
        .first()
        .map(|task| core.store.plan_revision(&task.plan_id))
        .transpose()?;
    serde_json::to_string(&json!({"base_revision":base_revision,"tasks":graph.iter().map(|task|&task.spec).collect::<Vec<_>>()})).map_err(Error::from)
}
