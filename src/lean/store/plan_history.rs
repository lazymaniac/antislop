use super::*;

impl Store {
    /// Detailed history of one plan: its tasks, agents, turns, reports and verdicts.
    pub fn plan_history(&self, conversation: &str, plan_id: &str) -> Result<PlanHistory> {
        self.conversation(conversation)?;
        let tasks = self.plan(plan_id)?;
        crate::require(
            !tasks.is_empty(),
            &format!("no plan {plan_id} in conversation {conversation}"),
        )?;

        let db = self.db();
        let created_ms: i64 = db.query_row(
            "SELECT COALESCE(
                (SELECT created_ms FROM plans WHERE id=?1 AND conversation_id=?2),
                (SELECT MIN(created_ms) FROM tasks WHERE plan_id=?1 AND conversation_id=?2),
                0
            )",
            params![plan_id, conversation],
            |row| row.get(0),
        )?;

        let active_plan_id: Option<String> = db
            .query_row(
                "SELECT plan_id FROM tasks WHERE conversation_id=?1 ORDER BY created_ms DESC, rowid DESC LIMIT 1",
                [conversation],
                |row| row.get(0),
            )
            .optional()?;
        let is_active = active_plan_id.as_deref() == Some(plan_id);

        let status = if tasks.iter().all(|t| t.status == "approved") {
            "approved".to_string()
        } else if tasks
            .iter()
            .any(|t| t.status == "working" || t.status == "verifying" || t.status == "queued")
        {
            "active".to_string()
        } else if tasks.iter().all(|t| t.status == "proposed") {
            "proposed".to_string()
        } else if tasks.iter().any(|t| t.status == "failed") {
            "failed".to_string()
        } else if tasks.iter().any(|t| t.status == "cancelled") {
            "cancelled".to_string()
        } else if tasks.iter().any(|t| t.status == "needs_user") {
            "needs_user".to_string()
        } else if tasks.iter().any(|t| t.status == "revised") {
            "revised".to_string()
        } else {
            "done".to_string()
        };

        let task_ids: std::collections::HashSet<String> =
            tasks.iter().map(|t| t.id.clone()).collect();
        let mut turn_stmt = db.prepare(
            "SELECT id, plan_id, role, provider, session_id, status, error, upto_message, started_ms, finished_ms, pid, (SELECT backend FROM run_contexts r WHERE r.turn_id=turns.id)
             FROM turns WHERE conversation_id=?1 ORDER BY started_ms ASC, rowid ASC",
        )?;
        let all_turns = turn_stmt
            .query_map([conversation], |row| {
                Ok(SessionInfo {
                    turn_id: row.get(0)?,
                    plan_id: row.get(1)?,
                    role: row.get(2)?,
                    provider: row.get(3)?,
                    session_id: row.get(4)?,
                    status: row.get(5)?,
                    error: row.get(6)?,
                    upto_message: row.get::<_, i64>(7)? as u64,
                    started_ms: row.get(8)?,
                    finished_ms: row.get(9)?,
                    pid: row.get(10)?,
                    backend: row.get(11)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let turns: Vec<SessionInfo> = all_turns
            .into_iter()
            .filter(|turn| {
                if turn.plan_id.as_deref() == Some(plan_id) {
                    return true;
                }
                if let Some(tid) = crate::lean::task_role(&turn.role).map(|(id, _)| id)
                    && task_ids.contains(tid)
                {
                    return true;
                }
                false
            })
            .collect();

        let mut agents_map: std::collections::BTreeMap<String, PlanAgentInfo> =
            std::collections::BTreeMap::new();
        for turn in &turns {
            let (task_id, task_label) =
                if let Some(tid) = crate::lean::task_role(&turn.role).map(|(id, _)| id) {
                    let label = tasks
                        .iter()
                        .find(|t| t.id == tid)
                        .and_then(|t| t.spec["id"].as_str())
                        .map(|s| s.to_string());
                    (Some(tid.to_string()), label)
                } else {
                    (None, None)
                };
            let entry = agents_map
                .entry(turn.role.clone())
                .or_insert_with(|| PlanAgentInfo {
                    role: turn.role.clone(),
                    provider: turn.provider.clone(),
                    model: None,
                    task_id,
                    task_label,
                    turns_count: 0,
                    status: turn.status.clone(),
                });
            entry.turns_count += 1;
            entry.status = turn.status.clone();
            entry.provider = turn.provider.clone();
        }
        let agents: Vec<PlanAgentInfo> = agents_map.into_values().collect();

        let mut reports = Vec::new();
        let mut verdicts = Vec::new();
        for task in &tasks {
            let label = task.spec["id"].as_str().unwrap_or(&task.id).to_string();
            if let Some(report) = &task.report {
                reports.push(TaskReportInfo {
                    task_id: task.id.clone(),
                    task_label: label.clone(),
                    round: task.round,
                    report: report.clone(),
                });
            }
            if let Some(verdict) = &task.verdict {
                verdicts.push(TaskVerdictInfo {
                    task_id: task.id.clone(),
                    task_label: label,
                    round: task.round,
                    verdict: verdict.clone(),
                });
            }
        }

        let run_id: Option<String> = db
            .query_row(
                "SELECT run_id FROM plan_runs WHERE plan_id=?1",
                [plan_id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(PlanHistory {
            run_id,
            id: plan_id.to_string(),
            conversation_id: conversation.to_string(),
            created_ms,
            status,
            is_active,
            tasks,
            agents,
            turns,
            reports,
            verdicts,
        })
    }
}
