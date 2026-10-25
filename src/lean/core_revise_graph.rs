use super::core_plan_helpers::finalize_plan;
use super::*;

impl Core {
    pub(in crate::lean) fn revise_running_graph(
        self: &Arc<Self>,
        conversation: &str,
        project: &Path,
        text: &str,
        baseline: &[Task],
        specs: &[TaskSpec],
        cancel: &AtomicBool,
    ) -> Result<()> {
        require_running_graph(baseline)?;
        let specs = finalize_plan(specs.to_vec(), self.config_snapshot().policy.required_gates);
        let findings = feasibility::check(&specs);
        let blocking: Vec<_> = findings
            .iter()
            .filter(|finding| finding.severity == feasibility::Severity::Block)
            .collect();
        require(blocking.is_empty(), &feasibility::message(&blocking))?;
        let plan = &baseline[0].plan_id;
        let review = self.review(
            conversation,
            PLAN_VERIFIER,
            Some(plan),
            project,
            cancel,
            |_, request| graph_review_prompt(project, request, &specs, baseline),
        )?;
        let verdict = accepted_graph_review(review, cancel)?;
        self.apply_running_graph_edit(conversation, plan, text, baseline, &specs, verdict)?;
        self.maybe_continue_plan(conversation, plan)?;
        Ok(())
    }

    fn apply_running_graph_edit(
        &self,
        conversation: &str,
        plan: &str,
        text: &str,
        baseline: &[Task],
        specs: &[TaskSpec],
        verdict: Verdict,
    ) -> Result<()> {
        let graph_guard = self.graph_updates.lock().expect("graph update mutex");
        let _integration = self.integrating.lock().expect("integration mutex");
        validate_editable_tasks(&self.task_cancels, baseline, specs)?;
        self.store
            .extend_running_graph(conversation, baseline, specs)?;
        let scope_hash = self.plan_scope_hash(conversation, plan)?;
        self.store.append_event(
            conversation,
            None,
            Some(plan),
            "graph_updated",
            &json!({
                "plan":plan,"text":text,"scope_hash":scope_hash,"review":verdict,
                "order":specs.iter().map(|spec| &spec.id).collect::<Vec<_>>()
            }),
        )?;
        drop(graph_guard);
        Ok(())
    }
}

fn accepted_graph_review(
    review: std::result::Result<Verdict, String>,
    cancel: &AtomicBool,
) -> Result<Verdict> {
    require(!cancel.load(Ordering::SeqCst), "graph review was cancelled")?;
    let verdict = review.map_err(Error::Invalid)?;
    require(
        verdict.approved,
        &format!(
            "graph reviewer rejected the edit: {}",
            serde_json::to_string(&verdict.findings)?
        ),
    )?;
    Ok(verdict)
}

fn require_running_graph(baseline: &[Task]) -> Result<()> {
    require(
        baseline.iter().any(|task| {
            matches!(
                task.status.as_str(),
                status::QUEUED | status::WORKING | status::VERIFYING
            )
        }),
        "this graph has finished; send a new request to start another interview",
    )
}

fn graph_review_prompt(
    project: &Path,
    request: &str,
    specs: &[TaskSpec],
    baseline: &[Task],
) -> String {
    let states: Vec<_> = baseline
        .iter()
        .map(|task| {
            json!({
                "id":task.spec["id"], "status":task.status, "round":task.round, "spec":task.spec
            })
        })
        .collect();
    format!(
        "{}\nThis is an additive edit of a running graph, explicitly authorized by the user. Existing tasks and execution history must remain. Only dependencies of unstarted tasks can change. Verify the new tasks are within the already agreed requirements and retain all necessary security, performance, test and integration gates after the new implementation. Reject scope expansion. Current tasks and execution states:\n{}",
        tasks::plan_review_prompt(&project.to_string_lossy(), request, specs),
        serde_json::to_string(&states).unwrap_or_default()
    )
}

fn validate_editable_tasks(
    task_cancels: &Mutex<HashMap<String, (String, Arc<AtomicBool>)>>,
    baseline: &[Task],
    specs: &[TaskSpec],
) -> Result<()> {
    let active = task_cancels.lock().expect("task cancel mutex");
    for task in baseline {
        let changed = specs.iter().any(|spec| {
            task.spec["id"].as_str() == Some(&spec.id)
                && serde_json::from_value::<TaskSpec>(task.spec.clone())
                    .is_ok_and(|before| before != *spec)
        });
        require(
            !changed || !active.contains_key(&task.id),
            "a task affected by this edit has already been scheduled; retry without changing that task",
        )?;
    }
    Ok(())
}
