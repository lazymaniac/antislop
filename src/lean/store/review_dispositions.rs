use super::*;
use crate::lean::tasks::TaskContract;
use std::collections::BTreeSet;

impl Store {
    pub(in crate::lean) fn check_finding_actions(
        &self,
        conversation: &str,
        revision: &PlanRevision,
        baseline: &[Task],
        candidate: &[TaskContract],
    ) -> Result<()> {
        let mut seen = BTreeSet::new();
        let inherited = self.db().prepare(
            "SELECT DISTINCT d.destination_task,d.destination_criterion FROM review_finding_dispositions d
             JOIN plan_revision_jobs j ON j.id=d.revision_request WHERE j.plan_id=?1
               AND j.status='applied' AND d.destination_task IS NOT NULL",
        )?.query_map([&revision.plan], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for (task, criterion) in inherited {
            if baseline
                .iter()
                .any(|original| original.spec["id"] == task && original.status != "approved")
            {
                crate::require(
                    candidate.iter().any(|item| {
                        item.id == task
                            && item
                                .acceptance_criteria
                                .iter()
                                .any(|item| item.id == criterion)
                    }),
                    &format!(
                        "task {task}: retain transferred proof criterion {criterion} until reviewed"
                    ),
                )?;
            }
        }
        for action in &revision.finding_actions {
            let source = baseline
                .iter()
                .find(|task| {
                    task.id == action.source_task
                        && task.input_generation == action.source_generation
                })
                .ok_or_else(|| crate::invalid("finding_actions: source task inputs changed"))?;
            crate::require(
                !action.reason.trim().is_empty()
                    && seen.insert((&action.source_task, &action.finding))
                    && !self.disposed_findings(source)?.contains(&action.finding),
                "finding_actions: give one reason per still-open finding",
            )?;
            let (review_turn, _, review) = self
                .previous_task_review_record(conversation, source, "")?
                .ok_or_else(|| crate::invalid("finding_actions: source has no completed review"))?;
            crate::require(
                review_turn == action.review_turn
                    && review.findings.iter().any(|finding| {
                        finding["id"] == action.finding
                            && finding["reproduction"].is_object()
                            && crate::lean::progress::blocks(finding)
                    }),
                "finding_actions: source review changed or no longer carries the reproduced finding",
            )?;
            if let Some(destination) = &action.destination {
                let target = candidate
                    .iter()
                    .find(|task| {
                        task.id == destination.task
                            && task.options_for.is_none()
                            && !task.quality_gate
                    })
                    .ok_or_else(|| {
                        crate::invalid(
                            "finding_actions: destination must be an authored candidate task",
                        )
                    })?;
                crate::require(
                    source.spec["id"] != target.id
                        && target
                            .acceptance_criteria
                            .iter()
                            .any(|criterion| criterion.id == destination.criterion),
                    "finding_actions: name an existing destination criterion on another task",
                )?;
                crate::require(
                    !baseline.iter().any(|task| {
                        task.spec["id"] == target.id
                            && task.status == "approved"
                            && !revision.redo.contains(&target.id)
                    }),
                    "finding_actions: destination already approved; add a new task or explicitly redo it",
                )?;
                let new = target
                    .acceptance_criteria
                    .iter()
                    .find(|item| item.id == destination.criterion)
                    .expect("validated criterion");
                crate::require(
                    revision.redo.contains(&target.id)
                        || !baseline.iter().any(|task| {
                            task.spec["id"] == target.id
                                && task.spec["acceptance_criteria"].as_array().is_some_and(
                                    |criteria| {
                                        criteria.iter().any(|old| {
                                            old["id"] == new.id && old["text"] == new.text
                                        })
                                    },
                                )
                        }),
                    "finding_actions: add or update the destination proof criterion, or explicitly redo its task",
                )?;
            }
        }
        Ok(())
    }

    pub(in crate::lean) fn transferred_criteria(&self, task: &Task) -> Result<BTreeSet<String>> {
        Ok(self
            .db()
            .prepare(
                "SELECT d.destination_criterion FROM review_finding_dispositions d
             JOIN plan_revision_jobs j ON j.id=d.revision_request
             WHERE j.plan_id=?1 AND j.status='applied' AND d.destination_task=?2",
            )?
            .query_map(params![task.plan_id, task.spec["id"].as_str()], |row| {
                row.get(0)
            })?
            .collect::<std::result::Result<_, _>>()?)
    }

    pub(in crate::lean) fn disposed_findings(&self, source: &Task) -> Result<BTreeSet<String>> {
        Ok(self.db().prepare("SELECT finding_id FROM review_finding_dispositions WHERE task_id=?1 AND generation=?2")?
            .query_map(params![source.id, source.input_generation], |row| row.get(0))?
            .collect::<std::result::Result<_, _>>()?)
    }
}

pub(in crate::lean) fn persist_dispositions(
    tx: &rusqlite::Transaction<'_>,
    conversation: &str,
    revision: &PlanRevision,
    request: &str,
) -> Result<()> {
    for action in &revision.finding_actions {
        let latest: Option<String> = tx.query_row(
            "SELECT t.id FROM turns t JOIN agent_results r ON r.turn_id=t.id
             JOIN turn_scopes s ON s.turn_id=t.id JOIN turn_inputs i ON i.turn_id=t.id
             JOIN task_inputs g ON g.task_id=s.task_id AND g.generation=i.generation
             WHERE t.conversation_id=?1 AND t.plan_id=?2 AND s.task_id=?3 AND i.generation=?4
               AND t.role='reviewer:'||s.task_id AND t.status='done' AND json_extract(r.result,'$.type')='review'
             ORDER BY t.rowid DESC LIMIT 1",
            params![conversation, revision.plan, action.source_task, action.source_generation],
            |row| row.get(0),
        ).optional()?;
        crate::require(
            latest.as_deref() == Some(&action.review_turn),
            "finding_actions: source review changed during independent review",
        )?;
        tx.execute(
            "INSERT INTO review_finding_dispositions(task_id,generation,finding_id,review_turn,revision_request,reason,destination_task,destination_criterion)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            params![action.source_task,action.source_generation,action.finding,action.review_turn,request,action.reason,
                action.destination.as_ref().map(|dest| &dest.task),action.destination.as_ref().map(|dest| &dest.criterion)],
        )?;
        tx.execute("INSERT INTO events(conversation_id,plan_id,kind,payload,created_ms) VALUES(?1,?2,'review_finding_disposition',?3,?4)",
            params![conversation,revision.plan,serde_json::to_string(action)?,now_ms()])?;
    }
    Ok(())
}
