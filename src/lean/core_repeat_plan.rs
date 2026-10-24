use super::*;

#[path = "core_repeat_plan/delivery.rs"]
mod delivery;

impl Core {
    /// Whether `specs` asks for the same work as the last plan of the
    /// conversation that ran (ids aside).
    pub(in crate::lean) fn repeats_a_plan_that_ran(
        &self,
        conversation: &str,
        plan_id: &str,
        specs: &[TaskSpec],
    ) -> Result<bool> {
        let work = |spec: &TaskSpec| {
            (
                spec.title.clone(),
                spec.goal.clone(),
                spec.kind,
                spec.writes,
            )
        };
        let Some(earlier) = self.store.last_plan_that_ran(conversation, plan_id)? else {
            return Ok(false);
        };
        let mut before: Vec<_> = earlier
            .iter()
            .filter_map(|task| serde_json::from_value::<TaskSpec>(task.spec.clone()).ok())
            .map(|spec| work(&spec))
            .collect();
        let mut now: Vec<_> = specs.iter().map(work).collect();
        before.sort();
        now.sort();
        Ok(before == now)
    }

    /// Sends the ```message blocks of `answer` and returns the answer
    /// without them.
    pub(in crate::lean) fn post(
        self: &Arc<Self>,
        conversation: &str,
        plan_id: Option<&str>,
        sender: &str,
        answer: &str,
    ) -> Result<String> {
        let (text, outgoing, unreadable) = tasks::parse_messages_checked(answer);
        for body in unreadable {
            self.store.append_event(
                conversation,
                None,
                plan_id,
                "mail_rejected",
                &json!({"sender": sender, "recipient": "?", "text": body,
                        "reason": "unreadable message block: invalid JSON or an unescaped ``` inside it"}),
            )?;
        }
        for message in outgoing {
            self.deliver(conversation, plan_id, sender, message)?;
        }
        Ok(text)
    }

    /// The approved reports of the tasks `spec` depends on, as prompt text.
    pub(in crate::lean) fn upstream(&self, plan_id: &str, spec: &TaskSpec) -> Result<String> {
        let reports = self.store.dependency_reports(plan_id, &spec.depends_on)?;
        Ok(tasks::dependency_reports_note(&reports))
    }

    /// Where a writer starts: from the result of the one task it depends on,
    /// from the user's HEAD when none of them wrote, and from HEAD with all
    /// their results laid on it when several did.
    pub(in crate::lean) fn dependency_tasks(
        &self,
        plan_id: &str,
        spec: &TaskSpec,
    ) -> Result<Vec<Task>> {
        let plan = self.store.plan(plan_id)?;
        let mut seen = std::collections::HashSet::new();
        let mut ordered = Vec::new();
        for dep in &spec.depends_on {
            collect_dependencies(dep, &plan, &mut seen, &mut ordered)?;
        }
        Ok(ordered)
    }
}

fn collect_dependencies(
    id: &str,
    plan: &[Task],
    seen: &mut std::collections::HashSet<String>,
    ordered: &mut Vec<Task>,
) -> Result<()> {
    if !seen.insert(id.to_owned()) {
        return Ok(());
    }
    let task = plan
        .iter()
        .find(|task| task.spec["id"] == id)
        .ok_or_else(|| Error::Invalid(format!("missing dependency {id}")))?;
    let dependencies: Vec<String> = serde_json::from_value(task.spec["depends_on"].clone())?;
    for dependency in dependencies {
        collect_dependencies(&dependency, plan, seen, ordered)?;
    }
    ordered.push(task.clone());
    Ok(())
}
