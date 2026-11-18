use super::*;

#[path = "helpers/recording.rs"]
mod recording;
use recording::*;

#[path = "helpers/review.rs"]
mod review;

impl Core {
    pub(super) fn prepare_plan(
        &self,
        conversation: &str,
        specs: &[TaskContract],
        scope: Option<&str>,
    ) -> Result<PreparedPlan> {
        let inherited = self
            .store
            .latest_plan(conversation)?
            .first()
            .map(|task| self.store.plan_scope_id(&task.plan_id))
            .transpose()?
            .flatten();
        let scope_id = match (scope, inherited) {
            (Some(summary), Some(id))
                if self.store.scope(conversation, &id)?.summary == summary
                    && self.store.scope_documents_current(conversation, &id)? =>
            {
                id
            }
            (Some(summary), _) => self.store.create_scope(conversation, summary)?.id,
            (None, Some(id))
                if self.store.scope(conversation, &id)?.summary
                    == self.store.workflow(conversation)?.summary
                    && self.store.scope_documents_current(conversation, &id)? =>
            {
                id
            }
            (None, Some(_)) => {
                self.store
                    .create_scope(conversation, &self.store.workflow(conversation)?.summary)?
                    .id
            }
            (None, None) => {
                self.store
                    .create_scope(conversation, &self.store.workflow(conversation)?.summary)?
                    .id
            }
        };
        let (tasks, id) = self.persist_proposed_tasks(conversation, specs)?;
        self.store.bind_plan_scope(conversation, &id, &scope_id)?;
        Ok(PreparedPlan {
            id,
            tasks,
            answered: self.store.last_orchestration_answered(conversation)?,
        })
    }

    fn persist_proposed_tasks(
        &self,
        conversation: &str,
        specs: &[TaskContract],
    ) -> Result<(Vec<Task>, String)> {
        let values = specs
            .iter()
            .map(serde_json::to_value)
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let tasks = self.store.add_plan(conversation, &values)?;
        let plan_id = tasks[0].plan_id.clone();
        Ok((tasks, plan_id))
    }

    pub(super) fn publish_plan(
        self: &Arc<Self>,
        conversation: &str,
        text: &str,
        addressed: Option<&[u64]>,
        plan: &PreparedPlan,
        specs: &[TaskContract],
        review: &std::result::Result<Verdict, String>,
    ) -> Result<()> {
        let checked = review.as_ref().is_ok_and(|verdict| verdict.approved);
        self.resolve_plan_comments(conversation, addressed, plan, checked)?;
        let waiting = (!checked).then_some("the plan reviewer did not approve this plan");
        let revision_token = self.plan_revision_token(conversation, &plan.id)?;
        let scope_id = self
            .store
            .plan_scope_id(&plan.id)?
            .ok_or_else(|| Error::Invalid("plan has no scope".into()))?;
        let scope = self.store.scope(conversation, &scope_id)?;
        let previous = self
            .store
            .accepted_plan(conversation)?
            .map(|id| self.store.plan_scope_id(&id))
            .transpose()?
            .flatten()
            .map(|id| self.store.scope(conversation, &id))
            .transpose()?;
        let document = match previous.as_ref().filter(|old| old.id != scope.id) {
            Some(old) => format!(
                "# Scope change\n\n## Previously accepted\n{}\nOriginal documents: {}\n\n## Proposed scope\n{}\nOriginal documents: {}\n\n{}",
                old.summary,
                old.source_ids.len(),
                scope.summary,
                scope.source_ids.len(),
                text
            ),
            None => format!("# Scope\n\n{}\n\n{}", scope.summary, text),
        };
        let mut payload =
            plan_event_payload(&document, plan, specs, review, waiting, revision_token)?;
        payload["scope"] = json!(scope);
        payload["previous_scope"] = json!(previous);
        self.store.append_event(
            conversation,
            None,
            Some(&plan.id),
            "plan_proposed",
            &payload,
        )?;
        Ok(())
    }

    fn resolve_plan_comments(
        &self,
        conversation: &str,
        addressed: Option<&[u64]>,
        plan: &PreparedPlan,
        checked: bool,
    ) -> Result<()> {
        if !checked {
            return Ok(());
        }
        let pending = self.store.pending_plan_comment_ids(conversation)?;
        self.store.resolve_listed_plan_comments(
            conversation,
            plan.answered,
            addressed.or(pending.as_deref()),
        )?;
        Ok(())
    }
}
pub(super) struct PreparedPlan {
    pub(super) id: String,
    tasks: Vec<Task>,
    answered: u64,
}

pub(super) fn record_addressed_comments(
    store: &Store,
    conversation: &str,
    addressed: Option<&[u64]>,
) -> Result<()> {
    if let Some(ids) = addressed {
        store.append_event(
            conversation,
            None,
            None,
            "plan_comments_addressed",
            &json!({"ids": ids, "upto_message": store.last_orchestration_answered(conversation)?}),
        )?;
    }
    Ok(())
}
