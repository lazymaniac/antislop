//! Paged, turn-bound access to contracts, scope and individual audit reports.
use super::dependencies::page_text;
use super::*;

impl Core {
    pub(super) fn read_plan_context(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        tool: &str,
        args: &Value,
    ) -> Result<Value> {
        let task_reader = task_role(role);
        require(
            task_reader.is_some() && tool == "read_task_contract"
                || matches!(
                    role,
                    ORCHESTRATOR | COMMENT_ORCHESTRATOR | "planner" | PLAN_REVIEWER | AUDITOR
                )
                || role.starts_with("code-review:"),
            "tool unavailable for this role",
        )?;
        let plan = self.store.dependency_plan(conversation, role, turn)?;
        let revision = self.store.plan_revision(&plan)?;
        let text = if tool == "read_task_contract" {
            let requested = required_text(args, "task")?;
            let target = self
                .store
                .task_in_plan_by_id(&plan, &requested)?
                .filter(|task| task.status != "superseded")
                .ok_or_else(|| {
                    Error::Invalid(format!(
                        "task {requested:?} is not in the plan bound to this turn"
                    ))
                })?;
            if let Some((id, _)) = task_reader {
                let own = self
                    .store
                    .task_by_id(id)?
                    .ok_or_else(|| Error::Invalid("task no longer exists".into()))?;
                require(
                    own.plan_id == plan
                        && (target.id == id || own.spec["options_for"] == target.spec["id"]),
                    "task contract is outside this role's scope",
                )?;
            }
            let mut spec = target.spec;
            if let Ok(mut contract) = serde_json::from_value::<TaskContract>(spec.clone()) {
                tasks::normalize_options_presentation(&mut contract);
                spec["goal"] = json!(contract.goal);
            }
            crate::lean::prompt_context::markdown(&spec)
        } else {
            format!("{}\n\nPlan document:\n{}\n\nTask index:\n{}", self.plan_scope_context(conversation, &plan)?, self.store.plan_document(conversation, &plan)?.unwrap_or_default(),
                crate::lean::prompt_context::markdown(&self.store.plan_specs(&plan)?.iter().map(|spec| json!({
                    "id":spec["id"], "summary":spec["summary"].as_str().or(spec["title"].as_str()),
                    "depends_on":spec["depends_on"],
                })).collect::<Vec<_>>()))
        };
        require(
            self.store.plan_revision(&plan)? == revision,
            "plan context revision changed while reading; reread without cursor",
        )?;
        let binding = crate::hash(serde_json::to_vec(&(
            conversation,
            role,
            turn,
            &plan,
            revision,
            tool,
            &args["task"],
            crate::hash(text.as_bytes()),
        ))?);
        if let Some(cursor) = args["cursor"].as_str() {
            require(
                cursor
                    .split_once(':')
                    .is_some_and(|(key, _)| key == binding),
                "plan context cursor is stale: plan revision or context changed; reread from the first page without cursor",
            )?;
        }
        let (text, cursor) = page_text(&text, &binding, args)?;
        Ok(
            json!({"plan":plan,"revision":revision,"text":text,"complete":cursor.is_none(),"cursor":cursor}),
        )
    }

    pub(super) fn read_audit(
        &self,
        conversation: &str,
        role: &str,
        turn: &str,
        args: &Value,
    ) -> Result<Value> {
        require(
            matches!(
                role,
                ORCHESTRATOR | COMMENT_ORCHESTRATOR | "planner" | PLAN_REVIEWER | AUDITOR
            ) || role.starts_with("code-review:"),
            "tool unavailable for this role",
        )?;
        let plan = self.store.dependency_plan(conversation, role, turn)?;
        let reference = required_text(args, "audit_ref")?;
        let seq = reference
            .strip_prefix("audit:")
            .and_then(|id| id.parse::<u64>().ok())
            .filter(|id| *id > 0)
            .ok_or_else(|| Error::Invalid("audit_ref: expected audit:<event sequence>".into()))?;
        let event = self
            .store
            .events_since(conversation, seq - 1, 1)?
            .into_iter()
            .find(|event| {
                event.seq == seq && event.kind == "audit" && event.plan_id.as_deref() == Some(&plan)
            })
            .ok_or_else(|| {
                Error::Invalid(
                    "audit_ref: unknown audit in this turn's bound plan and conversation".into(),
                )
            })?;
        // Older ordinary audits omitted the report; never substitute an aggregate plan report.
        let report = event.payload["report"].as_str();
        let text = report.unwrap_or("Legacy audit: the separate auditor report was not retained in this event. The verdict and findings remain available; no aggregate report is substituted.");
        let binding = crate::hash(serde_json::to_vec(&(conversation, role, turn, &reference))?);
        let (text, cursor) = page_text(text, &binding, args)?;
        Ok(
            json!({"audit_ref":reference,"plan":plan,"report":text,"report_available":report.is_some(),"verdict":{"approved":event.payload["approved"],"findings":event.payload["findings"],"error":event.payload["error"]},"complete":cursor.is_none(),"cursor":cursor}),
        )
    }
}
