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
        "task":task.spec["id"], "runtime_id":task.id, "status":task.status, "round":task.round, "input_generation":task.input_generation, "deferred":task.spec["deferred"] == true,
        "reason":task.feedback.as_ref().and_then(|feedback| feedback.get("reason")).and_then(serde_json::Value::as_str).map(|text| tasks::bound(text, 2000)),
        "recovery":if task.spec["deferred"] == true { "deliberate scheduling hold; release by reviewed revision when its prerequisite is resolved" } else { match task.status.as_str() {
            status::FAILED => "retry_task for a transient CLI failure; revise_plan for a contract change",
            status::COORDINATING => "resume_task with current round/input_generation for a resolved blocker or independent authorized work; keep unresolved steps held; revise_plan for contract/dependency changes",
            status::WAITING => "answer_agent for the existing question, respecting its disposition",
            _ => "do not retry; preserve approved work and let queued/running tasks proceed",
        }}
    })).collect::<Vec<_>>();
    serde_json::to_string(&json!({"plan":graph.first().map(|task|&task.plan_id),"base_revision":base_revision,"tasks":graph.iter().map(|task|&task.spec).collect::<Vec<_>>(),"task_states":states,"unfinished_checkpoints":checkpoints})).map_err(Error::from)
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
            .map(|id| json!({"id":id,"title":id,"goal":"Check","change_kind": "none", "kind":"analysis"}));
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
                &[json!({"id":"obsolete","title":"Draft","goal":"Draft","change_kind": "none", "kind":"analysis"})],
            )
            .unwrap();
        tasks[0].feedback = Some(json!({"reason":"binary apply failed ".repeat(130_000)}));
        core.store.update_task(&tasks[0]).unwrap();
        let view = load(&core, &conversation).unwrap();
        let snapshot: serde_json::Value = serde_json::from_str(&view.graph_json).unwrap();
        assert!(snapshot["task_states"][0]["reason"].as_str().unwrap().len() < 2100);
        assert_eq!(snapshot["plan"], tasks[0].plan_id);
        assert_eq!(snapshot["tasks"], json!(specs));
        assert_eq!(snapshot["unfinished_checkpoints"], json!([]));
        assert_eq!(snapshot["task_states"][0]["status"], status::FAILED);
        assert_eq!(snapshot["task_states"][1]["status"], status::COORDINATING);
        assert_eq!(
            snapshot["task_states"][1]["input_generation"],
            tasks[1].input_generation
        );
        assert!(
            snapshot["task_states"][1]["recovery"]
                .as_str()
                .unwrap()
                .contains("revise_plan")
        );
        assert!(
            snapshot["task_states"][1]["recovery"]
                .as_str()
                .unwrap()
                .contains("resume_task")
        );
        assert_eq!(snapshot["task_states"][2]["status"], status::APPROVED);
        assert_eq!(snapshot["task_states"][2]["runtime_id"], tasks[2].id);
    }
}
