use super::super::*;
use crate::lean::prompts;

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

    let graph = editable_plan(core, conversation)?;
    record_revision_view(core, conversation, &graph)?;
    let graph_json = graph_json(core, &graph)?;
    Ok(PlanView {
        workflow,
        graph,
        graph_json,
    })
}

/// The plan `revise_plan` edits: the accepted one, which keeps running while
/// a replacement draft is drafted, sent back or awaits the user. Before any
/// plan is accepted, the latest draft.
fn editable_plan(core: &Core, conversation: &str) -> Result<Vec<Task>> {
    match core.store.accepted_plan(conversation)? {
        Some(plan) => core.store.plan(&plan),
        None => core.store.latest_plan(conversation),
    }
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
    core.revisions
        .views
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
    let checkpoints = graph
        .first()
        .map(|task| {
            let conversation = core.store.plan_conversation(&task.plan_id)?;
            core.store
                .unfinished_checkpoints(&conversation, &task.plan_id)
        })
        .transpose()?
        .unwrap_or_default();
    let states = graph.iter().map(|task| json!({
        "task":task.spec["id"], "summary":task.spec["summary"].as_str().or(task.spec["title"].as_str()), "depends_on":task.spec["depends_on"], "runtime_id":task.id, "status":task.status, "round":task.round, "input_generation":task.input_generation, "deferred":task.spec["deferred"] == true,
        "reason":task.current_reason().map(|text| tasks::bound(text, 2000)),
        "waiting_request":task.feedback.as_ref().and_then(|feedback| feedback.get("waiting_request")),
    })).collect::<Vec<_>>();
    Ok(crate::lean::prompt_context::untrusted(
        prompts::CURRENT_PLAN_INDEX_LABEL,
        &crate::lean::prompt_context::markdown(
            &json!({"plan":graph.first().map(|task|&task.plan_id),"base_revision":base_revision,"task_states":states,"unfinished_checkpoints":checkpoints}),
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_plan_snapshot_contains_recovery_states_without_polluting_contracts() {
        let directory = tempfile::tempdir().unwrap();
        let config = Config::from_toml(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/examples/antislop.toml"
        )))
        .unwrap();
        let core = Core::open(&directory.path().join("state"), config).unwrap();
        let conversation = core.store.create_conversation("fixture").unwrap().id;
        let specs = ["failed", "stalled", "done"]
            .map(|id| json!({"id":id,"title":id,"acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Check","change_kind": "none", "kind":"analysis"}));
        let mut tasks = core.store.add_plan(&conversation, &specs).unwrap();
        for (task, state) in
            tasks
                .iter_mut()
                .zip([status::FAILED, status::COORDINATING, status::APPROVED])
        {
            task.status = state.into();
            core.store.update_task(task).unwrap();
        }
        core.store
            .db()
            .execute(
                "INSERT INTO plan_activations(plan_id,conversation_id) VALUES(?1,?2)",
                rusqlite::params![tasks[0].plan_id, conversation],
            )
            .unwrap();
        core.store
            .add_plan(
                &conversation,
                &[json!({"id":"obsolete","title":"Draft","acceptance_criteria":[{"id":"C1","text":"The task goal is verified"}],"goal":"Draft","change_kind": "none", "kind":"analysis"})],
            )
            .unwrap();
        tasks[0].feedback = Some(json!({"reason":"binary apply failed ".repeat(130_000)}));
        core.store.update_task(&tasks[0]).unwrap();
        let view = load(&core, &conversation).unwrap();
        let snapshot = &view.graph_json;
        assert!(snapshot.contains(&format!("plan: {}", tasks[0].plan_id)));
        assert!(!snapshot.contains("The task goal is verified"));
        assert!(!snapshot.contains("binary apply failed ".repeat(130_000).as_str()));
        assert!(snapshot.contains("status: failed") && snapshot.contains("status: coordinating"));
        assert!(snapshot.contains("status: approved"));
        for task in &tasks {
            assert!(snapshot.contains(&task.id));
        }
        assert!(snapshot.contains("round:") && snapshot.contains("input_generation:"));
        assert!(!snapshot.contains("revise_plan") && !snapshot.contains("resume_task"));
        assert!(snapshot.contains("read_task_contract"));
    }
}
